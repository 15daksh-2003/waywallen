module;
#include "waywallen/objmodel/renderer.moc.h"

#undef assert
#include <rstd/macro.hpp>

module waywallen;
import :renderer;

using namespace Qt::Literals::StringLiterals;
using namespace qextra::prelude;
using namespace rstd::prelude;

namespace proto = waywallen::control::v1;

namespace waywallen
{

// ---------------------------------------------------------------------------
// Renderer
// ---------------------------------------------------------------------------

static auto exitReason(bool present, const proto::RendererExit& exit) -> QString {
    return present ? exit.reason() : QString {};
}

static auto rendererStateFromPb(const proto::RendererInstance& info) -> RendererStateValue {
    if (! info.hasState()) return RendererStateValue::Unknown();
    const auto& state = info.state();
    if (state.hasStarting()) {
        return RendererStateValue::Starting(state.starting().generation());
    }
    if (state.hasRunning()) {
        const auto& running = state.running();
        return RendererStateValue::Running(running.generation(), running.activity());
    }
    if (state.hasStopping()) {
        const auto& stopping = state.stopping();
        return RendererStateValue::Stopping(stopping.generation(), stopping.keep());
    }
    if (state.hasStopped()) {
        const auto& stopped = state.stopped();
        return RendererStateValue::Stopped(stopped.keep(),
                                           exitReason(stopped.hasLastExit(), stopped.lastExit()));
    }
    if (state.hasKilled()) {
        const auto& killed = state.killed();
        return RendererStateValue::Killed(killed.keep(),
                                          exitReason(killed.hasLastExit(), killed.lastExit()));
    }
    if (state.hasFailed()) {
        const auto& failed = state.failed();
        return RendererStateValue::Failed(exitReason(failed.hasFailure(), failed.failure()));
    }
    return RendererStateValue::Unknown();
}

Renderer::Renderer(const proto::RendererInstance& info, QObject* parent)
    : QObject(parent),
      m_id(info.rendererId()),
      m_fps(info.fps()),
      m_state(rendererStateFromPb(info)),
      m_name(info.name()),
      m_pid(info.pid()),
      m_texture_width(info.textureWidth()),
      m_texture_height(info.textureHeight()),
      m_drm_render_major(info.drmRenderMajor()),
      m_drm_render_minor(info.drmRenderMinor()),
      m_runtime_conditions(runtimeConditionsFromPb(info.conditions())),
      m_runtime_tags(runtimeTagsFromPb(info.runtimeTags())) {}

auto Renderer::state() const -> State {
    using Tag = RendererStateValue::Tag;
    switch (m_state.tag()) {
    case Tag::Starting: return State::Starting;
    case Tag::Running:
        switch (m_state.as_Running().activity) {
        case proto::RendererActivity::RENDERER_ACTIVITY_PLAYING: return State::Playing;
        case proto::RendererActivity::RENDERER_ACTIVITY_PAUSED: return State::Paused;
        case proto::RendererActivity::RENDERER_ACTIVITY_MUTED: return State::Muted;
        default: return State::Unknown;
        }
    case Tag::Stopping: return State::Stopping;
    case Tag::Stopped: return State::Stopped;
    case Tag::Killed: return State::Killed;
    case Tag::Failed: return State::Failed;
    case Tag::Unknown: return State::Unknown;
    }
}

auto Renderer::status() const -> QString {
    switch (state()) {
    case State::Starting: return u"starting"_s;
    case State::Playing: return u"playing"_s;
    case State::Paused: return u"paused"_s;
    case State::Muted: return u"muted"_s;
    case State::Stopping: return u"stopping"_s;
    case State::Stopped: return u"stopped"_s;
    case State::Killed: return u"killed"_s;
    case State::Failed: return u"failed"_s;
    case State::Unknown: return {};
    }
}

auto Renderer::running() const -> bool {
    const auto value = state();
    return value == State::Playing || value == State::Paused || value == State::Muted;
}

auto Renderer::keep() const -> bool {
    if (m_state.is_Stopping()) return m_state.as_Stopping().keep;
    if (m_state.is_Stopped()) return m_state.as_Stopped().keep;
    if (m_state.is_Killed()) return m_state.as_Killed().keep;
    return false;
}

auto Renderer::processGeneration() const -> quint64 {
    if (m_state.is_Starting()) return m_state.as_Starting().generation;
    if (m_state.is_Running()) return m_state.as_Running().generation;
    if (m_state.is_Stopping()) return m_state.as_Stopping().generation;
    return 0;
}

auto Renderer::lastExitReason() const -> QString {
    if (m_state.is_Stopped()) return m_state.as_Stopped().reason;
    if (m_state.is_Killed()) return m_state.as_Killed().reason;
    if (m_state.is_Failed()) return m_state.as_Failed().reason;
    return {};
}

void Renderer::updateFrom(const proto::RendererInstance& info) {
    rstd_assert(info.rendererId() == m_id, "Renderer::updateFrom id mismatch");

    if (m_fps != info.fps()) {
        m_fps = info.fps();
        Q_EMIT fpsChanged();
    }
    const auto old_state      = state();
    const auto old_keep       = keep();
    const auto old_generation = processGeneration();
    const auto old_reason     = lastExitReason();
    m_state                   = rendererStateFromPb(info);
    if (old_state != state() || old_keep != keep() || old_generation != processGeneration() ||
        old_reason != lastExitReason()) {
        Q_EMIT stateChanged();
    }
    if (m_name != info.name()) {
        m_name = info.name();
        Q_EMIT nameChanged();
    }
    if (m_pid != info.pid()) {
        m_pid = info.pid();
        Q_EMIT pidChanged();
    }
    if (m_texture_width != info.textureWidth() || m_texture_height != info.textureHeight()) {
        m_texture_width  = info.textureWidth();
        m_texture_height = info.textureHeight();
        Q_EMIT textureSizeChanged();
    }
    if (m_drm_render_major != info.drmRenderMajor() ||
        m_drm_render_minor != info.drmRenderMinor()) {
        m_drm_render_major = info.drmRenderMajor();
        m_drm_render_minor = info.drmRenderMinor();
        Q_EMIT drmRenderChanged();
    }
    auto conditions = runtimeConditionsFromPb(info.conditions());
    if (m_runtime_conditions != conditions) {
        m_runtime_conditions = std::move(conditions);
        Q_EMIT runtimeConditionsChanged();
    }
    auto tags = runtimeTagsFromPb(info.runtimeTags());
    if (m_runtime_tags != tags) {
        m_runtime_tags = std::move(tags);
        Q_EMIT runtimeTagsChanged();
    }
}

// ---------------------------------------------------------------------------
// RendererManager
// ---------------------------------------------------------------------------

static auto rm_instance(RendererManager* in = nullptr) -> RendererManager* {
    static RendererManager* instance { in };
    if (in && instance != in) instance = in;
    return instance;
}

RendererManager::RendererManager(QObject* parent): QObject(parent) { rm_instance(this); }

RendererManager::~RendererManager() {
    if (rm_instance() == this) {
        // best-effort: leave static pointer dangling only if app is tearing
        // down anyway; no other lifecycle consumer.
    }
}

auto RendererManager::instance() -> RendererManager* { return rm_instance(); }

auto RendererManager::renderers() const -> QVariantList {
    QVariantList out;
    out.reserve(m_ordered.size());
    for (auto* r : m_ordered) out.append(QVariant::fromValue(r));
    return out;
}

auto RendererManager::get(const QString& id) const -> Renderer* {
    auto it = m_by_id.find(id);
    return (it == m_by_id.end()) ? nullptr : it->second;
}

void RendererManager::replaceAll(const QList<proto::RendererInstance>& list) {
    std::map<QString, Renderer*> next_by_id;
    QList<Renderer*>             next_ordered;
    next_ordered.reserve(list.size());

    for (const auto& info : list) {
        auto id = info.rendererId();
        auto it = m_by_id.find(id);
        if (it != m_by_id.end()) {
            it->second->updateFrom(info);
            next_by_id[id] = it->second;
            next_ordered.append(it->second);
            m_by_id.erase(it);
        } else {
            auto* r        = new Renderer(info, this);
            next_by_id[id] = r;
            next_ordered.append(r);
        }
    }
    // Anything left in m_by_id was not in the new snapshot → drop it.
    for (auto& [id, r] : m_by_id) r->deleteLater();
    m_by_id.clear();

    // Stable ordering by id for UI determinism.
    std::sort(next_ordered.begin(), next_ordered.end(), [](Renderer* a, Renderer* b) {
        return a->id() < b->id();
    });

    m_ordered = std::move(next_ordered);
    m_by_id   = std::move(next_by_id);
    Q_EMIT renderersChanged();
}

void RendererManager::upsert(const proto::RendererInstance& info) {
    auto id = info.rendererId();
    auto it = m_by_id.find(id);
    if (it != m_by_id.end()) {
        it->second->updateFrom(info);
        return;
    }
    auto* r     = new Renderer(info, this);
    m_by_id[id] = r;
    auto pos =
        std::upper_bound(m_ordered.begin(), m_ordered.end(), id, [](const QString& v, Renderer* x) {
            return v < x->id();
        });
    m_ordered.insert(pos, r);
    Q_EMIT renderersChanged();
}

void RendererManager::remove(const QString& id) {
    auto it = m_by_id.find(id);
    if (it == m_by_id.end()) return;
    auto* r = it->second;
    m_by_id.erase(it);
    m_ordered.removeOne(r);
    r->deleteLater();
    Q_EMIT renderersChanged();
}

void RendererManager::attachTo(Backend* backend) {
    connect(backend,
            &Backend::eventReceived,
            this,
            &RendererManager::handleEvent,
            Qt::QueuedConnection);
}

void RendererManager::handleEvent(const proto::Event& evt) {
    if (evt.hasRendererSnapshot()) {
        const auto& snap = evt.rendererSnapshot();
        replaceAll(snap.renderers());
    } else if (evt.hasRendererChanged()) {
        upsert(evt.rendererChanged().renderer());
    } else if (evt.hasRendererRemoved()) {
        remove(evt.rendererRemoved().rendererId());
    }
}

} // namespace waywallen

#include "waywallen/objmodel/renderer.moc.cpp"
