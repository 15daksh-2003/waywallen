//! Steam Workshop directory fetcher backed by DepotDownloader.
//!
//! Downloading requires a Steam account that owns Wallpaper Engine; the login
//! is bootstrapped once interactively (`-remember-password`) and persists in the
//! daemon user's HOME, so the daemon must run as that same user.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::Result;
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::error::Error;
use crate::AppState;

const APPID: &str = "431960";
const FETCH_TIMEOUT: Duration = Duration::from_secs(30 * 60);

static FETCH_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Pin DepotDownloader's single-file extraction dir so its IsolatedStorage
/// (login token) path is stable. Apply to every DepotDownloader invocation.
pub(crate) fn pin_extract_dir(cmd: &mut Command) {
    cmd.env(
        "DOTNET_BUNDLE_EXTRACT_BASE_DIR",
        crate::settings::dd_extract_dir(),
    );
}

/// Find DepotDownloader's `AssemFiles` dir (holds `account.config`), or None.
fn find_assemfiles() -> Option<PathBuf> {
    let root = crate::settings::isolated_storage_root();
    let pattern = format!("{}/*/*/Url.*/AssemFiles", root.display());
    glob::glob(&pattern).ok()?.flatten().next()
}

/// Ensure DepotDownloader's IsolatedStorage exists and return AssemFiles. Its
/// startup `LoadFromFile` creates the store, so a brief run bootstraps it.
async fn ensure_isolated_storage(bin: &str) -> Result<PathBuf> {
    if let Some(af) = find_assemfiles() {
        return Ok(af);
    }
    let tmp = std::env::temp_dir().join("waywallen-dd-init");
    let _ = tokio::fs::create_dir_all(&tmp).await;
    let mut cmd = Command::new(bin);
    cmd.arg("-app")
        .arg(APPID)
        .arg("-pubfile")
        .arg("1")
        .arg("-dir")
        .arg(&tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    pin_extract_dir(&mut cmd);
    if let Ok(mut child) = cmd.spawn() {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    find_assemfiles()
        .ok_or_else(|| Error::WorkshopFetch("could not initialize DepotDownloader storage".into()))
        .map_err(Into::into)
}

/// Hand DepotDownloader a refresh token via its `account.config` so it downloads
/// without an interactive login.
pub async fn write_token(state: &Arc<AppState>, account: &str, refresh_token: &str) -> Result<()> {
    let bytes = crate::steam_auth::encode_account_config(account, refresh_token)?;
    let bin = resolve_bin(state);
    let _guard = FETCH_LOCK.lock().await;
    let assemfiles = ensure_isolated_storage(&bin).await?;
    tokio::fs::write(assemfiles.join("account.config"), bytes)
        .await
        .map_err(|e| Error::WorkshopFetch(format!("write account.config: {e}")))?;
    Ok(())
}

pub async fn clear_token() -> Result<()> {
    if let Some(af) = find_assemfiles() {
        let _ = tokio::fs::remove_file(af.join("account.config")).await;
    }
    Ok(())
}

pub fn has_token() -> bool {
    find_assemfiles()
        .map(|af| af.join("account.config").is_file())
        .unwrap_or(false)
}

pub(crate) fn resolve_bin(state: &AppState) -> String {
    if let Some(bin) = std::env::var_os("WAYWALLEN_DEPOTDOWNLOADER_BIN") {
        return bin.to_string_lossy().into_owned();
    }
    let configured = state.settings.snapshot().workshop.depotdownloader_bin;
    if configured.trim().is_empty() {
        "DepotDownloader".to_string()
    } else {
        configured
    }
}

pub async fn fetch(
    state: &Arc<AppState>,
    _source_id: &str,
    id: &str,
    dir: &Path,
) -> Result<PathBuf> {
    let bin = resolve_bin(state);
    let (username, refresh_token) = match crate::steam_session::load() {
        Some(s) if !s.account_name.trim().is_empty() => (s.account_name, Some(s.refresh_token)),
        _ => (state.settings.snapshot().workshop.steam_username, None),
    };
    if let Some(refresh_token) = refresh_token {
        if !has_token() {
            write_token(state, &username, &refresh_token).await?;
        }
    }
    run(&bin, &username, id, dir).await
}

async fn run(bin: &str, username: &str, id: &str, dir: &Path) -> Result<PathBuf> {
    if username.trim().is_empty() {
        return Err(Error::WorkshopFetch(
            "sign in to Steam from the Steam Workshop settings with an account that owns \
             Wallpaper Engine"
                .to_string(),
        )
        .into());
    }

    let _guard = FETCH_LOCK.lock().await;

    let staging = dir.join(format!(".staging-{id}"));
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging).await?;

    let mut cmd = Command::new(bin);
    cmd.arg("-app")
        .arg(APPID)
        .arg("-pubfile")
        .arg(id)
        .arg("-username")
        .arg(username)
        .arg("-remember-password")
        .arg("-dir")
        .arg(&staging)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    pin_extract_dir(&mut cmd);

    let child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::WorkshopFetch(format!(
                "DepotDownloader binary '{bin}' not found; install it (nixpkgs: \
                 depotdownloader) or set [workshop] depotdownloader_bin"
            ))
        } else {
            Error::WorkshopFetch(format!("failed to spawn DepotDownloader: {e}"))
        }
    })?;

    let output = match tokio::time::timeout(FETCH_TIMEOUT, child.wait_with_output()).await {
        Ok(res) => {
            res.map_err(|e| Error::WorkshopFetch(format!("DepotDownloader io error: {e}")))?
        }
        Err(_) => {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(Error::WorkshopFetch("DepotDownloader timed out".to_string()).into());
        }
    };

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    if !output.status.success() || needs_auth(&combined) {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(classify(&combined, bin, id, username).into());
    }

    let _ = tokio::fs::remove_dir_all(staging.join(".DepotDownloader")).await;
    let final_dir = dir.join(id);
    let _ = tokio::fs::remove_dir_all(&final_dir).await;
    tokio::fs::rename(&staging, &final_dir)
        .await
        .map_err(|e| Error::WorkshopFetch(format!("failed to finalize download: {e}")))?;
    Ok(final_dir)
}

