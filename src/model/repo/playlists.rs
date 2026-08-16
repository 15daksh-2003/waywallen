use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, Set, TransactionTrait,
};

use crate::error::{Result, ResultExt};
use crate::model::entities::{playlist, playlist_item};
use crate::playback::Mode;

#[derive(Debug, Clone)]
pub struct Summary {
    pub id: i64,
    pub name: String,
    pub mode: Mode,
    pub interval_secs: u32,
    pub item_count: u32,
}

pub async fn create(
    db: &DatabaseConnection,
    name: &str,
    mode: Mode,
    interval_secs: u32,
    now_ms: i64,
    entry_ids: &[i64],
) -> Result<i64> {
    let txn = db.begin().await.context("begin playlist create")?;
    let pl = playlist::ActiveModel {
        name: Set(name.to_owned()),
        mode: Set(mode.into()),
        interval_secs: Set(interval_secs as i64),
        created_at: Set(now_ms),
        updated_at: Set(now_ms),
        ..Default::default()
    }
    .insert(&txn)
    .await
    .context("insert playlist")?;
    insert_items(&txn, pl.id, entry_ids).await?;
    txn.commit().await.context("commit playlist create")?;
    Ok(pl.id)
}

async fn insert_items<C: sea_orm::ConnectionTrait>(
    conn: &C,
    playlist_id: i64,
    entry_ids: &[i64],
) -> Result<()> {
    for (pos, entry_id) in entry_ids.iter().enumerate() {
        playlist_item::ActiveModel {
            playlist_id: Set(playlist_id),
            entry_id: Set(*entry_id),
            position: Set(pos as i64),
            ..Default::default()
        }
        .insert(conn)
        .await
        .with_context(|| format!("insert playlist_item pl={playlist_id} entry={entry_id}"))?;
    }
    Ok(())
}

pub async fn list(db: &DatabaseConnection) -> Result<Vec<Summary>> {
    let pls = playlist::Entity::find()
        .order_by_desc(playlist::Column::Id)
        .all(db)
        .await
        .context("select playlists")?;
    let mut out = Vec::with_capacity(pls.len());
    for pl in pls {
        let count = playlist_item::Entity::find()
            .filter(playlist_item::Column::PlaylistId.eq(pl.id))
            .count(db)
            .await
            .context("count playlist_item")? as u32;
        out.push(Summary {
            id: pl.id,
            name: pl.name,
            mode: pl.mode.into(),
            interval_secs: u32::try_from(pl.interval_secs).unwrap_or(0),
            item_count: count,
        });
    }
    Ok(out)
}

pub async fn get(
    db: &DatabaseConnection,
    id: i64,
) -> Result<Option<crate::playback::playlist::Playlist>> {
    let model = playlist::Entity::find_by_id(id)
        .one(db)
        .await
        .with_context(|| format!("get playlist id={id}"))?;
    Ok(model.map(|model| crate::playback::playlist::Playlist {
        id: model.id,
        name: model.name,
        mode: model.mode.into(),
        interval_secs: u32::try_from(model.interval_secs).unwrap_or(0),
    }))
}

pub async fn delete(db: &DatabaseConnection, id: i64) -> Result<u64> {
    let res = playlist::Entity::delete_by_id(id)
        .exec(db)
        .await
        .with_context(|| format!("delete playlist id={id}"))?;
    Ok(res.rows_affected)
}

pub async fn rename(db: &DatabaseConnection, id: i64, name: &str, now_ms: i64) -> Result<()> {
    let mut am: playlist::ActiveModel = require(db, id).await?.into();
    am.name = Set(name.to_owned());
    am.updated_at = Set(now_ms);
    am.update(db).await.context("rename playlist")?;
    Ok(())
}

pub async fn set_mode(db: &DatabaseConnection, id: i64, mode: Mode, now_ms: i64) -> Result<()> {
    let mut am: playlist::ActiveModel = require(db, id).await?.into();
    am.mode = Set(mode.into());
    am.updated_at = Set(now_ms);
    am.update(db).await.context("set playlist mode")?;
    Ok(())
}

pub async fn set_interval(db: &DatabaseConnection, id: i64, secs: u32, now_ms: i64) -> Result<()> {
    let mut am: playlist::ActiveModel = require(db, id).await?.into();
    am.interval_secs = Set(secs as i64);
    am.updated_at = Set(now_ms);
    am.update(db).await.context("set playlist interval")?;
    Ok(())
}

