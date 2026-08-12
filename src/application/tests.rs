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
