module;
#include "QExtra/macro_qt.hpp"

#ifdef Q_MOC_RUN
#    include "waywallen/objmodel/display.moc"
#endif

export module waywallen:display;
export import :proto;
export import :backend;
import rstd;
import rstd.cppstd;
import qextra;

using rstd::boxed::Box;

namespace proto = waywallen::control::v1;

export namespace waywallen
{

/// One display, mirroring `proto::DisplayInfo` as a QObject so QML can
/// bind directly to its fields. Identity is `id()`; mutate via
/// `updateFrom(info)` which diff-emits per changed property.
class Display : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_UNCREATABLE("Display instances are owned by DisplayManager")

    Q_PROPERTY(quint64 id READ id CONSTANT FINAL)
    Q_PROPERTY(QString name READ name NOTIFY nameChanged FINAL)
    Q_PROPERTY(QString alias READ alias NOTIFY aliasChanged FINAL)
    Q_PROPERTY(QString displayLabel READ displayLabel NOTIFY displayLabelChanged FINAL)
    Q_PROPERTY(QString instanceId READ instanceId NOTIFY identityChanged FINAL)
    Q_PROPERTY(QString settingsKey READ settingsKey NOTIFY identityChanged FINAL)
    Q_PROPERTY(quint32 width READ width NOTIFY sizeChanged FINAL)
    Q_PROPERTY(quint32 height READ height NOTIFY sizeChanged FINAL)
    Q_PROPERTY(quint32 refreshMhz READ refreshMhz NOTIFY refreshMhzChanged FINAL)
    Q_PROPERTY(QVariantList links READ links NOTIFY linksChanged FINAL)
    /// Resolved layout currently in use for this display
    /// (per-display override on top of global defaults). Map keys:
    /// `fillmode` (int), `locationX` / `locationY` (0..100).
    Q_PROPERTY(QVariantMap effectiveLayout READ effectiveLayout NOTIFY layoutChanged FINAL)
    Q_PROPERTY(QVariantMap displayLayout READ displayLayout NOTIFY layoutChanged FINAL)
    Q_PROPERTY(bool layoutOverriddenByWallpaper READ layoutOverriddenByWallpaper NOTIFY
                   layoutChanged FINAL)
    /// Sparse per-display override. Same key set as effectiveLayout
    /// plus `fillmodeSet` / `locationSet` booleans
    /// indicating whether each field is explicitly overridden vs. inherited.
    Q_PROPERTY(QVariantMap layoutOverride READ layoutOverride NOTIFY layoutChanged FINAL)
    // DRM render-node id of the GPU this display's consumer is on.
    // Set once at register_display time; never changes for a live display.
    Q_PROPERTY(quint32 drmRenderMajor READ drmRenderMajor CONSTANT FINAL)
    Q_PROPERTY(quint32 drmRenderMinor READ drmRenderMinor CONSTANT FINAL)
    Q_PROPERTY(qint64 activePlaylistId READ activePlaylistId NOTIFY playlistStatusChanged FINAL)
    Q_PROPERTY(QVariantMap playlistStatus READ playlistStatus NOTIFY playlistStatusChanged FINAL)
    Q_PROPERTY(
        QVariantList runtimeConditions READ runtimeConditions NOTIFY runtimeConditionsChanged FINAL)
    Q_PROPERTY(QString canvasId READ canvasId NOTIFY canvasChanged FINAL)
    Q_PROPERTY(QVariantMap canvasRect READ canvasRect NOTIFY canvasChanged FINAL)
    Q_PROPERTY(quint32 canvasOverlapCount READ canvasOverlapCount NOTIFY canvasChanged FINAL)
    Q_PROPERTY(bool selectableTarget READ selectableTarget NOTIFY canvasChanged FINAL)

public:
    explicit Display(const proto::DisplayInfo& info, QObject* parent = nullptr);

