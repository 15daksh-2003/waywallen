//! Persisted Steam sign-in: the refresh token (handed to DepotDownloader via
//! account.config), the access token (drives the workshop browse API), and the
//! account name (DepotDownloader's `-username`). The file's presence is the
//! single source of truth for "signed in".

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub account_name: String,
    pub refresh_token: String,
    pub access_token: String,
}

pub fn load() -> Option<Session> {
    let bytes = std::fs::read(crate::settings::steam_session_path()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub async fn save(session: &Session) -> Result<()> {
    let path = crate::settings::steam_session_path();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(session).context("serialize steam session")?;
    tokio::fs::write(&path, bytes)
        .await
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub async fn clear() -> Result<()> {
    let path = crate::settings::steam_session_path();
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("remove {}", path.display())),
    }
}
