module;
#include <algorithm>
#include <memory>
#include <rstd/macro.hpp>
#include "waywallen/backend.moc.h"

module waywallen;
import :app;
import ncrequest;
import qextra;

using namespace Qt::Literals::StringLiterals;
using namespace qextra::prelude;
using namespace rstd::prelude;

namespace proto = waywallen::control::v1;

namespace waywallen
{

namespace detail
{

using ResponseHandle = rstd::async::CompletionHandle<proto::Response, QString>;

auto as_byte_array_view(slice<rstd::byte> bytes) -> QByteArrayView {
    return { reinterpret_cast<const char*>(bytes.as_raw_ptr()),
             static_cast<qsizetype>(bytes.len()) };
}

class BackendTransport : public QObject {
public:
    explicit BackendTransport(Backend* backend): m_backend(backend) {}

    void initialize() {
        m_serializer = std::make_unique<QProtobufSerializer>();
        m_client     = std::make_unique<ncrequest::WebSocketClient>();

        m_client->set_on_error_callback([this](ref<str> error) {
            auto message = QString::fromUtf8(reinterpret_cast<const char*>(error.data()),
                                             static_cast<qsizetype>(error.size()));
            qWarning("ws error: %s", qPrintable(message));
            fail_pending(message);
            Q_EMIT m_backend->error(message);
        });
        m_client->set_on_connected_callback([this] {
            Q_EMIT m_backend->connected();
        });
        m_client->set_on_disconnected_callback([this] {
            fail_pending(QStringLiteral("backend disconnected"));
            Q_EMIT m_backend->transportDisconnected();
        });
        m_client->set_on_message_callback(
            [this, cache = QByteArray {}](slice<rstd::byte> bytes, bool last) mutable {
                auto chunk = as_byte_array_view(bytes);
                if (! last) {
                    cache.append(chunk);
                    return;
                }

                proto::ServerFrame frame;
                if (cache.isEmpty()) {
                    frame.deserialize(m_serializer.get(), chunk);
                } else {
                    cache.append(chunk);
                    frame.deserialize(m_serializer.get(), cache);
                    cache.clear();
                }

                if (frame.hasResponse()) {
                    auto response = frame.response();
                    if (auto it = m_handlers.find(response.requestId()); it != m_handlers.end()) {
                        (void)it->second.complete(rstd::move(response));
                        m_handlers.erase(it);
                    }
                } else if (frame.hasEvent()) {
                    Q_EMIT m_backend->eventReceived(frame.event());
                } else {
                    qWarning("ws: ServerFrame with no kind set");
                }
            });
    }

    void connect_to(std::string url) {
        if (m_client) (void)m_client->connect(url);
    }

    void disconnect_from_server() {
        if (m_client) m_client->disconnect();
        fail_pending(QStringLiteral("backend disconnected"));
    }

    void send(proto::Request request, ResponseHandle response) {
        if (! m_client || ! m_client->is_connected()) {
            (void)response.fail(QStringLiteral("backend disconnected"));
            return;
        }

        auto id = request.requestId();
        m_handlers.insert_or_assign(id, rstd::move(response));
        send_untracked(rstd::move(request));
    }

    void send_untracked(proto::Request request) {
        if (! m_client || ! m_client->is_connected()) return;
        auto bytes = request.serialize(m_serializer.get());
        m_client->send(slice<rstd::byte>::from_raw_parts(
            reinterpret_cast<const rstd::byte*>(bytes.constData()),
            static_cast<usize>(bytes.size())));
    }

    void cancel(quint64 request_id) { m_handlers.erase(request_id); }

    void shutdown() {
        disconnect_from_server();
        m_client.reset();
        m_serializer.reset();
    }

private:
    void fail_pending(const QString& error) {
        for (auto& [id, handler] : m_handlers) {
            (void)id;
            (void)handler.fail(error);
        }
        m_handlers.clear();
    }

    Backend*                                    m_backend;
    std::unique_ptr<ncrequest::WebSocketClient> m_client;
    std::unique_ptr<QProtobufSerializer>        m_serializer;
    std::map<quint64, ResponseHandle>           m_handlers;
};

class RequestGuard {
public:
    RequestGuard(BackendTransport* transport, quint64 request_id)
        : m_transport(transport), m_request_id(request_id) {}

    RequestGuard(const RequestGuard&)            = delete;
    RequestGuard& operator=(const RequestGuard&) = delete;
    RequestGuard(RequestGuard&&)                 = delete;
    RequestGuard& operator=(RequestGuard&&)      = delete;