    auto id() const -> quint64 { return m_id; }
    auto name() const -> const QString& { return m_name; }
    auto alias() const -> const QString& { return m_alias; }
    auto displayLabel() const -> QString {
        const QString base = m_alias.isEmpty() ? m_name : m_alias;
        if (base.isEmpty()) return QString("Display #%1").arg(m_id);
        return QString("%1 (#%2)").arg(base).arg(m_id);
    }
    auto instanceId() const -> const QString& { return m_instance_id; }
    auto settingsKey() const -> const QString& { return m_settings_key; }
    auto width() const -> quint32 { return m_width; }
    auto height() const -> quint32 { return m_height; }
    auto refreshMhz() const -> quint32 { return m_refresh_mhz; }
    auto links() const -> const QVariantList& { return m_links; }
    auto effectiveLayout() const -> const QVariantMap& { return m_effective_layout; }
    auto displayLayout() const -> const QVariantMap& { return m_display_layout; }
    auto layoutOverriddenByWallpaper() const -> bool { return m_layout_overridden_by_wallpaper; }
    auto layoutOverride() const -> const QVariantMap& { return m_layout_override; }
    auto drmRenderMajor() const -> quint32 { return m_drm_render_major; }
    auto drmRenderMinor() const -> quint32 { return m_drm_render_minor; }
    auto activePlaylistId() const -> qint64 { return m_active_playlist_id; }
    auto playlistStatus() const -> const QVariantMap& { return m_playlist_status; }
    auto runtimeConditions() const -> const QVariantList& { return m_runtime_conditions; }
    auto canvasId() const -> const QString& { return m_canvas_id; }
    auto canvasRect() const -> const QVariantMap& { return m_canvas_rect; }
    auto canvasOverlapCount() const -> quint32 { return m_canvas_overlap_count; }
    auto selectableTarget() const -> bool { return m_selectable_target; }

    /// Diff-update from a freshly-received `DisplayInfo`. Only emits
    /// the signals for properties that actually changed.
    void updateFrom(const proto::DisplayInfo& info);
    void updatePlaylistStatus(const proto::PlaylistDisplayStatus* status);

    Q_SIGNAL void nameChanged();
    Q_SIGNAL void aliasChanged();
    Q_SIGNAL void displayLabelChanged();
    Q_SIGNAL void identityChanged();
    Q_SIGNAL void sizeChanged();
    Q_SIGNAL void refreshMhzChanged();
    Q_SIGNAL void linksChanged();
    Q_SIGNAL void layoutChanged();
    Q_SIGNAL void playlistStatusChanged();
    Q_SIGNAL void runtimeConditionsChanged();
    Q_SIGNAL void canvasChanged();

private:
    static auto linksFromPb(const proto::DisplayInfo& info) -> QVariantList;
    static auto effectiveLayoutFromPb(const proto::DisplayInfo& info) -> QVariantMap;
    static auto displayLayoutFromPb(const proto::DisplayInfo& info) -> QVariantMap;
    static auto layoutOverriddenByWallpaperFromPb(const proto::DisplayInfo& info) -> bool;
    static auto layoutOverrideFromPb(const proto::DisplayInfo& info) -> QVariantMap;
    static auto playlistStatusFromPb(const proto::PlaylistDisplayStatus* status) -> QVariantMap;
    static auto canvasRectFromPb(const proto::CanvasRect& rect) -> QVariantMap;

    quint64      m_id;
    QString      m_name;
    QString      m_alias;
    QString      m_instance_id;
    QString      m_settings_key;
    quint32      m_width;
    quint32      m_height;
    quint32      m_refresh_mhz;
    QVariantList m_links;
    QVariantMap  m_effective_layout;
    QVariantMap  m_display_layout;
    bool         m_layout_overridden_by_wallpaper { false };
    QVariantMap  m_layout_override;
    quint32      m_drm_render_major;
    quint32      m_drm_render_minor;
    qint64       m_active_playlist_id { 0 };
    QVariantMap  m_playlist_status;
    QVariantList m_runtime_conditions;
    QString      m_canvas_id;
    QVariantMap  m_canvas_rect;
    quint32      m_canvas_overlap_count { 0 };
    bool         m_selectable_target { true };
};

