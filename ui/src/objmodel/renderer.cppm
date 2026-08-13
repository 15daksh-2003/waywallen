module;
#include "QExtra/macro_qt.hpp"

#ifdef Q_MOC_RUN
#    include "waywallen/objmodel/renderer.moc"
#endif

export module waywallen:renderer;
export import :proto;
export import :backend;
import rstd;
import rstd.cppstd;
import qextra;

using rstd::boxed::Box;

namespace proto = waywallen::control::v1;

export namespace waywallen
{

/// One renderer, mirroring `proto::RendererInstance` as a QObject so
/// QML can bind directly to its fields. Identity is `id()`; mutate via
/// `updateFrom(info)` which diff-emits per changed property.
class Renderer : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_UNCREATABLE("Renderer instances are owned by RendererManager")

    Q_PROPERTY(QString id READ id CONSTANT FINAL)
    Q_PROPERTY(quint32 fps READ fps NOTIFY fpsChanged FINAL)
    Q_PROPERTY(QString status READ status NOTIFY statusChanged FINAL)
    Q_PROPERTY(int processState READ processState NOTIFY processStateChanged FINAL)
    Q_PROPERTY(int activityState READ activityState NOTIFY activityStateChanged FINAL)
    Q_PROPERTY(bool keep READ keep NOTIFY keepChanged FINAL)
    Q_PROPERTY(
        quint64 processGeneration READ processGeneration NOTIFY processGenerationChanged FINAL)
    Q_PROPERTY(QString lastExitReason READ lastExitReason NOTIFY lastExitChanged FINAL)
    Q_PROPERTY(QString name READ name NOTIFY nameChanged FINAL)
    Q_PROPERTY(quint32 pid READ pid NOTIFY pidChanged FINAL)
    Q_PROPERTY(quint32 textureWidth READ textureWidth NOTIFY textureSizeChanged FINAL)
    Q_PROPERTY(quint32 textureHeight READ textureHeight NOTIFY textureSizeChanged FINAL)
    Q_PROPERTY(
        QVariantList runtimeConditions READ runtimeConditions NOTIFY runtimeConditionsChanged FINAL)
    Q_PROPERTY(QVariantList runtimeTags READ runtimeTags NOTIFY runtimeTagsChanged FINAL)
    Q_PROPERTY(quint32 drmRenderMajor READ drmRenderMajor NOTIFY drmRenderChanged FINAL)
    Q_PROPERTY(quint32 drmRenderMinor READ drmRenderMinor NOTIFY drmRenderChanged FINAL)

public:
    explicit Renderer(const proto::RendererInstance& info, QObject* parent = nullptr);

    auto id() const -> const QString& { return m_id; }
    auto fps() const -> quint32 { return m_fps; }
    auto status() const -> const QString& { return m_status; }
    auto processState() const -> int { return m_process_state; }
    auto activityState() const -> int { return m_activity_state; }
    auto keep() const -> bool { return m_keep; }
    auto processGeneration() const -> quint64 { return m_process_generation; }
    auto lastExitReason() const -> const QString& { return m_last_exit_reason; }
    auto name() const -> const QString& { return m_name; }
    auto pid() const -> quint32 { return m_pid; }
    auto textureWidth() const -> quint32 { return m_texture_width; }
    auto textureHeight() const -> quint32 { return m_texture_height; }
    auto drmRenderMajor() const -> quint32 { return m_drm_render_major; }
    auto drmRenderMinor() const -> quint32 { return m_drm_render_minor; }
    auto runtimeConditions() const -> const QVariantList& { return m_runtime_conditions; }
    auto runtimeTags() const -> const QVariantList& { return m_runtime_tags; }

    /// Diff-update from a freshly-received `RendererInstance`. Only emits
    /// the signals for properties that actually changed.
    void updateFrom(const proto::RendererInstance& info);

    Q_SIGNAL void fpsChanged();
    Q_SIGNAL void statusChanged();
    Q_SIGNAL void processStateChanged();
    Q_SIGNAL void activityStateChanged();
    Q_SIGNAL void keepChanged();
    Q_SIGNAL void processGenerationChanged();
    Q_SIGNAL void lastExitChanged();
    Q_SIGNAL void nameChanged();
    Q_SIGNAL void pidChanged();
    Q_SIGNAL void textureSizeChanged();
    Q_SIGNAL void drmRenderChanged();
    Q_SIGNAL void runtimeConditionsChanged();
    Q_SIGNAL void runtimeTagsChanged();

private:
    QString      m_id;
    quint32      m_fps;
    QString      m_status;
    int          m_process_state;
    int          m_activity_state;
    bool         m_keep;
    quint64      m_process_generation;
    QString      m_last_exit_reason;
    QString      m_name;
    quint32      m_pid;
    quint32      m_texture_width;
    quint32      m_texture_height;
    quint32      m_drm_render_major;
    quint32      m_drm_render_minor;
    QVariantList m_runtime_conditions;
    QVariantList m_runtime_tags;
};

/// Singleton model for all currently-registered renderers. Fed by:
///   1. the snapshot that arrives on ws connect (via `Backend::eventReceived`),
///   2. subsequent `RendererChanged` / `RendererRemoved` events,
///   3. `RendererListQuery::reload` as a fallback refresh path.
///
/// Consumers should prefer reading from `RendererManager` over issuing
/// a fresh `RendererListRequest` — the manager is push-updated.
class RendererManager : public QObject {
    Q_OBJECT
    QML_ELEMENT

    Q_PROPERTY(QVariantList renderers READ renderers NOTIFY renderersChanged FINAL)
    Q_PROPERTY(int count READ count NOTIFY renderersChanged FINAL)

public:
    RendererManager(QObject* parent = nullptr);
    ~RendererManager() override;

    static auto instance() -> RendererManager*;

    /// Snapshot of all renderers (ordered by ascending id) as a list of
    /// `Renderer*`, suitable for QML `Repeater { model: RendererManager.renderers }`.
    auto renderers() const -> QVariantList;
    auto count() const -> int { return (int)m_ordered.size(); }

    Q_INVOKABLE waywallen::Renderer* get(const QString& id) const;

    /// Full replace. Removes any id not present in `list`, upserts the rest.
    /// Exactly-once `renderersChanged` after the batch.
    void replaceAll(const QList<proto::RendererInstance>& list);

    /// Upsert a single renderer; emits `renderersChanged` only if this
    /// was an add (removal/add changes the ordered list). Property
    /// changes on an existing renderer emit per-property signals.
    void upsert(const proto::RendererInstance& info);

    /// Remove by id. Emits `renderersChanged` if the id existed.
    void remove(const QString& id);

    /// Wire up to a backend's `eventReceived` signal. Call once from
    /// `App::init` after the backend is constructed.
    void attachTo(Backend* backend);

    Q_SIGNAL void renderersChanged();

private:
    void handleEvent(const proto::Event& evt);

    QList<Renderer*>             m_ordered; // sorted by id
    std::map<QString, Renderer*> m_by_id;
};

} // namespace waywallen