/// Fetch the Wallpaper Engine shared assets tree into `dest` via DepotDownloader,
/// once. The assets ship in the WE Linux depots (`-os linux`), which we restrict
/// to `assets/` with a filelist.
pub async fn ensure_assets(state: &Arc<AppState>, dest: &Path) -> Result<()> {
    if dest.join("shaders").is_dir() {
        return Ok(());
    }
    let bin = resolve_bin(state);
    let username = crate::steam_session::load()
        .map(|s| s.account_name)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| state.settings.snapshot().workshop.steam_username);
    fetch_assets(&bin, &username, dest).await
}

async fn fetch_assets(bin: &str, username: &str, dest: &Path) -> Result<()> {
    if username.trim().is_empty() {
        return Err(Error::WorkshopFetch(
            "sign in to Steam from the Steam Workshop settings with an account that owns \
             Wallpaper Engine"
                .to_string(),
        )
        .into());
    }
    let parent = dest.parent().unwrap_or(dest);

    let _guard = FETCH_LOCK.lock().await;

    tokio::fs::create_dir_all(parent).await?;
    let staging = parent.join(".assets-staging");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging).await?;
    // Restrict the multi-depot download to just the shared assets subtree.
    let filelist = staging.join("filelist.txt");
    tokio::fs::write(&filelist, "regex:^assets[/\\\\].*\n").await?;

    let mut cmd = Command::new(bin);
    cmd.arg("-app")
        .arg(APPID)
        .arg("-os")
        .arg("linux")
        .arg("-username")
        .arg(username)
        .arg("-remember-password")
        .arg("-filelist")
        .arg(&filelist)
        .arg("-dir")
        .arg(&staging)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    pin_extract_dir(&mut cmd);

    let child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::WorkshopFetch(format!(
                "DepotDownloader binary '{bin}' not found; install it (nixpkgs: \
                 depotdownloader) or set [workshop] depotdownloader_bin"
            ))
        } else {
            Error::WorkshopFetch(format!("failed to spawn DepotDownloader: {e}"))
        }
    })?;

    let output = match tokio::time::timeout(FETCH_TIMEOUT, child.wait_with_output()).await {
        Ok(res) => {
            res.map_err(|e| Error::WorkshopFetch(format!("DepotDownloader io error: {e}")))?
        }
        Err(_) => {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(Error::WorkshopFetch("DepotDownloader timed out".to_string()).into());
        }
    };

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let produced = staging.join("assets");
    if !output.status.success() || needs_auth(&combined) || !produced.join("shaders").is_dir() {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(classify_assets(&combined, bin, username).into());
    }

    let _ = tokio::fs::remove_dir_all(dest).await;
    tokio::fs::rename(&produced, dest)
        .await
        .map_err(|e| Error::WorkshopFetch(format!("failed to finalize WE assets: {e}")))?;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    Ok(())
}

fn needs_auth(out: &str) -> bool {
    let l = out.to_lowercase();
    l.contains("steam guard")
        || l.contains("two-factor")
        || l.contains("please enter")
        || l.contains("enter your")
        || l.contains("password:")
        || l.contains("account password")
        || l.contains("logon requires")
}