class Canvas : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_UNCREATABLE("Canvas instances are owned by DisplayManager")

    Q_PROPERTY(QString id READ id CONSTANT FINAL)
    Q_PROPERTY(QString name READ name NOTIFY changed FINAL)
    Q_PROPERTY(QString displayLabel READ name NOTIFY changed FINAL)
    Q_PROPERTY(QVariantList members READ members NOTIFY changed FINAL)
    Q_PROPERTY(QVariantMap extent READ extent NOTIFY changed FINAL)
    Q_PROPERTY(quint32 width READ width NOTIFY changed FINAL)
    Q_PROPERTY(quint32 height READ height NOTIFY changed FINAL)
    Q_PROPERTY(QVariantMap layoutOverride READ layoutOverride NOTIFY changed FINAL)
    Q_PROPERTY(QVariantMap effectiveLayout READ effectiveLayout NOTIFY changed FINAL)
    Q_PROPERTY(QVariantList links READ links NOTIFY runtimeChanged FINAL)
    Q_PROPERTY(qint64 activePlaylistId READ activePlaylistId NOTIFY runtimeChanged FINAL)
    Q_PROPERTY(QVariantMap playlistStatus READ playlistStatus NOTIFY runtimeChanged FINAL)
    Q_PROPERTY(QVariantList runtimeConditions READ runtimeConditions NOTIFY runtimeChanged FINAL)
    Q_PROPERTY(QString wallpaperId READ wallpaperId NOTIFY changed FINAL)
    Q_PROPERTY(quint64 revision READ revision NOTIFY changed FINAL)
    Q_PROPERTY(int memberCount READ memberCount NOTIFY changed FINAL)
    Q_PROPERTY(int onlineCount READ onlineCount NOTIFY changed FINAL)
    Q_PROPERTY(bool hasLiveDisplays READ hasLiveDisplays NOTIFY changed FINAL)
    Q_PROPERTY(bool empty READ empty NOTIFY changed FINAL)

public:
    explicit Canvas(const proto::CanvasInfo& info, QObject* parent = nullptr);

    auto id() const -> const QString& { return m_id; }
    auto name() const -> const QString& { return m_name; }
    auto members() const -> const QVariantList& { return m_members; }
    auto extent() const -> const QVariantMap& { return m_extent; }
    auto width() const -> quint32 { return m_width; }
    auto height() const -> quint32 { return m_height; }
    auto layoutOverride() const -> const QVariantMap& { return m_layout_override; }
    auto effectiveLayout() const -> const QVariantMap& { return m_effective_layout; }
    auto links() const -> const QVariantList& { return m_links; }
    auto activePlaylistId() const -> qint64 { return m_active_playlist_id; }
    auto playlistStatus() const -> const QVariantMap& { return m_playlist_status; }
    auto runtimeConditions() const -> const QVariantList& { return m_runtime_conditions; }
    auto wallpaperId() const -> const QString& { return m_wallpaper_id; }
    auto revision() const -> quint64 { return m_revision; }
    auto memberCount() const -> int { return m_members.size(); }
    auto onlineCount() const -> int { return m_online_count; }
    auto hasLiveDisplays() const -> bool { return m_online_count > 0; }
    auto empty() const -> bool { return m_members.isEmpty(); }

    void updateFrom(const proto::CanvasInfo& info);
    void updateRuntime(const QList<Display*>& displays);

    Q_SIGNAL void changed();
    Q_SIGNAL void runtimeChanged();

