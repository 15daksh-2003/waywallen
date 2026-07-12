module;
#include "waywallen/query/playlist_query.moc.h"
#include <qtprotobuftypes.h>
#undef assert
#include <rstd/macro.hpp>

module waywallen;
import :query.playlist;
import :app;

using namespace Qt::Literals::StringLiterals;

namespace proto = waywallen::control::v1;
using namespace qextra::prelude;

namespace waywallen
{

PlaylistListQuery::PlaylistListQuery(QObject* parent): Query(parent) {}
auto PlaylistListQuery::playlists() const -> const QVariantList& { return m_playlists; }

void PlaylistListQuery::reload() {
    setStatus(Status::Querying);
    auto backend = App::instance()->backend();
    auto req     = proto::Request {};
    req.setPlaylistList(proto::PlaylistListRequest {});
    auto self = QWatcher { this };
    spawn([self, backend, req = std::move(req)]() mutable -> task<void> {
        auto result = co_await backend->send(std::move(req));
        if (! co_await QAsyncResult::qexecutor()) co_return;
        if (! self) co_return;
        self->inspect_set(result, [self](const proto::Response& rsp) {
            QVariantList out;
            for (const auto& p : rsp.playlistList().playlists()) {
                QVariantMap m;
                m[u"id"_s]           = static_cast<qint64>(p.id_proto());
                m[u"name"_s]         = p.name();
                m[u"mode"_s]         = static_cast<int>(p.mode());
                m[u"intervalSecs"_s] = p.intervalSecs();
                m[u"itemCount"_s]    = p.itemCount();
                QStringList eids;
                for (const auto& e : p.entryIds()) eids.append(e);
                m[u"entryIds"_s] = eids;
                out.append(m);
            }
            self->m_playlists = std::move(out);
            Q_EMIT self->playlistsChanged();
        });
        co_return;
    });
}

PlaylistMutationQuery::PlaylistMutationQuery(QObject* parent): Query(parent) {}

static QStringList toStr(const QVariantList& v) {
    QStringList out;
    for (const auto& x : v) out.append(x.toString());
    return out;
}

static QtProtobuf::uint64List toU64(const QVariantList& v) {
    QtProtobuf::uint64List out;
    out.reserve(v.size());
    for (const auto& x : v) out.append(x.toULongLong());
    return out;
}

void PlaylistMutationQuery::send(proto::Request req, bool captureCreate) {
    setStatus(Status::Querying);
    auto backend = App::instance()->backend();
    auto self    = QWatcher { this };
    spawn([self, backend, req = std::move(req), captureCreate]() mutable -> task<void> {
        auto result = co_await backend->send(std::move(req));
        if (! co_await QAsyncResult::qexecutor()) co_return;
        if (! self) co_return;
        self->inspect_set(result, [self, captureCreate](const proto::Response& rsp) {
            if (captureCreate) {
                self->m_createdId = static_cast<qint64>(rsp.playlistCreate().id_proto());
                Q_EMIT self->createdIdChanged();
            }
        });
        Q_EMIT self->done();
        co_return;
    });
}

void PlaylistMutationQuery::create(const QString& name, int mode, int intervalSecs,
                                   const QVariantList& itemIds) {
    proto::PlaylistCreateRequest r;
    r.setName(name);
    r.setMode(static_cast<proto::PlaylistMode>(mode));
    r.setIntervalSecs(static_cast<QtProtobuf::uint32>(intervalSecs));
    r.setEntryIds(toStr(itemIds));
    proto::Request req;
    req.setPlaylistCreate(std::move(r));
    send(std::move(req), true);
}

void PlaylistMutationQuery::remove(qint64 id) {
    proto::PlaylistDeleteRequest r;
    r.setId_proto(id);
    proto::Request req;
    req.setPlaylistDelete(std::move(r));
    send(std::move(req), false);
}

void PlaylistMutationQuery::rename(qint64 id, const QString& name) {
    proto::PlaylistRenameRequest r;
    r.setId_proto(id);
    r.setName(name);
    proto::Request req;
    req.setPlaylistRename(std::move(r));
    send(std::move(req), false);
}

void PlaylistMutationQuery::setItems(qint64 id, const QVariantList& itemIds) {
    proto::PlaylistSetItemsRequest r;
    r.setId_proto(id);
    r.setEntryIds(toStr(itemIds));
    proto::Request req;
    req.setPlaylistSetItems(std::move(r));
    send(std::move(req), false);
}

void PlaylistMutationQuery::setMode(qint64 id, int mode) {
    proto::PlaylistSetModeRequest r;
    r.setId_proto(id);
    r.setMode(static_cast<proto::PlaylistMode>(mode));
    proto::Request req;
    req.setPlaylistSetMode(std::move(r));
    send(std::move(req), false);
}

void PlaylistMutationQuery::setInterval(qint64 id, int intervalSecs) {
    proto::PlaylistSetIntervalRequest r;
    r.setId_proto(id);
    r.setIntervalSecs(static_cast<QtProtobuf::uint32>(intervalSecs));
    proto::Request req;
    req.setPlaylistSetInterval(std::move(r));
    send(std::move(req), false);
}

void PlaylistMutationQuery::activate(qint64 id, const QVariantList& displayIds, bool autoAttach) {
    proto::PlaylistActivateRequest r;
    r.setId_proto(id);
    r.setDisplayIds(toU64(displayIds));
    r.setAutoAttach(autoAttach);
    proto::Request req;
    req.setPlaylistActivate(std::move(r));
    send(std::move(req), false);
}

void PlaylistMutationQuery::deactivate(const QVariantList& displayIds, qint64 clearAutoAttach) {
    proto::PlaylistDeactivateRequest r;
    r.setDisplayIds(toU64(displayIds));
    r.setClearAutoAttach(clearAutoAttach);
    proto::Request req;
    req.setPlaylistDeactivate(std::move(r));
    send(std::move(req), false);
}

void PlaylistMutationQuery::jumpTo(qint64 id, const QString& entryId) {
    proto::PlaylistJumpToRequest r;
    r.setId_proto(id);
    r.setEntryId(entryId);
    proto::Request req;
    req.setPlaylistJumpTo(std::move(r));
    send(std::move(req), false);
}

} // namespace waywallen

#include "waywallen/query/playlist_query.moc.cpp"
