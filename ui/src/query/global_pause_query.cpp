module;
#include "waywallen/query/global_pause_query.moc.h"

module waywallen;
import :query.global_pause;
import :app;

using namespace qextra::prelude;

namespace proto = waywallen::control::v1;

namespace waywallen
{

GlobalPauseToggleQuery::GlobalPauseToggleQuery(QObject* parent): Query(parent) {}

bool GlobalPauseToggleQuery::paused() const { return m_paused; }

void GlobalPauseToggleQuery::reload() {
    setStatus(Status::Querying);
    auto backend = App::instance()->backend();

    auto req = proto::Request {};
    req.setGlobalPauseToggle(proto::GlobalPauseToggleRequest {});

    auto self = QWatcher { this };
    spawn([self, backend, req = std::move(req)]() mutable -> task<void> {
        auto result = co_await backend->send(std::move(req));
        if (! co_await QAsyncResult::qexecutor()) co_return;
        if (! self) co_return;

        self->inspect_set(result, [self](const proto::Response& rsp) {
            const bool paused = rsp.globalPauseToggle().paused();
            if (self->m_paused != paused) {
                self->m_paused = paused;
                Q_EMIT self->pausedChanged();
            }
            Q_EMIT self->toggled(paused);
        });
        co_return;
    });
}

GlobalPauseSetQuery::GlobalPauseSetQuery(QObject* parent): Query(parent) {}

bool GlobalPauseSetQuery::paused() const { return m_paused; }

void GlobalPauseSetQuery::setPaused(bool paused) {
    if (m_paused == paused) return;
    m_paused = paused;
    Q_EMIT pausedChanged();
}

void GlobalPauseSetQuery::reload() {
    setStatus(Status::Querying);
    auto backend = App::instance()->backend();

    auto inner = proto::GlobalPauseSetRequest {};
    inner.setPaused(m_paused);
    auto req = proto::Request {};
    req.setGlobalPauseSet(std::move(inner));

    auto self = QWatcher { this };
    spawn([self, backend, req = std::move(req)]() mutable -> task<void> {
        auto result = co_await backend->send(std::move(req));
        if (! co_await QAsyncResult::qexecutor()) co_return;
        if (! self) co_return;

        self->inspect_set(result, [self](const proto::Response& rsp) {
            self->setPaused(rsp.globalPauseSet().paused());
        });
        co_return;
    });
}

GlobalMuteSetQuery::GlobalMuteSetQuery(QObject* parent): Query(parent) {}

bool GlobalMuteSetQuery::muted() const { return m_muted; }

void GlobalMuteSetQuery::setMuted(bool muted) {
    if (m_muted == muted) return;
    m_muted = muted;
    Q_EMIT mutedChanged();
}

void GlobalMuteSetQuery::reload() {
    setStatus(Status::Querying);
    auto backend = App::instance()->backend();

    auto inner = proto::GlobalMuteSetRequest {};
    inner.setMuted(m_muted);
    auto req = proto::Request {};
    req.setGlobalMuteSet(std::move(inner));

    auto self = QWatcher { this };
    spawn([self, backend, req = std::move(req)]() mutable -> task<void> {
        auto result = co_await backend->send(std::move(req));
        if (! co_await QAsyncResult::qexecutor()) co_return;
        if (! self) co_return;

        self->inspect_set(result, [self](const proto::Response& rsp) {
            self->setMuted(rsp.globalMuteSet().muted());
        });
        co_return;
    });
}

} // namespace waywallen

#include "waywallen/query/global_pause_query.moc.cpp"
