use super::*;

fn parse_user_property_overrides(item_id: i64, raw: Option<&str>) -> HashMap<String, String> {
    let Some(raw) = raw else {
        return HashMap::new();
    };
    match serde_json::from_str::<HashMap<String, String>>(raw) {
        Ok(properties) => crate::catalog::properties::normalize_user_property_overrides(properties),
        Err(error) => {
            log::warn!(
                "item {item_id}: user_property_overrides JSON unparseable ({error}); treating as empty"
            );
            HashMap::new()
        }
    }
}

pub async fn get_user_property_overrides(
    db: &DatabaseConnection,
    item_id: i64,
) -> Result<HashMap<String, String>> {
    let row = item::Entity::find_by_id(item_id)
        .one(db)
        .await
        .with_context(|| format!("select item by id={item_id} for overrides"))?;
    let Some(item) = row else {
        return Ok(HashMap::new());
    };
    Ok(parse_user_property_overrides(
        item_id,
        item.user_property_overrides.as_deref(),
    ))
}

pub async fn get_user_property_overrides_raw(
    db: &DatabaseConnection,
    item_id: i64,
) -> Result<Option<String>> {
    let row = item::Entity::find_by_id(item_id)
        .one(db)
        .await
        .with_context(|| format!("select item by id={item_id} for raw overrides"))?;
    Ok(row
        .and_then(|item| item.user_property_overrides)
        .map(|raw| crate::catalog::properties::normalize_user_property_overrides_json(&raw)))
}

fn parse_wallpaper_layout_override_raw(
    item_id: i64,
    raw: Option<&str>,
) -> Option<crate::catalog::properties::WallpaperLayoutOverride> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed = crate::catalog::properties::wallpaper_layout_override_from_json(raw);
    if parsed.is_none() {
        log::warn!("item {item_id}: wallpaper_layout_override JSON unparseable; ignoring");
    }
    parsed
}

pub async fn get_wallpaper_render_properties(
    db: &DatabaseConnection,
    item_id: i64,
) -> Result<(
    HashMap<String, String>,
    crate::catalog::properties::WallpaperLayoutOverride,
)> {
    let row = item::Entity::find_by_id(item_id)
        .one(db)
        .await
        .with_context(|| format!("select item by id={item_id} for render properties"))?;
    let Some(item) = row else {
        return Ok((HashMap::new(), Default::default()));
    };
    let user_properties =
        parse_user_property_overrides(item_id, item.user_property_overrides.as_deref());
    let (renderer_properties, legacy_layout) =
        crate::catalog::properties::split_renderer_properties(user_properties);
    let layout =
        parse_wallpaper_layout_override_raw(item_id, item.wallpaper_layout_override.as_deref())
            .unwrap_or(legacy_layout);
    Ok((renderer_properties, layout))
}

pub async fn get_wallpaper_layout_override_with_legacy(
    db: &DatabaseConnection,
    item_id: i64,
) -> Result<Option<crate::catalog::properties::WallpaperLayoutOverride>> {
    let (_, layout) = get_wallpaper_render_properties(db, item_id).await?;
    Ok((!layout.is_empty()).then_some(layout))
}

pub async fn set_wallpaper_layout_override(
    db: &DatabaseConnection,
    item_id: i64,
    layout: Option<crate::settings::ResolvedLayout>,
) -> Result<()> {
    let clearing = layout.is_none();
    let serialized = layout
        .map(crate::catalog::properties::wallpaper_layout_override_to_json)
        .transpose()
        .context("serialize wallpaper_layout_override")?;
    let user_property_overrides = if clearing {
        let mut current = get_user_property_overrides(db, item_id).await?;
        current.retain(|key, _| !crate::catalog::properties::is_daemon_display_property_key(key));
        let serialized = if current.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&current).context("serialize user_property_overrides")?)
        };
        Set(serialized)
    } else {
        NotSet
    };
    let active = item::ActiveModel {
        id: Set(item_id),
        wallpaper_layout_override: Set(serialized),
        user_property_overrides,
        ..Default::default()
    };
    item::Entity::update(active)
        .exec(db)
        .await
        .with_context(|| format!("update item {item_id} wallpaper_layout_override"))?;
    Ok(())
}

pub async fn set_user_property_override(
    db: &DatabaseConnection,
    item_id: i64,
    key: &str,
    value: Option<&str>,
) -> Result<()> {
    let mut current = get_user_property_overrides(db, item_id).await?;
    let key = crate::catalog::properties::canonical_user_property_key(key);
    if let Some(value) = value {
        current.insert(key.to_string(), value.to_string());
    } else {
        current.remove(key);
    }
    let serialized = if current.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&current).context("serialize user_property_overrides")?)
    };
    let active = item::ActiveModel {
        id: Set(item_id),
        user_property_overrides: Set(serialized),
        ..Default::default()
    };
    item::Entity::update(active)
        .exec(db)
        .await
        .with_context(|| format!("update item {item_id} user_property_overrides"))?;
    Ok(())
}
