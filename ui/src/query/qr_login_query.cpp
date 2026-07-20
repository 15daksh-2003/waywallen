module;
#include "waywallen/query/qr_login_query.moc.h"

module waywallen;
import :query.qr_login;
import :app;

using namespace qextra::prelude;

namespace proto = waywallen::control::v1;

namespace waywallen
{

QrLoginCancelQuery::QrLoginCancelQuery(QObject* parent): Query(parent) {}

auto QrLoginCancelQuery::sessionId() const -> const QString& { return m_session_id; }

void QrLoginCancelQuery::setSessionId(const QString& value) {
    if (m_session_id == value) return;
    m_session_id = value;
    Q_EMIT sessionIdChanged();
}

void QrLoginCancelQuery::reload() {
    if (m_session_id.isEmpty()) return;
    setStatus(Status::Querying);
    auto backend = App::instance()->backend();

    auto req   = proto::Request {};
    auto inner = proto::QrLoginCancelRequest {};
    inner.setSessionId(m_session_id);
    req.setQrLoginCancel(std::move(inner));

    auto self = QWatcher { this };
    spawn([self, backend, req = std::move(req)]() mutable -> task<void> {
        auto result = co_await backend->send(std::move(req));
        if (! co_await QAsyncResult::qexecutor()) co_return;
        if (! self) co_return;
        self->inspect_set(result, [](const proto::Response&) {
        });
        co_return;
    });
}

} // namespace waywallen

#include "waywallen/query/qr_login_query.moc.cpp"
