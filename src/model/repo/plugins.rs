use super::*;

// source_plugin

/// Insert or refresh a `source_plugin` row keyed by `name`. `version`
/// is updated on every call so plugin upgrades are reflected in DB state.
pub async fn upsert_plugin(
    db: &DatabaseConnection,
    name: &str,
    version: &str,
) -> Result<source_plugin::Model> {
    if let Some(existing) = source_plugin::Entity::find()
        .filter(source_plugin::Column::Name.eq(name))
        .one(db)
        .await
        .with_context(|| format!("select plugin name={name}"))?
    {
        if existing.version == version {
            return Ok(existing);
        }
        let mut am: source_plugin::ActiveModel = existing.into();
        am.version = Set(version.to_owned());
        return am
            .update(db)
            .await
            .with_context(|| format!("update plugin version name={name}"));
    }
    let am = source_plugin::ActiveModel {
        name: Set(name.to_owned()),
        version: Set(version.to_owned()),
        ..Default::default()
    };
    am.insert(db)
        .await
        .with_context(|| format!("insert plugin name={name}"))
}

pub async fn list_plugins(db: &DatabaseConnection) -> Result<Vec<source_plugin::Model>> {
    source_plugin::Entity::find()
        .order_by_asc(source_plugin::Column::Id)
        .all(db)
        .await
        .context("select plugins")
}

pub async fn find_plugin_by_name(
    db: &DatabaseConnection,
    name: &str,
) -> Result<Option<source_plugin::Model>> {
    source_plugin::Entity::find()
        .filter(source_plugin::Column::Name.eq(name))
        .one(db)
        .await
        .with_context(|| format!("select plugin name={name}"))
}

pub async fn find_plugin_by_id(
    db: &DatabaseConnection,
    id: i64,
) -> Result<Option<source_plugin::Model>> {
    source_plugin::Entity::find_by_id(id)
        .one(db)
        .await
        .with_context(|| format!("select plugin id={id}"))
}

pub async fn remove_plugin(db: &DatabaseConnection, id: i64) -> Result<u64> {
    let res = source_plugin::Entity::delete_by_id(id)
        .exec(db)
        .await
        .with_context(|| format!("delete plugin id={id}"))?;
    Ok(res.rows_affected)
}

// ---------------------------------------------------------------------------
