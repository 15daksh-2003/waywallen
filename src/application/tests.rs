use super::*;

#[test]
fn force_shared_disables_duplicate_path() {
    assert!(!should_duplicate_renderers(
        true,
        true,
        RendererSharingPolicy::Shared
    ));
}

#[test]
fn duplicate_only_when_setting_and_targets() {
    assert!(should_duplicate_renderers(
        true,
        true,
        RendererSharingPolicy::UseSettings
    ));
    assert!(!should_duplicate_renderers(
        false,
        true,
        RendererSharingPolicy::UseSettings
    ));
    assert!(!should_duplicate_renderers(
        true,
        false,
        RendererSharingPolicy::UseSettings
    ));
    assert!(!should_duplicate_renderers(
        true,
        false,
        RendererSharingPolicy::Shared
    ));
}

#[test]
fn apply_sources_project_to_start_preemption_once() {
    let background = [
        ApplySource::QueueRotation,
        ApplySource::PlaylistRotation,
        ApplySource::PlaylistRebuild,
    ];
    for source in background {
        assert!(!source.preempts_pending_start(), "{}", source.as_str());
    }

    let immediate = [
        ApplySource::UserWallpaper,
        ApplySource::UserQueueStep,
        ApplySource::UserPlaylistActivation,
        ApplySource::UserPlaylistJump,
        ApplySource::StartupRestore,
        ApplySource::DisplayRecall,
        ApplySource::PlaylistAttach,
        ApplySource::PluginRestart,
    ];
    for source in immediate {
        assert!(source.preempts_pending_start(), "{}", source.as_str());
    }
}
