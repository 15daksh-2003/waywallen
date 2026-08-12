use std::sync::Arc;

use crate::error::Result;
use crate::model::repo;
use crate::model::repo::playlists as plrepo;
use crate::DaemonContext;

pub async fn resolve(app: &Arc<DaemonContext>, playlist_id: i64) -> Result<Vec<String>> {
    let ids = plrepo::entry_ids(&app.db, playlist_id).await?;
    let mut filtered = Vec::with_capacity(ids.len());
    for id in &ids {
        if repo::get_entry(&app.db, *id).await?.is_some() {
            filtered.push(id.to_string());
        }
    }
    Ok(if filtered.is_empty() {
        ids.iter().map(|i| i.to_string()).collect()
    } else {
        filtered
    })
}