fn classify(out: &str, bin: &str, id: &str, user: &str) -> Error {
    let l = out.to_lowercase();
    let _ = (bin, id, user);
    if needs_auth(out) || l.contains("invalidpassword") || l.contains("login failure") {
        return Error::WorkshopFetch(
            "Steam sign-in needed or expired. Open the Steam Workshop settings and \
             use Sign in to Steam, then try again."
                .to_string(),
        );
    }
    if l.contains("not available")
        || l.contains("no license")
        || l.contains("access is denied")
        || l.contains("does not have")
    {
        return Error::WorkshopFetch(
            "this Steam account does not own Wallpaper Engine".to_string(),
        );
    }
    Error::WorkshopFetch("DepotDownloader failed; see daemon logs".to_string())
}

fn classify_assets(out: &str, bin: &str, user: &str) -> Error {
    let l = out.to_lowercase();
    let _ = (bin, user);
    if needs_auth(out) || l.contains("invalidpassword") || l.contains("login failure") {
        return Error::WorkshopFetch(
            "Steam sign-in needed or expired to fetch Wallpaper Engine assets. Open \
             the Steam Workshop settings and use Sign in to Steam, then try again."
                .to_string(),
        );
    }
    if l.contains("not available")
        || l.contains("no license")
        || l.contains("access is denied")
        || l.contains("does not have")
    {
        return Error::WorkshopFetch(
            "this Steam account does not own Wallpaper Engine".to_string(),
        );
    }
    Error::WorkshopFetch("failed to fetch Wallpaper Engine assets; see daemon logs".to_string())
}

#[cfg(test)]
mod tests {
    use super::run;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn fake_dd(dir: &Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "#!/usr/bin/env bash\n{body}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn run_moves_downloaded_item_into_place() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = fake_dd(
            tmp.path(),
            "dd-ok.sh",
            r#"while [ $# -gt 0 ]; do [ "$1" = -dir ] && { shift; STAGING="$1"; }; shift; done
mkdir -p "$STAGING"
printf '{"type":"scene","title":"T"}' > "$STAGING/project.json"
printf pkg > "$STAGING/scene.pkg"
printf prev > "$STAGING/preview.jpg"
exit 0
"#,
        );
        let remote = tmp.path().join("remote");
        std::fs::create_dir_all(&remote).unwrap();
        let out = run(&bin, "user", "123", &remote).await.unwrap();
        assert_eq!(out, remote.join("123"));
        assert!(out.join("project.json").exists());
        assert!(out.join("scene.pkg").exists());
        assert!(!remote.join(".staging-123").exists());
    }

    #[tokio::test]
    async fn run_maps_auth_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = fake_dd(
            tmp.path(),
            "dd-auth.sh",
            "echo 'STEAM GUARD! Please enter code' >&2\nexit 1\n",
        );
        let err = run(&bin, "user", "123", tmp.path()).await.unwrap_err();
        assert!(err.to_string().contains("Steam sign-in needed"), "{err}");
    }

    #[tokio::test]
    async fn run_maps_missing_license() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = fake_dd(
            tmp.path(),
            "dd-lic.sh",
            "echo 'App 431960 not available from this account'\nexit 1\n",
        );
        let err = run(&bin, "user", "123", tmp.path()).await.unwrap_err();
        assert!(
            err.to_string().contains("does not own Wallpaper Engine"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn fetch_assets_publishes_the_shared_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = fake_dd(
            tmp.path(),
            "dd-assets.sh",
            r#"while [ $# -gt 0 ]; do [ "$1" = -dir ] && { shift; STAGING="$1"; }; shift; done
mkdir -p "$STAGING/assets/shaders"
printf frag > "$STAGING/assets/shaders/rough.frag"
exit 0
"#,
        );
        let dest = tmp.path().join("wallpaper_engine").join("assets");
        super::fetch_assets(&bin, "user", &dest).await.unwrap();
        assert!(dest.join("shaders").join("rough.frag").exists());
        assert!(!dest.parent().unwrap().join(".assets-staging").exists());
    }

    #[tokio::test]
    async fn fetch_assets_fails_when_tree_missing() {
        let tmp = tempfile::tempdir().unwrap();
        // Exits 0 but produces nothing under assets/ (e.g. a license/filter miss).
        let bin = fake_dd(tmp.path(), "dd-empty.sh", "exit 0\n");
        let dest = tmp.path().join("wallpaper_engine").join("assets");
        let err = super::fetch_assets(&bin, "user", &dest).await.unwrap_err();
        assert!(!dest.exists());
        assert!(err.to_string().contains("Wallpaper Engine assets"), "{err}");
    }

    #[tokio::test]
    async fn run_requires_username() {
        let tmp = tempfile::tempdir().unwrap();
        let err = run("DepotDownloader", "  ", "123", tmp.path())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("sign in to Steam"), "{err}");
    }
}