pub async fn set_items(
    db: &DatabaseConnection,
    id: i64,
    entry_ids: &[i64],
    now_ms: i64,
) -> Result<()> {
    let txn = db.begin().await.context("begin set_items")?;
    let pl = require(&txn, id).await?;
    playlist_item::Entity::delete_many()
        .filter(playlist_item::Column::PlaylistId.eq(id))
        .exec(&txn)
        .await
        .context("clear playlist_item")?;
    insert_items(&txn, id, entry_ids).await?;
    let mut am: playlist::ActiveModel = pl.into();
    am.updated_at = Set(now_ms);
    am.update(&txn).await.context("touch playlist")?;
    txn.commit().await.context("commit set_items")?;
    Ok(())
}

pub async fn entry_ids(db: &DatabaseConnection, id: i64) -> Result<Vec<i64>> {
    let rows = playlist_item::Entity::find()
        .filter(playlist_item::Column::PlaylistId.eq(id))
        .order_by_asc(playlist_item::Column::Position)
        .all(db)
        .await
        .with_context(|| format!("list playlist_item pl={id}"))?;
    Ok(rows.into_iter().map(|r| r.entry_id).collect())
}

async fn require<C: sea_orm::ConnectionTrait>(conn: &C, id: i64) -> Result<playlist::Model> {
    playlist::Entity::find_by_id(id)
        .one(conn)
        .await
        .with_context(|| format!("require playlist id={id}"))?
        .ok_or_else(|| crate::error::Error::PlaylistNotFound(id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::repo as model_repo;

    async fn mem_db() -> DatabaseConnection {
        crate::model::connect_url("sqlite::memory:").await.unwrap()
    }

    async fn seed_items(db: &DatabaseConnection, count: usize) -> Vec<i64> {
        let plugin = model_repo::upsert_plugin(db, "playlist-test", "")
            .await
            .unwrap();
        let library = model_repo::add_library(db, plugin.id, "/playlist-test")
            .await
            .unwrap();
        let mut ids = Vec::with_capacity(count);
        for idx in 0..count {
            let path = format!("item-{idx}");
            let item = model_repo::upsert_item(
                db,
                model_repo::ItemUpsertArgs {
                    plugin_id: plugin.id,
                    library_id: library.id,
                    path: &path,
                    ty: "image",
                    display_name: "",
                    preview_path: None,
                    description: None,
                    external_id: None,
                    web_url: None,
                    size: None,
                    width: None,
                    height: None,
                    content_rating: None,
                },
            )
            .await
            .unwrap();
            ids.push(item.id);
        }
        ids
    }

    #[tokio::test]
    async fn create_list_delete_roundtrip() {
        let db = mem_db().await;
        let items = seed_items(&db, 1).await;
        let id = create(&db, "Nature", Mode::Shuffle, 300, 1, &items)
            .await
            .unwrap();
        let all = list(&db).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "Nature");
        assert_eq!(all[0].mode, Mode::Shuffle);
        assert_eq!(all[0].interval_secs, 300);
        assert_eq!(all[0].item_count, 1);
        assert_eq!(delete(&db, id).await.unwrap(), 1);
        assert!(list(&db).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn set_items_replaces_in_order() {
        let db = mem_db().await;
        let items = seed_items(&db, 3).await;
        let id = create(&db, "p", Mode::Sequential, 0, 1, &items[0..1])
            .await
            .unwrap();
        set_items(&db, id, &[items[1], items[2], items[0]], 2)
            .await
            .unwrap();
        assert_eq!(
            entry_ids(&db, id).await.unwrap(),
            vec![items[1], items[2], items[0]]
        );
        set_items(&db, id, &items[0..1], 3).await.unwrap();
        assert_eq!(entry_ids(&db, id).await.unwrap(), vec![items[0]]);
    }

    #[tokio::test]
    async fn delete_cascades_items() {
        let db = mem_db().await;
        let items = seed_items(&db, 2).await;
        let id = create(&db, "p", Mode::Sequential, 0, 1, &items)
            .await
            .unwrap();
        delete(&db, id).await.unwrap();
        assert!(entry_ids(&db, id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn mutators_touch_fields() {
        let db = mem_db().await;
        let items = seed_items(&db, 1).await;
        let id = create(&db, "p", Mode::Sequential, 0, 1, &items)
            .await
            .unwrap();
        rename(&db, id, "Renamed", 5).await.unwrap();
        set_mode(&db, id, Mode::Random, 6).await.unwrap();
        set_interval(&db, id, 60, 7).await.unwrap();
        let s = &list(&db).await.unwrap()[0];
        assert_eq!(s.name, "Renamed");
        assert_eq!(s.mode, Mode::Random);
        assert_eq!(s.interval_secs, 60);
    }
}
