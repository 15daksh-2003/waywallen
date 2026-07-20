use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};

use crate::error::{Error, Result};

pub const MAX_PLUGIN_STATE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct PluginStateStore {
    root: PathBuf,
    legacy_root: PathBuf,
}

impl PluginStateStore {
    pub fn new(root: PathBuf, legacy_root: PathBuf) -> Self {
        Self { root, legacy_root }
    }

    pub fn standard() -> Self {
        Self::new(
            crate::settings::plugin_state_dir(),
            crate::settings::data_dir(),
        )
    }

    fn state_path(&self, plugin_id: &str) -> PathBuf {
        self.root.join(format!(
            "{}.state",
            crate::settings::sanitize_path_segment(plugin_id)
        ))
    }

    pub fn load(&self, plugin_id: &str) -> Result<Option<String>> {
        let path = self.state_path(plugin_id);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(Error::Internal(anyhow!(
                    "read plugin state {}: {error}",
                    path.display()
                )))
            }
        };
        if bytes.len() > MAX_PLUGIN_STATE_BYTES {
            return Err(Error::Internal(anyhow!(
                "plugin state {} exceeds {} bytes",
                path.display(),
                MAX_PLUGIN_STATE_BYTES
            )));
        }
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|error| Error::Internal(anyhow!("plugin state is not UTF-8: {error}")))
    }

    pub fn save_if_changed(
        &self,
        plugin_id: &str,
        previous: Option<&str>,
        state: &str,
    ) -> Result<bool> {
        if previous == Some(state) {
            return Ok(false);
        }
        if state.len() > MAX_PLUGIN_STATE_BYTES {
            return Err(Error::Internal(anyhow!(
                "plugin state for {plugin_id} exceeds {MAX_PLUGIN_STATE_BYTES} bytes"
            )));
        }

        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("create plugin state dir {}", self.root.display()))?;
        set_user_only_dir(&self.root)?;

        let path = self.state_path(plugin_id);
        let temp = self.root.join(format!(
            ".{}.{}.tmp",
            crate::settings::sanitize_path_segment(plugin_id),
            uuid::Uuid::new_v4()
        ));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp)
            .with_context(|| format!("create plugin state temp {}", temp.display()))?;
        let write_result = (|| -> anyhow::Result<()> {
            file.write_all(state.as_bytes())?;
            file.sync_all()?;
            std::fs::rename(&temp, &path)?;
            Ok(())
        })();
        if let Err(error) = write_result {
            let _ = std::fs::remove_file(&temp);
            return Err(Error::Internal(
                error.context(format!("replace plugin state {}", path.display())),
            ));
        }
        Ok(true)
    }

    pub fn load_legacy(&self, file_name: &str) -> Result<Option<String>> {
        let path = safe_legacy_path(&self.legacy_root, file_name)?;
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(Error::Internal(anyhow!(
                    "read legacy plugin state {}: {error}",
                    path.display()
                )))
            }
        };
        if bytes.len() > MAX_PLUGIN_STATE_BYTES {
            return Err(Error::Internal(anyhow!(
                "legacy plugin state {} exceeds {} bytes",
                path.display(),
                MAX_PLUGIN_STATE_BYTES
            )));
        }
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|error| Error::Internal(anyhow!("legacy plugin state is not UTF-8: {error}")))
    }

    pub fn preserve_legacy(&self, file_name: &str) -> Result<()> {
        let path = safe_legacy_path(&self.legacy_root, file_name)?;
        let backup = path.with_extension("migrated.bak");
        if backup.exists() {
            set_user_only_file(&backup)?;
            if path.exists() {
                set_user_only_file(&path)?;
            }
            return Ok(());
        }
        if !path.exists() {
            return Ok(());
        }
        set_user_only_file(&path)?;
        std::fs::rename(&path, &backup).with_context(|| {
            format!(
                "preserve migrated plugin state {} as {}",
                path.display(),
                backup.display()
            )
        })?;
        Ok(())
    }
}

fn safe_legacy_path(root: &Path, file_name: &str) -> Result<PathBuf> {
    let path = Path::new(file_name);
    if path.components().count() != 1 || path.file_name().is_none() {
        return Err(Error::Internal(anyhow!(
            "legacy plugin state file must be a basename"
        )));
    }
    Ok(root.join(path))
}

#[cfg(unix)]
fn set_user_only_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("set plugin state permissions on {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_user_only_dir(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_user_only_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("set plugin state permissions on {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_user_only_file(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_is_atomic_user_only_and_skips_unchanged_writes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("state");
        let store = PluginStateStore::new(root.clone(), dir.path().to_path_buf());
        assert!(store.save_if_changed("org.example", None, "one").unwrap());
        assert!(!store
            .save_if_changed("org.example", Some("one"), "one")
            .unwrap());
        assert_eq!(store.load("org.example").unwrap().as_deref(), Some("one"));
        assert!(store
            .save_if_changed("org.example", Some("one"), "two")
            .unwrap());
        assert_eq!(store.load("org.example").unwrap().as_deref(), Some("two"));
        assert!(std::fs::read_dir(&root).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir_mode = std::fs::metadata(&root).unwrap().permissions().mode() & 0o777;
            let file_mode = std::fs::metadata(root.join("org.example.state"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(dir_mode, 0o700);
            assert_eq!(file_mode, 0o600);
        }
    }

    #[test]
    fn rejects_oversized_state_and_legacy_path_escape() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("state");
        let store = PluginStateStore::new(root.clone(), dir.path().to_path_buf());
        let oversized = "x".repeat(MAX_PLUGIN_STATE_BYTES + 1);
        assert!(store
            .save_if_changed("org.example", None, &oversized)
            .is_err());
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("org.example.state"), &oversized).unwrap();
        assert!(store.load("org.example").is_err());
        assert!(store.load_legacy("../secret").is_err());
    }

    #[test]
    fn preserves_migrated_legacy_blob() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("old.json");
        std::fs::write(&legacy, "legacy").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&legacy, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let store = PluginStateStore::new(dir.path().join("state"), dir.path().to_path_buf());
        assert_eq!(
            store.load_legacy("old.json").unwrap().as_deref(),
            Some("legacy")
        );
        store.preserve_legacy("old.json").unwrap();
        assert!(!dir.path().join("old.json").exists());
        let backup = dir.path().join("old.migrated.bak");
        assert!(backup.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&backup).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);

            std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o644)).unwrap();
            std::fs::write(&legacy, "recreated legacy").unwrap();
            std::fs::set_permissions(&legacy, std::fs::Permissions::from_mode(0o644)).unwrap();
            store.preserve_legacy("old.json").unwrap();
            for path in [&backup, &legacy] {
                let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o600);
            }
        }
    }
}
