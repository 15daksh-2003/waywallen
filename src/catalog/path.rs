/// Strip `root` from `resource` and return the path remainder.
/// Returns `None` when `resource` is outside `root`.
pub fn relative_under_root(root: &str, resource: &str) -> Option<String> {
    let root = root.trim_end_matches('/');
    let rest = resource.strip_prefix(root)?;
    Some(rest.trim_start_matches('/').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_root_and_leading_separator() {
        assert_eq!(
            relative_under_root("/library/", "/library/folder/image.png").as_deref(),
            Some("folder/image.png")
        );
        assert!(relative_under_root("/other", "/library/image.png").is_none());
    }
}
