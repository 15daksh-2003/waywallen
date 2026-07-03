use crate::error::{Error, Result};
use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

fn install_fail(msg: impl Into<String>) -> Error {
    Error::PluginInstallFailed(msg.into())
}

fn delete_fail(msg: impl Into<String>) -> Error {
    Error::PluginDeleteFailed(msg.into())
}

fn user_plugins_dir_path() -> Option<PathBuf> {
    crate::plugin::renderer_registry::standard_plugin_dirs("plugins")
        .into_iter()
        .next_back()
}

fn user_plugins_dir() -> Result<PathBuf> {
    user_plugins_dir_path().ok_or_else(|| install_fail("no user plugin directory"))
}

fn read_plugin_id(manifest: &Path) -> std::result::Result<String, String> {
    #[derive(serde::Deserialize)]
    struct Manifest {
        plugin: Plugin,
    }
    #[derive(serde::Deserialize)]
    struct Plugin {
        id: String,
    }
    let text = std::fs::read_to_string(manifest).map_err(|e| e.to_string())?;
    let m: Manifest = toml::from_str(&text).map_err(|e| format!("parse plugin.toml: {e}"))?;
    Ok(m.plugin.id)
}

/// Extract a plugin `.zip` into the user plugin directory and return the
/// installed plugin id. The archive must contain a top-level directory
pub fn install_zip(zip_path: &str) -> Result<String> {
    let file =
        std::fs::File::open(zip_path).map_err(|e| install_fail(format!("open {zip_path}: {e}")))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| install_fail(format!("read zip: {e}")))?;

    let dest_root = user_plugins_dir()?;
    std::fs::create_dir_all(&dest_root)
        .map_err(|e| install_fail(format!("mkdir {}: {e}", dest_root.display())))?;

    let mut top_dirs: BTreeSet<String> = BTreeSet::new();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| install_fail(e.to_string()))?;
        // `enclosed_name` rejects absolute paths and `..` traversal.
        let rel = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => {
                return Err(install_fail(format!(
                    "unsafe path in archive: {}",
                    entry.name()
                )))
            }
        };
        if let Some(std::path::Component::Normal(first)) = rel.components().next() {
            top_dirs.insert(first.to_string_lossy().into_owned());
        }
        let out = dest_root.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| install_fail(e.to_string()))?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| install_fail(e.to_string()))?;
            }
            let mut f = std::fs::File::create(&out).map_err(|e| install_fail(e.to_string()))?;
            std::io::copy(&mut entry, &mut f).map_err(|e| install_fail(e.to_string()))?;
        }
    }

    for d in &top_dirs {
        let manifest = dest_root.join(d).join("plugin.toml");
        if manifest.is_file() {
            let id = read_plugin_id(&manifest).map_err(|e| install_fail(e))?;
            log::info!(
                "installed plugin '{id}' into {}",
                dest_root.join(d).display()
            );
            return Ok(id);
        }
    }

    Err(install_fail(
        "archive must contain a top-level directory with plugin.toml",
    ))
}

pub fn delete_user_plugin(plugin_id: &str) -> Result<String> {
    if plugin_id.is_empty() {
        return Err(delete_fail("plugin id is empty"));
    }

    let dest_root =
        user_plugins_dir_path().ok_or_else(|| delete_fail("no user plugin directory"))?;
    let entries = match std::fs::read_dir(&dest_root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(delete_fail(format!("user plugin '{plugin_id}' not found")));
        }
        Err(e) => return Err(delete_fail(format!("read {}: {e}", dest_root.display()))),
    };
    let dest_root_canon = dest_root
        .canonicalize()
        .map_err(|e| delete_fail(format!("canonicalize {}: {e}", dest_root.display())))?;

    for entry in entries.flatten() {
        let plugin_dir = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let manifest = plugin_dir.join("plugin.toml");
        if !manifest.is_file() {
            continue;
        }
        let id = match read_plugin_id(&manifest) {
            Ok(id) => id,
            Err(e) => {
                log::warn!("skip {} while deleting plugin: {e}", manifest.display());
                continue;
            }
        };
        if id != plugin_id {
            continue;
        }

        let plugin_dir_canon = plugin_dir
            .canonicalize()
            .map_err(|e| delete_fail(format!("canonicalize {}: {e}", plugin_dir.display())))?;
        if !plugin_dir_canon.starts_with(&dest_root_canon) {
            return Err(delete_fail(format!(
                "plugin '{plugin_id}' is outside the user plugin directory"
            )));
        }

        std::fs::remove_dir_all(&plugin_dir)
            .map_err(|e| delete_fail(format!("delete {}: {e}", plugin_dir.display())))?;
        log::info!(
            "deleted user plugin '{plugin_id}' from {}",
            plugin_dir.display()
        );
        return Ok(id);
    }

    Err(delete_fail(format!("user plugin '{plugin_id}' not found")))
}
