module;
#include "QExtra/macro_qt.hpp"
#include <QtCore/QStringList>
#include <QtCore/QVariant>
#include <QtCore/QVariantList>
#include <QtCore/qtypes.h>

#ifdef Q_MOC_RUN
#    include "waywallen/model/wallpaper_select_storage.moc"
#endif

export module waywallen:model.wallpaper_select_storage;
export import :msg.backend_msg;
export import qextra;

namespace waywallen::model
{

export class WallpaperSelectStorage : public SelectStorage {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QVariant playlistEditTarget READ playlistEditTarget WRITE setPlaylistEditTarget
                   NOTIFY playlistEditTargetChanged FINAL)
    Q_PROPERTY(qint64 playlistEditTargetId READ playlistEditTargetId NOTIFY
                   playlistEditTargetChanged FINAL)
    Q_PROPERTY(qint32 removableSelectedCount READ removableSelectedCount NOTIFY
                   selectedRemovableCountChanged FINAL)

public:
    WallpaperSelectStorage(QObject* parent = nullptr);
    ~WallpaperSelectStorage() override;

    auto playlistEditTarget() const -> const QVariant&;
    void setPlaylistEditTarget(const QVariant& playlist);
    auto playlistEditTargetId() const -> qint64;

    Q_INVOKABLE bool         hasPlaylistEditTarget() const;
    Q_INVOKABLE bool         isEditingPlaylist(const QVariant& playlist) const;
    Q_INVOKABLE void         editPlaylistSelection(const QVariant& playlist);
    Q_INVOKABLE QVariantList selectedWallpaperIds() const;
    Q_INVOKABLE QStringList  removableSelectedWallpaperIds() const;
    Q_INVOKABLE void         clear() override;

    Q_SIGNAL void playlistEditTargetChanged();
    Q_SIGNAL void selectedRemovableCountChanged();

    auto removableSelectedCount() const -> qint32;

protected:
    auto keepActiveWithoutSelection() const -> bool override;

private:
    static auto playlistId(const QVariant& playlist) -> qint64;
    static auto playlistEntryIds(const QVariant& playlist) -> QStringList;

    QVariant m_playlist_edit_target;
    qint64   m_playlist_edit_target_id = 0;
};

} // namespace waywallen::model