    ~RequestGuard() {
        (void)QMetaObject::invokeMethod(
            m_transport,
            [transport = m_transport, request_id = m_request_id] {
                transport->cancel(request_id);
            },
            Qt::QueuedConnection);
    }

private:
    BackendTransport* m_transport;
    quint64           m_request_id;
};

} // namespace detail

Backend::Backend(quint16 port)
    : m_thread(Box<QThread>::make()),
      m_transport(new detail::BackendTransport(this)),
      m_serial(1),
      m_port(port),
      m_reconnect_timer(nullptr),
      m_reconnect_delay(1000),
      m_disconnect_requested(false),
      m_connected(false) {
    m_transport->moveToThread(m_thread.get());
    connect(m_thread.get(), &QThread::finished, m_transport, &QObject::deleteLater);
    m_thread->start();
    (void)QMetaObject::invokeMethod(
        m_transport,
        [transport = m_transport] {
            transport->initialize();
        },
        Qt::QueuedConnection);

    connect(this, &Backend::connected, this, &Backend::on_connected);
    connect(this, &Backend::error, this, &Backend::on_error);
    connect(this, &Backend::transportDisconnected, this, &Backend::on_disconnected);
}

Backend::~Backend() {
    (void)QMetaObject::invokeMethod(
        m_transport,
        [transport = m_transport] {
            transport->shutdown();
        },
        Qt::BlockingQueuedConnection);
    m_thread->quit();
    m_thread->wait();
}

void Backend::connectTo() {
    if (m_port == 0) return;

    m_disconnect_requested = false;
    m_reconnect_delay      = 1000;
    auto url               = std::format("ws://127.0.0.1:{}", m_port);
    (void)QMetaObject::invokeMethod(
        m_transport,
        [transport = m_transport, url = rstd::move(url)]() mutable {
            transport->connect_to(rstd::move(url));
        },
        Qt::QueuedConnection);
}

void Backend::setPort(quint16 port) {
    if (m_port == port) return;
    m_port = port;
    if (m_reconnect_timer) m_reconnect_timer->stop();
}

void Backend::disconnect() {
    if (m_reconnect_timer) m_reconnect_timer->stop();
    m_disconnect_requested = true;
    (void)QMetaObject::invokeMethod(
        m_transport,
        [transport = m_transport] {
            transport->disconnect_from_server();
        },
        Qt::QueuedConnection);
}

void Backend::on_connected() {
    m_connected       = true;
    m_reconnect_delay = 1000;
    if (m_reconnect_timer) m_reconnect_timer->stop();

    auto request = proto::Request {};
    request.setRequestId(serial());
    request.setHealth(proto::HealthRequest {});
    (void)QMetaObject::invokeMethod(
        m_transport,
        [transport = m_transport, request = rstd::move(request)]() mutable {
            transport->send_untracked(rstd::move(request));
        },
        Qt::QueuedConnection);
}

void Backend::on_error(QString message) {
    qWarning("backend error: %s", qPrintable(message));
    if (m_connected) {
        m_connected = false;
        Q_EMIT disconnected();
    }

    if (! m_reconnect_timer) {
        m_reconnect_timer = new QTimer(this);
        m_reconnect_timer->setSingleShot(true);
        connect(m_reconnect_timer, &QTimer::timeout, this, &Backend::on_retry);
    }
    m_reconnect_timer->start(m_reconnect_delay);
    m_reconnect_delay = std::min(m_reconnect_delay * 2, kMaxReconnectDelay);
}

void Backend::on_disconnected() {
    if (m_connected) {
        m_connected = false;
        Q_EMIT disconnected();
    }
    if (m_disconnect_requested) {
        m_disconnect_requested = false;
        return;
    }
    if (m_reconnect_timer && m_reconnect_timer->isActive()) return;

    if (! m_reconnect_timer) {
        m_reconnect_timer = new QTimer(this);
        m_reconnect_timer->setSingleShot(true);
        connect(m_reconnect_timer, &QTimer::timeout, this, &Backend::on_retry);
    }
    m_reconnect_timer->start(m_reconnect_delay);
    m_reconnect_delay = std::min(m_reconnect_delay * 2, kMaxReconnectDelay);
}

void Backend::on_retry() { connectTo(); }

auto Backend::send(proto::Request&& request) -> task<Result<proto::Response, QString>> {
    auto request_id = serial();
    request.setRequestId(request_id);

    auto completion_result = rstd::async::Completion<proto::Response, QString>::make();
    if (completion_result.is_err()) {
        co_return Err(QStringLiteral("failed to allocate request completion state"));
    }

    auto completion_pair = rstd::move(completion_result).unwrap_unchecked();
    auto completion      = rstd::move(completion_pair.get<0>());
    auto response        = rstd::move(completion_pair.get<1>());

    auto posted = QMetaObject::invokeMethod(
        m_transport,
        [transport = m_transport,
         request   = rstd::move(request),
         response  = rstd::move(response)]() mutable {
            transport->send(rstd::move(request), rstd::move(response));
        },
        Qt::QueuedConnection);
    if (! posted) {
        co_return Err(QStringLiteral("backend transport is unavailable"));
    }

    auto guard  = detail::RequestGuard { m_transport, request_id };
    auto result = co_await rstd::move(completion);
    if (result.is_err()) {
        auto error = rstd::move(result).unwrap_err_unchecked();
        if (error.is_failed()) {
            co_return Err(rstd::move(error).unwrap_failed());
        }
        co_return Err(QStringLiteral("request canceled"));
    }

    auto value = rstd::move(result).unwrap_unchecked();
    if (value.status() != proto::Status::OK) {
        auto error = value.message().isEmpty()
                         ? QStringLiteral("status %1").arg(static_cast<int>(value.status()))
                         : value.message();
        co_return Err(rstd::move(error));
    }
    co_return Ok(rstd::move(value));
}

auto Backend::serial() -> quint64 {
    quint64 current = m_serial.load();
    for (;;) {
        auto next = current + 1;
        if (m_serial.compare_exchange_strong(current, next)) break;
    }
    return current;
}

} // namespace waywallen

#include "waywallen/backend.moc"
