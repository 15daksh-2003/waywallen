module;
#include "QExtra/macro_qt.hpp"
#ifndef Q_MOC_RUN
#    include <rstd/enum.hpp>
#endif

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

#ifndef Q_MOC_RUN
class RendererStateValue final {
    RSTD_ENUM_DEFAULT(RendererStateValue, (Unknown), (Unknown), (Starting, (quint64 generation;)),
                      (Running, (quint64 generation; proto::RendererActivity activity;)),
                      (Stopping, (quint64 generation; bool keep;)),
                      (Stopped, (bool keep; QString reason;)),
                      (Killed, (bool keep; QString reason;)), (Failed, (QString reason;)))
};
#else
class RendererStateValue;
#endif

/// One renderer, mirroring `proto::RendererInstance` as a QObject so
/// QML can bind directly to its fields. Identity is `id()`; mutate via
/// `updateFrom(info)` which diff-emits per changed property.
class Renderer : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_UNCREATABLE("Renderer instances are owned by RendererManager")

    Q_PROPERTY(QString id READ id CONSTANT FINAL)
    Q_PROPERTY(quint32 fps READ fps NOTIFY fpsChanged FINAL)
    Q_PROPERTY(State state READ state NOTIFY stateChanged FINAL)
    Q_PROPERTY(QString status READ status NOTIFY stateChanged FINAL)
    Q_PROPERTY(bool running READ running NOTIFY stateChanged FINAL)
    Q_PROPERTY(bool keep READ keep NOTIFY stateChanged FINAL)
    Q_PROPERTY(quint64 processGeneration READ processGeneration NOTIFY stateChanged FINAL)
    Q_PROPERTY(QString lastExitReason READ lastExitReason NOTIFY stateChanged FINAL)
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
    enum class State
    {
        Unknown,
        Starting,
        Playing,
        Paused,
        Muted,
        Stopping,
        Stopped,
        Killed,
        Failed,
    };
    Q_ENUM(State)

    explicit Renderer(const proto::RendererInstance& info, QObject* parent = nullptr);

    auto id() const -> const QString& { return m_id; }
    auto fps() const -> quint32 { return m_fps; }
    auto state() const -> State;
    auto status() const -> QString;
    auto running() const -> bool;
    auto keep() const -> bool;
    auto processGeneration() const -> quint64;
    auto lastExitReason() const -> QString;
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
    Q_SIGNAL void stateChanged();
    Q_SIGNAL void nameChanged();
    Q_SIGNAL void pidChanged();
    Q_SIGNAL void textureSizeChanged();
    Q_SIGNAL void drmRenderChanged();
    Q_SIGNAL void runtimeConditionsChanged();
    Q_SIGNAL void runtimeTagsChanged();

private:
    QString            m_id;
    quint32            m_fps;
    RendererStateValue m_state;
    QString            m_name;
    quint32            m_pid;
    quint32            m_texture_width;
    quint32            m_texture_height;
    quint32            m_drm_render_major;
    quint32            m_drm_render_minor;
    QVariantList       m_runtime_conditions;
    QVariantList       m_runtime_tags;
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