private:
    static auto membersFromPb(const proto::CanvasInfo& info) -> QVariantList;
    static auto rectFromPb(const proto::CanvasRect& rect) -> QVariantMap;
    static auto layoutOverrideFromPb(const proto::CanvasInfo& info) -> QVariantMap;
    static auto effectiveLayoutFromPb(const proto::CanvasInfo& info) -> QVariantMap;

    QString      m_id;
    QString      m_name;
    QVariantList m_members;
    QVariantMap  m_extent;
    quint32      m_width { 0 };
    quint32      m_height { 0 };
    QVariantMap  m_layout_override;
    QVariantMap  m_effective_layout;
    QString      m_wallpaper_id;
    quint64      m_revision { 0 };
    int          m_online_count { 0 };
    QVariantList m_links;
    qint64       m_active_playlist_id { 0 };
    QVariantMap  m_playlist_status;
    QVariantList m_runtime_conditions;
};

/// Singleton model for all currently-registered displays. Fed by:
///   1. the snapshot that arrives on ws connect (via `Backend::eventReceived`),
///   2. subsequent `DisplayChanged` / `DisplayRemoved` events,
///   3. `DisplayListQuery::reload` as a fallback refresh path.
///
/// Consumers should prefer reading from `DisplayManager` over issuing
/// a fresh `DisplayListRequest` — the manager is push-updated.
class DisplayManager : public QObject {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QVariantList displays READ displays NOTIFY displaysChanged FINAL)
    Q_PROPERTY(QVariantList canvases READ canvases NOTIFY canvasesChanged FINAL)
    Q_PROPERTY(quint64 canvasRevision READ canvasRevision NOTIFY canvasesChanged FINAL)
    Q_PROPERTY(int count READ count NOTIFY displaysChanged FINAL)
    Q_PROPERTY(bool hasActivePlaylistDisplays READ hasActivePlaylistDisplays NOTIFY
                   playlistStatusChanged FINAL)

public:
    DisplayManager(QObject* parent = nullptr);
    ~DisplayManager() override;

    static auto instance() -> DisplayManager*;

    /// Snapshot of all displays (ordered by ascending id) as a list of
    /// `Display*`, suitable for QML `Repeater { model: DisplayManager.displays }`.
    auto displays() const -> QVariantList;
    auto canvases() const -> QVariantList;
    auto count() const -> int { return (int)m_ordered.size(); }
    auto canvasRevision() const -> quint64 { return m_canvas_revision; }
    auto hasActivePlaylistDisplays() const -> bool;

    Q_INVOKABLE waywallen::Display* get(quint64 id) const;
    Q_INVOKABLE waywallen::Canvas* getCanvas(const QString& id) const;

    /// Full replace. Removes any id not present in `list`, upserts the rest.
    /// Exactly-once `displaysChanged` after the batch.
    void replaceAll(const QList<proto::DisplayInfo>& list);

    /// Upsert a single display; emits `displaysChanged` only if this
    /// was an add (removal/add changes the ordered list). Property
    /// changes on an existing display emit per-property signals.
    void upsert(const proto::DisplayInfo& info);

    /// Remove by id. Emits `displaysChanged` if the id existed.
    void remove(quint64 id);
    void replaceCanvases(const QList<proto::CanvasInfo>& list, quint64 revision);

    /// Full replacement of current playlist runtime state by display id.
    /// Missing displays are treated as inactive.
    void replacePlaylistStatuses(const QList<proto::PlaylistDisplayStatus>& list);

    /// Wire up to a backend's `eventReceived` signal. Call once from
    /// `App::init` after the backend is constructed.
    void attachTo(Backend* backend);

    Q_SIGNAL void displaysChanged();
    Q_SIGNAL void canvasesChanged();
    Q_SIGNAL void playlistStatusChanged();

private:
    void handleEvent(const proto::Event& evt);
    void refreshCanvasRuntime();

    QList<Display*>             m_ordered; // sorted by id
    std::map<quint64, Display*> m_by_id;
    QList<Canvas*>              m_canvases;
    std::map<QString, Canvas*>  m_canvas_by_id;
    quint64                     m_canvas_revision { 0 };
};

} // namespace waywallen
