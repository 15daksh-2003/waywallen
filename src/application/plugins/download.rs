use std::path::{Path, PathBuf};

use anyhow::Context;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::error::{Error, Result};
use crate::plugin::update::PluginUpdateInfo;

pub(super) async fn download_archive(
    info: &PluginUpdateInfo,
    reporter: crate::tasks::ProgressReporter,
) -> Result<PathBuf> {
    let tmp_dir = std::env::temp_dir().join("waywallen-plugin-updates");
    tokio::fs::create_dir_all(&tmp_dir).await?;
    let unique = uuid::Uuid::new_v4();
    let part = tmp_dir.join(format!("{unique}.part"));
    let archive = tmp_dir.join(format!("{unique}.zip"));
    let result = download_archive_inner(info, &part, &archive, reporter).await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
        let _ = tokio::fs::remove_file(&archive).await;
    }
    result
}

async fn download_archive_inner(
    info: &PluginUpdateInfo,
    part: &Path,
    archive: &Path,
    reporter: crate::tasks::ProgressReporter,
) -> Result<PathBuf> {
    let expected = normalize_sha256(&info.sha256)?;
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) waywallen")
        .build()
        .context("build plugin update download client")?;
    let response = client
        .get(&info.zip_url)
        .send()
        .await
        .with_context(|| format!("download plugin update {}", info.zip_url))?
        .error_for_status()
        .with_context(|| format!("download plugin update response {}", info.zip_url))?;
    let total = response.content_length();
    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(part).await?;
    let mut hasher = Sha256::new();
    let mut received = 0u64;

    reporter.report(0.10, "");
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("download plugin update chunk")?;
        file.write_all(&chunk).await?;
        hasher.update(&chunk);
        received = received.saturating_add(chunk.len() as u64);
        let progress = total
            .filter(|total| *total > 0)
            .map(|total| received as f32 / total as f32)
            .unwrap_or(0.5);
        reporter.report(0.10 + progress.clamp(0.0, 1.0) * 0.65, "");
    }
    file.flush().await?;
    drop(file);

    if hex_lower(&hasher.finalize()) != expected {
        return Err(Error::PluginInstallFailed(format!(
            "plugin '{}' update sha256 mismatch",
            info.plugin_id
        )));
    }
    reporter.report(0.78, "");
    tokio::fs::rename(part, archive).await?;
    Ok(archive.to_path_buf())
}

fn normalize_sha256(value: &str) -> Result<String> {
    let trimmed = value.trim().to_ascii_lowercase();
    if trimmed.len() == 64 && trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(trimmed)
    } else {
        Err(Error::PluginInstallFailed("invalid update sha256".into()))
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_normalization_requires_full_hex_digest() {
        assert!(normalize_sha256(&"A".repeat(64)).is_ok());
        assert!(normalize_sha256("abc").is_err());
        assert!(normalize_sha256(&"z".repeat(64)).is_err());
    }
}
