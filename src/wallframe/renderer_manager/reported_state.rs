use super::*;

fn runtime_tag_key_valid(key: &str) -> bool {
    let bytes = key.as_bytes();
    bytes.len() <= MAX_RUNTIME_TAG_KEY_BYTES
        && bytes.first().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes[1..].iter().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(*byte, b'.' | b'_' | b'-')
        })
}

pub(super) fn validate_runtime_tags(
    tags: &[(String, String)],
) -> std::result::Result<Vec<RendererRuntimeTag>, String> {
    if tags.len() > MAX_RUNTIME_TAGS {
        return Err(format!(
            "runtime_tags contains {} entries; maximum is {MAX_RUNTIME_TAGS}",
            tags.len()
        ));
    }
    let mut seen = HashSet::with_capacity(tags.len());
    let mut validated = Vec::with_capacity(tags.len());
    for (index, (key, value)) in tags.iter().enumerate() {
        if !runtime_tag_key_valid(key) {
            return Err(format!("runtime_tags[{index}] has invalid key {key:?}"));
        }
        if value.is_empty()
            || value.len() > MAX_RUNTIME_TAG_VALUE_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(format!(
                "runtime_tags[{index}] has an invalid value for key {key:?}"
            ));
        }
        if !seen.insert(key.as_str()) {
            return Err(format!("runtime_tags contains duplicate key {key:?}"));
        }
        validated.push(RendererRuntimeTag {
            key: key.clone(),
            value: value.clone(),
        });
    }
    Ok(validated)
}

pub(super) fn apply_renderer_state_patch(
    current: &mut RendererReportedState,
    patch: &RendererState,
) -> std::result::Result<u32, String> {
    if patch.fields == 0 {
        return Err("fields is empty".to_string());
    }
    let unknown = patch.fields & !RENDERER_STATE_KNOWN_FIELDS;
    if unknown != 0 {
        return Err(format!("fields contains unknown bits 0x{unknown:x}"));
    }

    let mut next = current.clone();
    if patch.fields & RENDERER_STATE_FIELD_CLEAR_COLOR != 0 {
        let color = patch.clear_color;
        let rgba = [color.r, color.g, color.b, color.a];
        if !rgba.iter().all(|component| component.is_finite()) {
            return Err("clear_color contains a non-finite value".to_string());
        }
        next.clear_rgba = rgba.map(|component| component.clamp(0.0, 1.0));
    }
    if patch.fields & RENDERER_STATE_FIELD_RUNTIME_TAGS != 0 {
        next.runtime_tags = validate_runtime_tags(&patch.runtime_tags)?;
    }

    let mut changed = 0;
    if next.clear_rgba != current.clear_rgba {
        changed |= RENDERER_STATE_FIELD_CLEAR_COLOR;
    }
    if next.runtime_tags != current.runtime_tags {
        changed |= RENDERER_STATE_FIELD_RUNTIME_TAGS;
    }
    *current = next;
    Ok(changed)
}
