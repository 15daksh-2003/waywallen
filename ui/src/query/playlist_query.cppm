module;
#include "QExtra/macro_qt.hpp"

#ifdef Q_MOC_RUN
#    include "waywallen/query/playlist_query.moc"
#endif

export module waywallen:query.playlist;
export import :query.query;

namespace waywallen
{

export class PlaylistListQuery : public Query,
                                 public QueryExtra<control::v1::Response, PlaylistListQuery> {
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(QVariantList playlists READ playlists NOTIFY playlistsChanged FINAL)
public:
    PlaylistListQuery(QObject* parent = nullptr);
    auto          playlists() const -> const QVariantList&;
    void          reload() override;
    Q_SIGNAL void playlistsChanged();

private:
    QVariantList m_playlists;
};

export class PlaylistMutationQuery
    : public Query,
      public QueryExtra<control::v1::Response, PlaylistMutationQuery> {
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(qint64 createdId READ createdId NOTIFY createdIdChanged FINAL)
public:
    PlaylistMutationQuery(QObject* parent = nullptr);
    qint64 createdId() const { return m_createdId; }

    Q_INVOKABLE void create(const QString& name, int mode, int intervalSecs,
                            const QVariantList& itemIds);
    Q_INVOKABLE void remove(qint64 id);
    Q_INVOKABLE void rename(qint64 id, const QString& name);
    Q_INVOKABLE void setItems(qint64 id, const QVariantList& itemIds);
    Q_INVOKABLE void setMode(qint64 id, int mode);
    Q_INVOKABLE void setInterval(qint64 id, int intervalSecs);
    Q_INVOKABLE void activate(qint64 id, const QVariantList& targets, bool autoAttach);
    Q_INVOKABLE void deactivate(const QVariantList& targets, qint64 clearAutoAttach);
    Q_INVOKABLE void jumpTo(qint64 id, const QString& entryId);

    void reload() override {}

    Q_SIGNAL void createdIdChanged();
    Q_SIGNAL void done();

private:
    void   send(proto::Request req, bool captureCreate);
    qint64 m_createdId = 0;
};

} // namespace waywallen
