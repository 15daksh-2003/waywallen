use anyhow::anyhow;

use crate::error::{Error, Result};
use mlua::prelude::*;
use sea_orm::DatabaseConnection;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use crate::model::repo;
use crate::probe::media::{AvFormatProbe, MediaProbe};
use crate::wallpaper::types::{WallpaperEntry, WallpaperType};

/// User-Agent the `ctx.http` default client sends.
const WAYWALLEN_HTTP_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) waywallen";

/// Lua entry ABI supported by this daemon. Plugin manifests must declare
/// the same value in `[plugin].entry_version`.
pub const ENTRY_VERSION: u32 = 2;

fn resolve_plugin_import(root: &Path, name: &str) -> LuaResult<PathBuf> {
    let mut rel = PathBuf::new();
    for part in name.split('.') {
        if part.is_empty()
            || part == ".."
            || part.contains('/')
            || part.contains('\\')
            || part == "."
        {
            return Err(LuaError::RuntimeError(format!(
                "invalid import module name: {name}"
            )));
        }
        rel.push(part);
    }

    let candidates = [
        root.join(&rel).with_extension("lua"),
        root.join(&rel).join("init.lua"),
    ];
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        let path = candidate.canonicalize().map_err(LuaError::external)?;
        if path.starts_with(root) {
            return Ok(path);
        }
    }

    Err(LuaError::RuntimeError(format!("module not found: {name}")))
}

// ---------------------------------------------------------------------------
// Public types

#[derive(Debug, Clone, serde::Serialize)]
pub struct SourcePluginInfo {
    pub name: String,
    /// Domain id of the owning installable plugin.
    /// Empty when loaded without package metadata.
    pub plugin_id: String,
    pub types: Vec<WallpaperType>,
    pub version: String,
    /// Short UI label or placeholder for prompting a library path.
    /// Empty when the plugin did not declare one.
    pub library_label: String,
    /// Longer helper text for choosing a library path.
    /// May contain newlines or inline-code Markdown markers.
    pub library_hint: String,
}

/// One sort option a discover-capable plugin advertises via
/// `info().capabilities.discover.sorts`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoverSort {
    pub key: String,
    pub label: String,
}

/// Discover capability of a single source plugin, derived from
/// `info().capabilities.discover`. Plugins without that table are not listed.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoverSourceInfo {
    /// Discover entry name — the routing key clients echo back in
    /// `DiscoverSearchRequest.plugin_id`.
    pub plugin_id: String,
    pub name: String,
    pub supports_search: bool,
    pub sorts: Vec<DiscoverSort>,
    pub tags: Vec<String>,
}

/// One remote item returned by a plugin's `discover.search(ctx, params)`.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DiscoverItem {
    pub id: String,
    pub title: String,
    pub preview_url: String,
    pub author: String,
    pub wp_type: String,
    pub extra: HashMap<String, String>,
}

/// Detail blob returned by a plugin's `discover.details(ctx, id)`.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DiscoverDetails {
    pub description: String,
    pub size: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub tags: Vec<String>,
    pub extra: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DiscoverSearchResult {
    pub items: Vec<DiscoverItem>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DiscoverDownload {
    pub wp_type: String,
    pub url: String,
    pub filename: String,
    pub title: String,
    pub preview_url: String,
    pub description: String,
    pub tags: Vec<String>,
    pub external_id: String,
    pub size: Option<i64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub content_rating: Option<String>,
}

#[derive(Debug, Clone)]
struct SourceCapability {
    types: Vec<WallpaperType>,
    library_label: String,
    library_hint: String,
    auto_detect: bool,
}

#[derive(Debug, Clone)]
struct DiscoverCapability {
    supports_search: bool,
    supports_details: bool,
    supports_download: bool,
    sorts: Vec<DiscoverSort>,
    tags: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct WallpaperCapability {
    extras: bool,
    properties: bool,
}

#[derive(Debug, Clone, Default)]
struct PluginCapabilities {
    source: Option<SourceCapability>,
    source_item_remove: bool,
    discover: Option<DiscoverCapability>,
    wallpaper: WallpaperCapability,
}

#[derive(Debug, Clone)]
struct LoadedPluginInfo {
    name: String,
    plugin_id: String,
    version: String,
    capabilities: PluginCapabilities,
}

// ---------------------------------------------------------------------------
// SourceManager

pub struct SourceManager {
    lua: Lua,
    /// plugin name → registry key for the loaded module table.
    plugins: HashMap<String, LuaRegistryKey>,
    /// source `info().name` → parsed ABI v2 metadata.
    plugin_infos: HashMap<String, LoadedPluginInfo>,
    /// Flattened scan results from all plugins.
    entries: Vec<WallpaperEntry>,
    /// Index: wp_type → indices into `entries`.
    by_type: HashMap<WallpaperType, Vec<usize>>,
    /// Shared media probe exposed to Lua via ctx.probe(path).
    probe: Arc<dyn MediaProbe>,
    /// DB used by the `ctx.library_meta_*` async-Lua-function bridge.
    /// `None` makes the bridge no-op, which is useful for DB-less tests.
    db: Option<DatabaseConnection>,
}

// mlua with the `send` feature makes Lua: Send.
// We wrap SourceManager in Arc<TokioMutex<>> so this is required.
fn assert_source_manager_send() {
    fn assert_send<T: Send>() {}
    assert_send::<SourceManager>();
}
const _: fn() = assert_source_manager_send;

impl SourceManager {
    pub fn new() -> Result<Self> {
        Self::with_probe(Arc::new(AvFormatProbe::new()))
    }

    pub fn with_probe(probe: Arc<dyn MediaProbe>) -> Result<Self> {
        let lua = Lua::new();
        Ok(Self {
            lua,
            plugins: HashMap::new(),
            plugin_infos: HashMap::new(),
            entries: Vec::new(),
            by_type: HashMap::new(),
            probe,
            db: None,
        })
    }

    /// Hand the DB to the source manager so `ctx.library_meta_get/set`
    /// can read and write per-library metadata.
    pub fn attach_db(&mut self, db: DatabaseConnection) {
        self.db = Some(db);
    }

    fn plugin_lua_env(&self, root: &Path) -> Result<LuaTable> {
        let root = root
            .canonicalize()
            .map_err(|e| Error::Internal(anyhow!("canonicalize {}: {e}", root.display())))?;
        let root = Arc::new(root);
        let cache: Arc<StdMutex<HashMap<PathBuf, LuaRegistryKey>>> =
            Arc::new(StdMutex::new(HashMap::new()));

        let env = self.lua.create_table()?;
        let mt = self.lua.create_table()?;
        mt.set("__index", self.lua.globals())?;
        env.set_metatable(Some(mt));

        let import_env = env.clone();
        let import_root = root.clone();
        let import_cache = cache.clone();
        let import_fn = self.lua.create_function(move |lua, name: String| {
            let path = resolve_plugin_import(&import_root, &name)?;
            {
                let cache = import_cache
                    .lock()
                    .map_err(|_| LuaError::RuntimeError("import cache poisoned".to_string()))?;
                if let Some(key) = cache.get(&path) {
                    return lua.registry_value::<LuaValue>(key);
                }
            }

            let source = std::fs::read_to_string(&path).map_err(LuaError::external)?;
            let value: LuaValue = lua
                .load(&source)
                .set_name(path.to_string_lossy())
                .set_environment(import_env.clone())
                .eval()?;
            let key = lua.create_registry_value(value)?;
            let mut cache = import_cache
                .lock()
                .map_err(|_| LuaError::RuntimeError("import cache poisoned".to_string()))?;
            cache.insert(path.clone(), key);
            lua.registry_value::<LuaValue>(cache.get(&path).expect("cached import"))
        })?;
        env.set("import", import_fn)?;
        Ok(env)
    }

    fn require_string(tbl: &LuaTable, key: &str, context: &str) -> Result<String> {
        tbl.get::<String>(key)
            .map_err(|e| Error::Internal(anyhow!("{context}.{key} required: {e}")))
    }

    fn optional_string(tbl: &LuaTable, key: &str, context: &str) -> Result<String> {
        match tbl
            .get::<LuaValue>(key)
            .map_err(|e| Error::Internal(anyhow!("{context}.{key}: {e}")))?
        {
            LuaValue::Nil => Ok(String::new()),
            LuaValue::String(s) => s
                .to_str()
                .map(|cow| cow.to_string())
                .map_err(|e| Error::Internal(anyhow!("{context}.{key} invalid string: {e}"))),
            other => Err(Error::Internal(anyhow!(
                "{context}.{key} must be a string, got {}",
                other.type_name()
            ))),
        }
    }

    fn require_string_sequence(tbl: &LuaTable, key: &str, context: &str) -> Result<Vec<String>> {
        let values: LuaTable = tbl
            .get(key)
            .map_err(|e| Error::Internal(anyhow!("{context}.{key} required: {e}")))?;
        let mut out = Vec::new();
        for (idx, value) in values.sequence_values::<String>().enumerate() {
            out.push(value.map_err(|e| {
                Error::Internal(anyhow!(
                    "{context}.{key}[{}] must be a string: {e}",
                    idx + 1
                ))
            })?);
        }
        Ok(out)
    }

    fn optional_string_sequence(tbl: &LuaTable, key: &str, context: &str) -> Result<Vec<String>> {
        let Some(values) = Self::optional_table(tbl, key, context)? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for (idx, value) in values.sequence_values::<String>().enumerate() {
            out.push(value.map_err(|e| {
                Error::Internal(anyhow!(
                    "{context}.{key}[{}] must be a string: {e}",
                    idx + 1
                ))
            })?);
        }
        Ok(out)
    }

    fn optional_discover_sorts(discover_tbl: &LuaTable) -> Result<Vec<DiscoverSort>> {
        let Some(sorts_tbl) =
            Self::optional_table(discover_tbl, "sorts", "info().capabilities.discover")?
        else {
            return Ok(Vec::new());
        };
        let mut sorts = Vec::new();
        for (idx, sort) in sorts_tbl.sequence_values::<LuaTable>().enumerate() {
            let sort = sort.map_err(|e| {
                Error::Internal(anyhow!(
                    "info().capabilities.discover.sorts[{}] must be a table: {e}",
                    idx + 1
                ))
            })?;
            let context = format!("info().capabilities.discover.sorts[{}]", idx + 1);
            let key = Self::require_string(&sort, "key", &context)?;
            let label = Self::require_string(&sort, "label", &context)?;
            if key.is_empty() || label.is_empty() {
                return Err(Error::Internal(anyhow!(
                    "{context}.key and {context}.label must not be empty"
                )));
            }
            sorts.push(DiscoverSort { key, label });
        }
        Ok(sorts)
    }

    fn require_table_function(tbl: &LuaTable, fn_name: &str, context: &str) -> Result<()> {
        tbl.get::<LuaFunction>(fn_name)
            .map(|_| ())
            .map_err(|e| Error::Internal(anyhow!("{context}.{fn_name} required: {e}")))
    }

    fn require_module_table(module: &LuaTable, name: &str) -> Result<LuaTable> {
        module
            .get::<LuaTable>(name)
            .map_err(|e| Error::Internal(anyhow!("module.{name} table required: {e}")))
    }

    fn optional_table(tbl: &LuaTable, key: &str, context: &str) -> Result<Option<LuaTable>> {
        match tbl
            .get::<LuaValue>(key)
            .map_err(|e| Error::Internal(anyhow!("{context}.{key}: {e}")))?
        {
            LuaValue::Nil => Ok(None),
            LuaValue::Table(t) => Ok(Some(t)),
            other => Err(Error::Internal(anyhow!(
                "{context}.{key} must be a table, got {}",
                other.type_name()
            ))),
        }
    }

    fn optional_bool(tbl: &LuaTable, key: &str, context: &str, default: bool) -> Result<bool> {
        match tbl
            .get::<LuaValue>(key)
            .map_err(|e| Error::Internal(anyhow!("{context}.{key}: {e}")))?
        {
            LuaValue::Nil => Ok(default),
            LuaValue::Boolean(v) => Ok(v),
            other => Err(Error::Internal(anyhow!(
                "{context}.{key} must be a boolean, got {}",
                other.type_name()
            ))),
        }
    }

    fn parse_plugin_info(
        module: &LuaTable,
        plugin_id: &str,
        plugin_version: &str,
    ) -> Result<LoadedPluginInfo> {
        let info_fn: LuaFunction = module
            .get("info")
            .map_err(|e| Error::Internal(anyhow!("plugin must export info(): {e}")))?;
        let info_table: LuaTable = info_fn
            .call(())
            .map_err(|e| Error::Internal(anyhow!("info() failed: {e}")))?;
        let name: String = info_table
            .get("name")
            .map_err(|e| Error::Internal(anyhow!("info().name required: {e}")))?;
        let caps_tbl: LuaTable = info_table
            .get("capabilities")
            .map_err(|e| Error::Internal(anyhow!("info().capabilities required: {e}")))?;
        let source_api = Self::optional_table(module, "source", "module")?;
        let source_item_remove = match &source_api {
            Some(source_api) => match source_api.get::<LuaValue>("remove")? {
                LuaValue::Nil => false,
                LuaValue::Function(_) => true,
                _ => {
                    return Err(Error::Internal(anyhow!(
                        "module.source.remove must be a function when present"
                    )));
                }
            },
            None => false,
        };

        let source = match Self::optional_table(&caps_tbl, "source", "info().capabilities")? {
            Some(source_tbl) => {
                if !Self::optional_bool(&source_tbl, "scan", "info().capabilities.source", false)? {
                    return Err(Error::Internal(anyhow!(
                        "info().capabilities.source.scan must be true"
                    )));
                }
                let source_api = source_api.clone().ok_or_else(|| {
                    Error::Internal(anyhow!(
                        "module.source table required for source capability"
                    ))
                })?;
                Self::require_table_function(&source_api, "scan", "module.source")?;
                let auto_detect = Self::optional_bool(
                    &source_tbl,
                    "auto_detect",
                    "info().capabilities.source",
                    false,
                )?;
                if auto_detect {
                    Self::require_table_function(&source_api, "auto_detect", "module.source")?;
                }
                let types = Self::require_string_sequence(
                    &source_tbl,
                    "types",
                    "info().capabilities.source",
                )?;
                if types.is_empty() {
                    return Err(Error::Internal(anyhow!(
                        "info().capabilities.source.types must not be empty"
                    )));
                }
                Some(SourceCapability {
                    types,
                    library_label: Self::optional_string(
                        &source_tbl,
                        "library_label",
                        "info().capabilities.source",
                    )?,
                    library_hint: Self::optional_string(
                        &source_tbl,
                        "library_hint",
                        "info().capabilities.source",
                    )?,
                    auto_detect,
                })
            }
            None => None,
        };

        let discover = match Self::optional_table(&caps_tbl, "discover", "info().capabilities")? {
            Some(discover_tbl) => {
                if !Self::optional_bool(
                    &discover_tbl,
                    "search",
                    "info().capabilities.discover",
                    false,
                )? {
                    return Err(Error::Internal(anyhow!(
                        "info().capabilities.discover.search must be true"
                    )));
                }
                let discover_api = Self::require_module_table(module, "discover")?;
                Self::require_table_function(&discover_api, "search", "module.discover")?;
                let supports_details = Self::optional_bool(
                    &discover_tbl,
                    "details",
                    "info().capabilities.discover",
                    false,
                )?;
                let supports_download = Self::optional_bool(
                    &discover_tbl,
                    "download",
                    "info().capabilities.discover",
                    false,
                )?;
                if supports_details {
                    Self::require_table_function(&discover_api, "details", "module.discover")?;
                }
                if supports_download {
                    Self::require_table_function(&discover_api, "download", "module.discover")?;
                }
                let sorts = Self::optional_discover_sorts(&discover_tbl)?;
                let tags = Self::optional_string_sequence(
                    &discover_tbl,
                    "tags",
                    "info().capabilities.discover",
                )?;
                Some(DiscoverCapability {
                    supports_search: true,
                    supports_details,
                    supports_download,
                    sorts,
                    tags,
                })
            }
            None => None,
        };

        let mut wallpaper = WallpaperCapability::default();
        if let Some(wallpaper_tbl) =
            Self::optional_table(&caps_tbl, "wallpaper", "info().capabilities")?
        {
            let wallpaper_api = Self::require_module_table(module, "wallpaper")?;
            wallpaper.extras = Self::optional_bool(
                &wallpaper_tbl,
                "extras",
                "info().capabilities.wallpaper",
                false,
            )?;
            wallpaper.properties = Self::optional_bool(
                &wallpaper_tbl,
                "properties",
                "info().capabilities.wallpaper",
                false,
            )?;
            if wallpaper.extras {
                Self::require_table_function(&wallpaper_api, "extras", "module.wallpaper")?;
            }
            if wallpaper.properties {
                Self::require_table_function(&wallpaper_api, "properties", "module.wallpaper")?;
            }
        }

        Ok(LoadedPluginInfo {
            name,
            plugin_id: plugin_id.to_owned(),
            version: plugin_version.to_owned(),
            capabilities: PluginCapabilities {
                source,
                source_item_remove,
                discover,
                wallpaper,
            },
        })
    }

    /// Load a single Lua entry, tagging it with the owning installable
    /// plugin's domain id. Returns the Lua plugin name.
    pub fn load_plugin(
        &mut self,
        path: &Path,
        plugin_id: &str,
        plugin_version: &str,
        entry_version: u32,
    ) -> Result<String> {
        if entry_version != ENTRY_VERSION {
            return Err(Error::Internal(anyhow!(
                "unsupported Lua entry_version {entry_version}; expected {ENTRY_VERSION}"
            )));
        }
        let source = std::fs::read_to_string(path)
            .map_err(|e| Error::Internal(anyhow!("read {}: {e}", path.display())))?;
        let root = path.parent().unwrap_or_else(|| Path::new("."));
        let env = self.plugin_lua_env(root)?;
        let module: LuaTable = self
            .lua
            .load(&source)
            .set_name(path.to_string_lossy())
            .set_environment(env)
            .eval()
            .map_err(|e| Error::Internal(anyhow!("eval {}: {e}", path.display())))?;

        let info = Self::parse_plugin_info(&module, plugin_id, plugin_version)?;
        let name = info.name.clone();

        let key = self.lua.create_registry_value(module)?;
        self.plugins.insert(name.clone(), key);
        self.plugin_infos.insert(name.clone(), info);
        log::info!(
            "loaded source plugin: {name} (plugin {plugin_id}) from {}",
            path.display()
        );
        Ok(name)
    }

    /// Run `scan(ctx)` on all loaded plugins and merge results.
    /// `libs_by_plugin` is the per-plugin library list from the DB.
    pub async fn scan_all(&mut self, libs_by_plugin: &HashMap<String, Vec<String>>) -> Result<()> {
        self.entries.clear();
        self.by_type.clear();

        let mut plugin_names: Vec<String> = self
            .plugin_infos
            .iter()
            .filter_map(|(name, info)| info.capabilities.source.is_some().then(|| name.clone()))
            .collect();
        plugin_names.sort();
        for name in &plugin_names {
            let libs = libs_by_plugin
                .get(name)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            if let Err(e) = self.scan_plugin(name, libs).await {
                log::warn!("scan plugin {name} failed: {e}");
            }
        }
        Ok(())
    }

    /// Run `scan(ctx)` on a single plugin by name with the supplied
    /// library list exposed as `ctx.libraries()`.
    async fn scan_plugin(&mut self, name: &str, libraries: &[String]) -> Result<()> {
        let key = self
            .plugins
            .get(name)
            .ok_or_else(|| Error::SourcePluginNotFound(name.to_string()))?;
        let module: LuaTable = self.lua.registry_value(key)?;
        let info = self
            .plugin_infos
            .get(name)
            .ok_or_else(|| Error::SourcePluginNotFound(name.to_string()))?;
        if info.capabilities.source.is_none() {
            return Ok(());
        }
        let source_api: LuaTable = module.get("source")?;
        let scan_fn: LuaFunction = source_api.get("scan")?;

        let ctx = self.build_ctx(Some(name), libraries)?;
        let results: LuaTable = scan_fn.call_async(ctx).await?;

        for pair in results.sequence_values::<LuaTable>() {
            let tbl = pair?;
            let entry_name = Self::require_string(&tbl, "name", "module.source.scan result")?;
            let wp_type = Self::require_string(&tbl, "wp_type", "module.source.scan result")?;
            let resource = Self::require_string(&tbl, "resource", "module.source.scan result")?;
            let entry = WallpaperEntry {
                // Identity comes from the DB item.id, assigned after
                // sync; plugins don't supply it.
                item_id: 0,
                name: entry_name,
                wp_type,
                resource,
                preview: tbl.get::<String>("preview").ok(),
                plugin_name: name.to_owned(),
                library_root: tbl.get("library_root").unwrap_or_default(),
                description: tbl.get::<String>("description").ok(),
                tags: tbl.get::<Vec<String>>("tags").unwrap_or_default(),
                external_id: tbl.get::<String>("external_id").ok(),
                // Optional plugin-supplied media metadata.
                // Plugins that know it can skip later probing.
                size: tbl.get::<i64>("size").ok(),
                width: tbl.get::<u32>("width").ok(),
                height: tbl.get::<u32>("height").ok(),
                content_rating: tbl.get::<String>("content_rating").ok(),
                // Daemon-only (filled from DB on read); scan leaves it None.
                modified_at: None,
                create_at: 0,
            };
            let idx = self.entries.len();
            self.by_type
                .entry(entry.wp_type.clone())
                .or_default()
                .push(idx);
            self.entries.push(entry);
        }
        Ok(())
    }

    /// Build the `ctx` table passed to Lua callbacks.
    /// `libraries` is exposed through `ctx.libraries()`.
    fn build_ctx(&self, plugin_name: Option<&str>, libraries: &[String]) -> Result<LuaTable> {
        let ctx = self.lua.create_table()?;

        // ctx.glob(pattern) -> list of file paths
        let glob_fn = self.lua.create_function(|lua, pattern: String| {
            let paths = lua.create_table()?;
            let mut i = 1;
            if let Ok(entries) = glob::glob(&pattern) {
                for entry in entries.flatten() {
                    if let Some(s) = entry.to_str() {
                        paths.set(i, s.to_string())?;
                        i += 1;
                    }
                }
            }
            Ok(paths)
        })?;
        ctx.set("glob", glob_fn)?;

        // ctx.list_dirs(path) -> list of subdirectory paths
        let list_dirs_fn = self.lua.create_function(|lua, path: String| {
            let dirs = lua.create_table()?;
            let mut i = 1;
            if let Ok(entries) = std::fs::read_dir(&path) {
                for entry in entries.flatten() {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        if let Some(s) = entry.path().to_str() {
                            dirs.set(i, s.to_string())?;
                            i += 1;
                        }
                    }
                }
            }
            Ok(dirs)
        })?;
        ctx.set("list_dirs", list_dirs_fn)?;

        // ctx.file_exists(path) -> bool
        let file_exists_fn = self
            .lua
            .create_function(|_, path: String| Ok(std::path::Path::new(&path).exists()))?;
        ctx.set("file_exists", file_exists_fn)?;

        // ctx.read_file(path) -> string|nil (capped at 1MB)
        let read_file_fn =
            self.lua
                .create_function(|lua, path: String| match std::fs::metadata(&path) {
                    Ok(meta) if meta.len() > 1_048_576 => Ok(mlua::Value::Nil),
                    Ok(_) => match std::fs::read_to_string(&path) {
                        Ok(s) => Ok(mlua::Value::String(lua.create_string(&s)?)),
                        Err(_) => Ok(mlua::Value::Nil),
                    },
                    Err(_) => Ok(mlua::Value::Nil),
                })?;
        ctx.set("read_file", read_file_fn)?;

        // ctx.extension(path) -> string|nil
        let extension_fn = self.lua.create_function(|_, path: String| {
            Ok(std::path::Path::new(&path)
                .extension()
                .and_then(|e| e.to_str())
                .map(String::from))
        })?;
        ctx.set("extension", extension_fn)?;

        // ctx.filename(path) -> string|nil
        let filename_fn = self.lua.create_function(|_, path: String| {
            Ok(std::path::Path::new(&path)
                .file_name()
                .and_then(|e| e.to_str())
                .map(String::from))
        })?;
        ctx.set("filename", filename_fn.clone())?;

        // ctx.basename(path) -> string|nil (same as filename on dirs)
        ctx.set("basename", filename_fn)?;

        // ctx.env(name) -> string|nil. Used for auto-detect probing of
        // well-known paths such as $HOME.
        let env_fn = self
            .lua
            .create_function(|_, name: String| Ok(std::env::var(&name).ok()))?;
        ctx.set("env", env_fn)?;

        // ctx.libraries() -> list of absolute library paths registered
        // for this plugin in the daemon DB.
        let libs_for_closure: Vec<String> = libraries.to_vec();
        let libraries_fn = self.lua.create_function(move |lua, ()| {
            let tbl = lua.create_table()?;
            for (i, lib) in libs_for_closure.iter().enumerate() {
                tbl.set(i + 1, lib.clone())?;
            }
            Ok(tbl)
        })?;
        ctx.set("libraries", libraries_fn)?;

        // ctx.json_parse(str) -> table|nil
        let json_parse_fn =
            self.lua.create_function(|lua, s: String| {
                match serde_json::from_str::<serde_json::Value>(&s) {
                    Ok(val) => json_to_lua(lua, &val),
                    Err(_) => Ok(mlua::Value::Nil),
                }
            })?;
        ctx.set("json_parse", json_parse_fn)?;

        // ctx.json_encode(value) -> string|nil
        let json_encode_fn = self.lua.create_function(|_, val: mlua::Value| {
            Ok(lua_to_json(&val).and_then(|j| serde_json::to_string(&j).ok()))
        })?;
        ctx.set("json_encode", json_encode_fn)?;

        // ctx.log(msg)
        let log_fn = self.lua.create_function(|_, msg: String| {
            log::info!("[lua] {msg}");
            Ok(())
        })?;
        ctx.set("log", log_fn)?;

        // ctx.file_size(path) -> integer|nil
        // Cheap stat-only helper for Lua plugins to pre-fill size metadata.
        let file_size_fn = self.lua.create_function(|_, path: String| {
            let bytes = std::fs::metadata(&path)
                .ok()
                .and_then(|m| i64::try_from(m.len()).ok());
            Ok(bytes)
        })?;
        ctx.set("file_size", file_size_fn)?;

        // ctx.remove_file(path) -> bool
        let remove_file_fn =
            self.lua
                .create_function(|_, path: String| match std::fs::remove_file(&path) {
                    Ok(()) => Ok(true),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                    Err(e) => Err(mlua::Error::external(e)),
                })?;
        ctx.set("remove_file", remove_file_fn)?;

        // ctx.probe(path) -> table|nil
        // Returns present file/media fields, or nil if nothing was found.
        let probe_arc = Arc::clone(&self.probe);
        let probe_fn = self.lua.create_function(move |lua, path: String| {
            let s = crate::probe::stat::stat_file(&path);
            let m = probe_arc.probe_media(&path);
            if s.is_none() && m.width.is_none() && m.height.is_none() {
                return Ok(mlua::Value::Nil);
            }
            let tbl = lua.create_table()?;
            if let Some(s) = s {
                tbl.set("size", s.size)?;
            }
            if let Some(v) = m.width {
                tbl.set("width", v)?;
            }
            if let Some(v) = m.height {
                tbl.set("height", v)?;
            }
            Ok(mlua::Value::Table(tbl))
        })?;
        ctx.set("probe", probe_fn)?;

        // ctx.library_meta_get(library_path, key) -> string|nil
        // ctx.library_meta_set(library_path, key, value_or_nil) -> bool
        {
            let kv_db = self.db.clone();
            let kv_plugin = plugin_name.map(str::to_owned);

            let getter_db = kv_db.clone();
            let getter_plugin = kv_plugin.clone();
            let library_meta_get_fn =
                self.lua
                    .create_async_function(move |lua, (lib_path, key): (String, String)| {
                        let db = getter_db.clone();
                        let plugin_name = getter_plugin.clone();
                        async move {
                            let (Some(db), Some(plugin_name)) = (db, plugin_name) else {
                                return Ok(mlua::Value::Nil);
                            };
                            let res: crate::error::Result<Option<String>> = async {
                                let Some(plugin) =
                                    repo::find_plugin_by_name(&db, &plugin_name).await?
                                else {
                                    return Ok(None);
                                };
                                let Some(lib) =
                                    repo::find_library(&db, plugin.id, &lib_path).await?
                                else {
                                    return Ok(None);
                                };
                                repo::get_library_metadata_value(&db, lib.id, &key).await
                            }
                            .await;
                            match res {
                                Ok(Some(v)) => Ok(mlua::Value::String(lua.create_string(&v)?)),
                                Ok(None) => Ok(mlua::Value::Nil),
                                Err(e) => {
                                    log::warn!("library_meta_get: {e:#}");
                                    Ok(mlua::Value::Nil)
                                }
                            }
                        }
                    })?;
            ctx.set("library_meta_get", library_meta_get_fn)?;

            let setter_db = kv_db;
            let setter_plugin = kv_plugin;
            let library_meta_set_fn = self.lua.create_async_function(
                move |_, (lib_path, key, value): (String, String, Option<String>)| {
                    let db = setter_db.clone();
                    let plugin_name = setter_plugin.clone();
                    async move {
                        let (Some(db), Some(plugin_name)) = (db, plugin_name) else {
                            return Ok(false);
                        };
                        let res: crate::error::Result<bool> = async {
                            let Some(plugin) = repo::find_plugin_by_name(&db, &plugin_name).await?
                            else {
                                return Ok(false);
                            };
                            let Some(lib) = repo::find_library(&db, plugin.id, &lib_path).await?
                            else {
                                return Ok(false);
                            };
                            repo::set_library_metadata_value(&db, lib.id, &key, value.as_deref())
                                .await?;
                            Ok(true)
                        }
                        .await;
                        match res {
                            Ok(b) => Ok(b),
                            Err(e) => {
                                log::warn!("library_meta_set: {e:#}");
                                Ok(false)
                            }
                        }
                    }
                },
            )?;
            ctx.set("library_meta_set", library_meta_set_fn)?;
        }

        // Source plugins write entry fields directly using the canonical
        // schema exposed by WallpaperEntry.

        // ctx.http is a fluent client:
        // ctx.http:get(url):headers({...}):send()
        ctx.set("http", mlua_extra::http::default(WAYWALLEN_HTTP_USER_AGENT))?;
        ctx.set("html", mlua_extra::html::create_module(&self.lua)?)?;
        ctx.set("url", mlua_extra::url::create_module(&self.lua)?)?;

        Ok(ctx)
    }

    fn wallpaper_entry_table(&self, entry: &WallpaperEntry) -> Result<LuaTable> {
        let entry_tbl = self.lua.create_table()?;
        entry_tbl.set("id", entry.item_id.to_string())?;
        entry_tbl.set("item_id", entry.item_id)?;
        entry_tbl.set("name", entry.name.clone())?;
        entry_tbl.set("wp_type", entry.wp_type.clone())?;
        entry_tbl.set("path", entry.resource.clone())?;
        entry_tbl.set("resource", entry.resource.clone())?;
        if let Some(p) = &entry.preview {
            entry_tbl.set("preview", p.clone())?;
        }
        if !entry.library_root.is_empty() {
            entry_tbl.set("library_root", entry.library_root.clone())?;
            if let Some(rel) =
                crate::model::sync::relative_under_root(&entry.library_root, &entry.resource)
            {
                entry_tbl.set("relative_path", rel)?;
            }
        }
        if let Some(d) = &entry.description {
            entry_tbl.set("description", d.clone())?;
        }
        if !entry.tags.is_empty() {
            let tags = self.lua.create_table()?;
            for (idx, tag) in entry.tags.iter().enumerate() {
                tags.set(idx + 1, tag.clone())?;
            }
            entry_tbl.set("tags", tags)?;
        }
        if let Some(eid) = &entry.external_id {
            entry_tbl.set("external_id", eid.clone())?;
        }
        if let Some(size) = entry.size {
            entry_tbl.set("size", size)?;
        }
        if let Some(width) = entry.width {
            entry_tbl.set("width", width)?;
        }
        if let Some(height) = entry.height {
            entry_tbl.set("height", height)?;
        }
        if let Some(content_rating) = &entry.content_rating {
            entry_tbl.set("content_rating", content_rating.clone())?;
        }
        Ok(entry_tbl)
    }

    // -----------------------------------------------------------------------
    // Query API

    pub fn list(&self) -> &[WallpaperEntry] {
        &self.entries
    }

    pub fn list_by_type(&self, wp_type: &str) -> Vec<&WallpaperEntry> {
        self.by_type
            .get(wp_type)
            .map(|indices| indices.iter().map(|&i| &self.entries[i]).collect())
            .unwrap_or_default()
    }

    pub fn get(&self, id: &str) -> Option<&WallpaperEntry> {
        self.entries.iter().find(|e| e.item_id.to_string() == id)
    }

    /// Ask the plugin that produced `entry` for the CLI `extras`
    /// dictionary the daemon should pass to the renderer subprocess
    pub async fn call_extras(
        &self,
        plugin_name: &str,
        entry: &WallpaperEntry,
    ) -> Result<HashMap<String, String>> {
        let key = self
            .plugins
            .get(plugin_name)
            .ok_or_else(|| Error::SourcePluginNotFound(plugin_name.to_string()))?;
        let Some(info) = self.plugin_infos.get(plugin_name) else {
            return Err(Error::SourcePluginNotFound(plugin_name.to_string()));
        };
        if !info.capabilities.wallpaper.extras {
            log::warn!("source plugin '{plugin_name}' has no wallpaper.extras capability");
            return Ok(HashMap::new());
        }
        // Keep the Lua body in one block so failures map to one typed
        // SourceExtrasFailed carrying the plugin name.
        let body = async {
            let module: LuaTable = self.lua.registry_value(key)?;
            let wallpaper_api: LuaTable = module.get("wallpaper")?;
            let extras_fn: LuaFunction = wallpaper_api.get("extras")?;
            let entry_tbl = self
                .wallpaper_entry_table(entry)
                .map_err(mlua::Error::external)?;
            // Build the same ctx scan(ctx) sees; extras runs per item, so
            // the libraries list is intentionally empty.
            let ctx = self
                .build_ctx(Some(plugin_name), &[])
                .map_err(mlua::Error::external)?;
            let result: LuaTable = extras_fn.call_async((entry_tbl, ctx)).await?;
            let mut out = HashMap::new();
            for pair in result.pairs::<String, String>() {
                let (k, v) = pair?;
                out.insert(k, v);
            }
            Ok(out)
        };
        body.await
            .map_err(|e: mlua::Error| Error::SourceExtrasFailed {
                plugin: plugin_name.to_string(),
                message: e.to_string(),
            })
    }

    /// Ask the plugin that produced `entry` for the wallpaper's
    /// editable property schema as a JSON string.
    pub async fn call_properties(
        &self,
        plugin_name: &str,
        entry: &WallpaperEntry,
    ) -> Result<Option<String>> {
        let key = self
            .plugins
            .get(plugin_name)
            .ok_or_else(|| Error::SourcePluginNotFound(plugin_name.to_string()))?;
        let Some(info) = self.plugin_infos.get(plugin_name) else {
            return Err(Error::SourcePluginNotFound(plugin_name.to_string()));
        };
        if !info.capabilities.wallpaper.properties {
            return Ok(None);
        }
        let module: LuaTable = self.lua.registry_value(key)?;
        let wallpaper_api: LuaTable = module.get("wallpaper")?;
        let props_fn: LuaFunction = wallpaper_api.get("properties")?;
        let entry_tbl = self.wallpaper_entry_table(entry)?;
        let ctx = self.build_ctx(Some(plugin_name), &[])?;
        let result: mlua::Value = props_fn
            .call_async((entry_tbl, ctx))
            .await
            .map_err(|e| Error::Internal(anyhow!("properties({plugin_name}): {e}")))?;
        match result {
            mlua::Value::Nil => Ok(None),
            mlua::Value::String(s) => Ok(Some(s.to_str()?.to_string())),
            other => Ok(lua_to_json(&other).map(|j| j.to_string())),
        }
    }

    /// Ask every plugin that exports `auto_detect(ctx)` to probe
    /// well-known filesystem locations and report any that exist.
    pub async fn auto_detect_all(&self) -> Result<HashMap<String, Vec<String>>> {
        let mut out: HashMap<String, Vec<String>> = HashMap::new();
        let empty: [String; 0] = [];
        let mut plugin_names: Vec<String> = self.plugin_infos.keys().cloned().collect();
        plugin_names.sort();
        for name in plugin_names {
            let Some(info) = self.plugin_infos.get(&name) else {
                continue;
            };
            let Some(source) = &info.capabilities.source else {
                continue;
            };
            if !source.auto_detect {
                continue;
            }
            let key = self
                .plugins
                .get(&name)
                .ok_or_else(|| Error::SourcePluginNotFound(name.clone()))?;
            let module: LuaTable = self.lua.registry_value(key)?;
            let source_api: LuaTable = module.get("source")?;
            let auto_fn: LuaFunction = source_api.get("auto_detect")?;
            let ctx = self.build_ctx(None, &empty)?;
            let results: LuaTable = match auto_fn.call_async(ctx).await {
                Ok(t) => t,
                Err(e) => {
                    log::warn!("auto_detect plugin {name}: {e}");
                    continue;
                }
            };
            let paths: Vec<String> = results
                .sequence_values::<String>()
                .filter_map(|v| v.ok())
                .collect();
            if !paths.is_empty() {
                out.insert(name, paths);
            }
        }
        Ok(out)
    }

    // -----------------------------------------------------------------------
    // Discover API — generic remote browsing relayed into plugin Lua.

    /// List plugins that opt into discovery and their declared sort/tag
    /// options.
    pub fn discover_sources(&self) -> Result<Vec<DiscoverSourceInfo>> {
        let mut out = Vec::new();
        for info in self.plugin_infos.values() {
            let Some(disc) = &info.capabilities.discover else {
                continue;
            };
            out.push(DiscoverSourceInfo {
                plugin_id: info.name.clone(),
                name: info.name.clone(),
                supports_search: disc.supports_search,
                sorts: disc.sorts.clone(),
                tags: disc.tags.clone(),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Relay a discover/search request to a plugin's `discover.search(ctx, params)`
    /// Lua function. `params` is `{ query, sort, page, tags }`.
    pub async fn call_discover(
        &self,
        plugin_name: &str,
        query: &str,
        sort: &str,
        page: u32,
        tags: &[String],
    ) -> Result<DiscoverSearchResult> {
        let key = self
            .plugins
            .get(plugin_name)
            .ok_or_else(|| Error::SourcePluginNotFound(plugin_name.to_string()))?;
        let Some(info) = self.plugin_infos.get(plugin_name) else {
            return Err(Error::SourcePluginNotFound(plugin_name.to_string()));
        };
        if info.capabilities.discover.is_none() {
            return Err(Error::DiscoverUnsupported(plugin_name.to_string()));
        }
        let default_wp_type = info
            .capabilities
            .source
            .as_ref()
            .and_then(|source| source.types.first())
            .cloned()
            .unwrap_or_default();
        let module: LuaTable = self.lua.registry_value(key)?;
        let discover_api: LuaTable = module.get("discover")?;
        let discover_fn: LuaFunction = discover_api
            .get("search")
            .map_err(|_| Error::DiscoverUnsupported(plugin_name.to_string()))?;

        let params = self.lua.create_table()?;
        params.set("query", query)?;
        params.set("sort", sort)?;
        params.set("page", page)?;
        let tags_tbl = self.lua.create_table()?;
        for (i, t) in tags.iter().enumerate() {
            tags_tbl.set(i + 1, t.clone())?;
        }
        params.set("tags", tags_tbl)?;

        let ctx = self.build_ctx(Some(plugin_name), &[])?;
        let result: LuaTable =
            discover_fn
                .call_async((ctx, params))
                .await
                .map_err(|e| Error::DiscoverFailed {
                    plugin: plugin_name.to_string(),
                    message: e.to_string(),
                })?;

        let mut items = Vec::new();
        let item_rows: LuaTable = result.get("items").map_err(|e| Error::DiscoverFailed {
            plugin: plugin_name.to_string(),
            message: format!("discover.search result.items required: {e}"),
        })?;
        for (idx, row) in item_rows.sequence_values::<LuaTable>().enumerate() {
            let row = row.map_err(|e| Error::DiscoverFailed {
                plugin: plugin_name.to_string(),
                message: format!(
                    "discover.search result.items[{}] must be a table: {e}",
                    idx + 1
                ),
            })?;
            let context = format!("module.discover.search result.items[{}]", idx + 1);
            items.push(DiscoverItem {
                id: Self::require_string(&row, "id", &context)?,
                title: Self::require_string(&row, "title", &context)?,
                preview_url: Self::require_string(&row, "preview_url", &context)?,
                author: Self::require_string(&row, "author", &context)?,
                wp_type: {
                    let value = Self::optional_string(&row, "wp_type", &context)?;
                    if value.is_empty() {
                        default_wp_type.clone()
                    } else {
                        value
                    }
                },
                extra: parse_lua_string_map(&row, "extra", &context)?,
            });
        }
        let has_more = result
            .get::<bool>("has_more")
            .map_err(|e| Error::DiscoverFailed {
                plugin: plugin_name.to_string(),
                message: format!("discover.search result.has_more required: {e}"),
            })?;
        Ok(DiscoverSearchResult { items, has_more })
    }

    /// Relay a detail request to a plugin's `discover.details(ctx, id)` Lua function.
    pub async fn call_details(&self, plugin_name: &str, id: &str) -> Result<DiscoverDetails> {
        let key = self
            .plugins
            .get(plugin_name)
            .ok_or_else(|| Error::SourcePluginNotFound(plugin_name.to_string()))?;
        let Some(info) = self.plugin_infos.get(plugin_name) else {
            return Err(Error::SourcePluginNotFound(plugin_name.to_string()));
        };
        let Some(discover) = &info.capabilities.discover else {
            return Err(Error::DiscoverUnsupported(plugin_name.to_string()));
        };
        if !discover.supports_details {
            return Err(Error::DiscoverUnsupported(plugin_name.to_string()));
        }
        let module: LuaTable = self.lua.registry_value(key)?;
        let discover_api: LuaTable = module.get("discover")?;
        let details_fn: LuaFunction = discover_api
            .get("details")
            .map_err(|_| Error::DiscoverUnsupported(plugin_name.to_string()))?;

        let ctx = self.build_ctx(Some(plugin_name), &[])?;
        let result: LuaTable = details_fn
            .call_async((ctx, id.to_string()))
            .await
            .map_err(|e| Error::DiscoverFailed {
                plugin: plugin_name.to_string(),
                message: e.to_string(),
            })?;

        Ok(DiscoverDetails {
            description: Self::require_string(
                &result,
                "description",
                "module.discover.details result",
            )?,
            size: Self::require_string(&result, "size", "module.discover.details result")?,
            width: result.get::<u32>("width").ok(),
            height: result.get::<u32>("height").ok(),
            tags: Self::require_string_sequence(&result, "tags", "module.discover.details result")?,
            extra: parse_lua_string_map(&result, "extra", "module.discover.details result")?,
        })
    }

    /// Relay a download-resolution request to a plugin's
    /// `discover.download(ctx, id)` function. The daemon owns the actual file transfer.
    pub async fn call_download(&self, plugin_name: &str, id: &str) -> Result<DiscoverDownload> {
        let key = self
            .plugins
            .get(plugin_name)
            .ok_or_else(|| Error::SourcePluginNotFound(plugin_name.to_string()))?;
        let Some(info) = self.plugin_infos.get(plugin_name) else {
            return Err(Error::SourcePluginNotFound(plugin_name.to_string()));
        };
        let Some(discover) = &info.capabilities.discover else {
            return Err(Error::DiscoverUnsupported(plugin_name.to_string()));
        };
        if !discover.supports_download {
            return Err(Error::DiscoverUnsupported(plugin_name.to_string()));
        }
        let module: LuaTable = self.lua.registry_value(key)?;
        let discover_api: LuaTable = module.get("discover")?;
        let download_fn: LuaFunction = discover_api
            .get("download")
            .map_err(|_| Error::DiscoverUnsupported(plugin_name.to_string()))?;

        let ctx = self.build_ctx(Some(plugin_name), &[])?;
        let result: LuaTable = download_fn
            .call_async((ctx, id.to_string()))
            .await
            .map_err(|e| Error::DiscoverFailed {
                plugin: plugin_name.to_string(),
                message: e.to_string(),
            })?;

        Ok(DiscoverDownload {
            wp_type: Self::require_string(&result, "wp_type", "module.discover.download result")?,
            url: Self::require_string(&result, "url", "module.discover.download result")?,
            filename: Self::require_string(&result, "filename", "module.discover.download result")?,
            title: Self::require_string(&result, "title", "module.discover.download result")?,
            preview_url: Self::optional_string(
                &result,
                "preview_url",
                "module.discover.download result",
            )?,
            description: Self::optional_string(
                &result,
                "description",
                "module.discover.download result",
            )?,
            tags: Self::optional_string_sequence(
                &result,
                "tags",
                "module.discover.download result",
            )?,
            external_id: Self::require_string(
                &result,
                "external_id",
                "module.discover.download result",
            )?,
            size: result.get::<i64>("size").ok(),
            width: result.get::<u32>("width").ok(),
            height: result.get::<u32>("height").ok(),
            content_rating: result.get::<String>("content_rating").ok(),
        })
    }

    pub fn plugins(&self) -> Result<Vec<SourcePluginInfo>> {
        let mut out = Vec::new();
        for info in self.plugin_infos.values() {
            let Some(source) = &info.capabilities.source else {
                continue;
            };
            out.push(SourcePluginInfo {
                name: info.name.clone(),
                plugin_id: info.plugin_id.clone(),
                types: source.types.clone(),
                version: info.version.clone(),
                library_label: source.library_label.clone(),
                library_hint: source.library_hint.clone(),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    pub fn supports_item_remove(&self, plugin_name: &str) -> bool {
        self.plugin_infos
            .get(plugin_name)
            .map(|info| info.capabilities.source_item_remove)
            .unwrap_or(false)
    }

    pub async fn remove_item(
        &self,
        plugin_name: &str,
        entry: &WallpaperEntry,
        libraries: &[String],
    ) -> Result<()> {
        let key = self
            .plugins
            .get(plugin_name)
            .ok_or_else(|| Error::SourcePluginNotFound(plugin_name.to_string()))?;
        let Some(info) = self.plugin_infos.get(plugin_name) else {
            return Err(Error::SourcePluginNotFound(plugin_name.to_string()));
        };
        if !info.capabilities.source_item_remove {
            return Err(Error::SourceItemRemoveUnsupported(plugin_name.to_string()));
        }

        let module: LuaTable = self.lua.registry_value(key)?;
        let source_api: LuaTable = module
            .get("source")
            .map_err(|_| Error::SourceItemRemoveUnsupported(plugin_name.to_string()))?;
        let remove_fn: LuaFunction = source_api
            .get("remove")
            .map_err(|_| Error::SourceItemRemoveUnsupported(plugin_name.to_string()))?;
        let ctx = self.build_ctx(Some(plugin_name), libraries)?;
        let entry_tbl = self.wallpaper_entry_table(entry)?;
        remove_fn
            .call_async::<()>((ctx, entry_tbl))
            .await
            .map_err(|e| Error::SourceItemRemoveFailed {
                plugin: plugin_name.to_string(),
                message: e.to_string(),
            })
    }

    pub fn plugin_version(&self, plugin_name: &str) -> Option<String> {
        self.plugin_infos
            .get(plugin_name)
            .map(|info| info.version.clone())
    }
}

// ---------------------------------------------------------------------------
// Helpers

fn parse_lua_string_map(
    tbl: &LuaTable,
    key: &str,
    context: &str,
) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    let Some(meta) = SourceManager::optional_table(tbl, key, context)? else {
        return Ok(map);
    };
    for pair in meta.pairs::<String, String>() {
        let (k, v) = pair
            .map_err(|e| Error::Internal(anyhow!("{context}.{key} must be a string map: {e}")))?;
        map.insert(k, v);
    }
    Ok(map)
}

fn json_to_lua(lua: &Lua, val: &serde_json::Value) -> LuaResult<LuaValue> {
    match val {
        serde_json::Value::Null => Ok(LuaValue::Nil),
        serde_json::Value::Bool(b) => Ok(LuaValue::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(LuaValue::Integer(i))
            } else {
                Ok(LuaValue::Number(n.as_f64().unwrap_or(0.0)))
            }
        }
        serde_json::Value::String(s) => Ok(LuaValue::String(lua.create_string(s)?)),
        serde_json::Value::Array(arr) => {
            let t = lua.create_table()?;
            for (i, v) in arr.iter().enumerate() {
                t.set(i + 1, json_to_lua(lua, v)?)?;
            }
            Ok(LuaValue::Table(t))
        }
        serde_json::Value::Object(obj) => {
            let t = lua.create_table()?;
            for (k, v) in obj {
                t.set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            Ok(LuaValue::Table(t))
        }
    }
}

/// Convert a `LuaValue` back to `serde_json::Value`.
/// Lua tables become arrays only with contiguous 1..=N integer keys.
fn lua_to_json(val: &LuaValue) -> Option<serde_json::Value> {
    match val {
        LuaValue::Nil => Some(serde_json::Value::Null),
        LuaValue::Boolean(b) => Some(serde_json::Value::Bool(*b)),
        LuaValue::Integer(i) => Some(serde_json::Value::Number((*i).into())),
        LuaValue::Number(n) => serde_json::Number::from_f64(*n).map(serde_json::Value::Number),
        LuaValue::String(s) => s
            .to_str()
            .ok()
            .map(|cow| serde_json::Value::String(cow.to_string())),
        LuaValue::Table(t) => {
            let len = t.raw_len();
            let mut all_int = len > 0;
            let mut count = 0;
            for pair in t.clone().pairs::<LuaValue, LuaValue>() {
                count += 1;
                let Ok((k, _)) = pair else {
                    all_int = false;
                    break;
                };
                if !matches!(&k, LuaValue::Integer(_)) {
                    all_int = false;
                    break;
                }
            }
            if all_int && count == len {
                let mut arr = Vec::with_capacity(len);
                for i in 1..=len {
                    let v: LuaValue = t.get(i).ok()?;
                    arr.push(lua_to_json(&v)?);
                }
                Some(serde_json::Value::Array(arr))
            } else {
                let mut map = serde_json::Map::new();
                for pair in t.clone().pairs::<LuaValue, LuaValue>() {
                    let (k, v) = pair.ok()?;
                    let key = match k {
                        LuaValue::String(s) => s.to_str().ok()?.to_string(),
                        LuaValue::Integer(i) => i.to_string(),
                        LuaValue::Number(n) => n.to_string(),
                        _ => continue,
                    };
                    map.insert(key, lua_to_json(&v)?);
                }
                Some(serde_json::Value::Object(map))
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::media::{MediaMeta, MediaProbe};
    use std::io::Write;

    struct FakeProbe {
        meta: MediaMeta,
    }
    impl MediaProbe for FakeProbe {
        fn probe_media(&self, _path: &str) -> MediaMeta {
            self.meta.clone()
        }
    }

    /// Drive an async scan from a sync `#[test]` — these tests don't
    /// touch the DB so a single-thread runtime is fine.
    fn block(fut: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fut)
    }

    fn block_value<T>(fut: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fut)
    }

    #[test]
    fn ctx_probe_callable_from_lua() {
        let probe = Arc::new(FakeProbe {
            meta: MediaMeta {
                width: Some(1920),
                height: Some(1080),
            },
        });
        let dir = tempfile::tempdir().unwrap();
        let plugin_path = dir.path().join("probe_test.lua");
        let mut f = std::fs::File::create(&plugin_path).unwrap();
        write!(
            f,
            r#"
local M = {{}}
function M.info()
    return {{
        name = "probe_test",
        capabilities = {{
            source = {{ types = {{"video"}}, scan = true }},
        }},
    }}
end
M.source = {{}}
function M.source.scan(ctx)
    local m = ctx.probe("/fake/path/video.mp4")
    if m == nil then error("probe returned nil") end
    return {{
        {{
            id = "v1",
            name = "Video",
            wp_type = "video",
            resource = "/lib/v1.mp4",
            library_root = "/lib",
            metadata = {{}},
            _probe_size = m.size,
            _probe_width = m.width,
            _probe_height = m.height,
        }},
    }}
end
return M
"#
        )
        .unwrap();

        let mut mgr = SourceManager::with_probe(probe as Arc<dyn MediaProbe>).unwrap();
        mgr.load_plugin(&plugin_path, "test.plugin", "1.0", ENTRY_VERSION)
            .unwrap();
        block(async { mgr.scan_all(&HashMap::new()).await.unwrap() });

        let entries = mgr.list();
        assert_eq!(entries.len(), 1);
        // The Lua plugin called ctx.probe successfully (it would error() otherwise).
        // Verify the entry was emitted correctly.
        assert_eq!(entries[0].resource, "/lib/v1.mp4");
    }

    #[test]
    fn test_load_and_scan_plugin() {
        let dir = tempfile::tempdir().unwrap();

        // Write a minimal source plugin
        let plugin_path = dir.path().join("test_source.lua");
        let mut f = std::fs::File::create(&plugin_path).unwrap();
        write!(
            f,
            r#"
local M = {{}}
function M.info()
    return {{
        name = "test",
        capabilities = {{
            source = {{ types = {{"image"}}, scan = true }},
        }},
    }}
end
M.source = {{}}
function M.source.scan(ctx)
    return {{
        {{ id = "w1", name = "Test Wallpaper", wp_type = "image",
           resource = "/tmp/test.png", metadata = {{}} }},
    }}
end
return M
"#
        )
        .unwrap();

        let mut mgr = SourceManager::new().unwrap();
        let name = mgr
            .load_plugin(&plugin_path, "test.plugin", "1.0", ENTRY_VERSION)
            .unwrap();
        assert_eq!(name, "test");

        block(async { mgr.scan_all(&HashMap::new()).await.unwrap() });
        assert_eq!(mgr.list().len(), 1);
        assert_eq!(mgr.list()[0].name, "Test Wallpaper");
        assert_eq!(mgr.list()[0].wp_type, "image");
        assert_eq!(mgr.list()[0].plugin_name, "test");

        let by_type = mgr.list_by_type("image");
        assert_eq!(by_type.len(), 1);

        let by_type_empty = mgr.list_by_type("video");
        assert!(by_type_empty.is_empty());

        // Identity is the DB item.id, assigned at sync time; this
        // scan-only test leaves it at 0, so look up by "0".
        let found = mgr.get("0");
        assert!(found.is_some());

        let plugins = mgr.plugins().unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "test");
        assert_eq!(plugins[0].version, "1.0");
    }

    #[test]
    fn plugin_import_loads_plugin_local_modules() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("helpers")).unwrap();
        std::fs::write(
            dir.path().join("helpers/names.lua"),
            r#"
local M = {}
function M.name()
    return "Imported"
end
return M
"#,
        )
        .unwrap();
        let plugin_path = dir.path().join("main.lua");
        std::fs::write(
            &plugin_path,
            r#"
local names = import("helpers.names")
local M = {}
function M.info()
    return {
        name = "imported",
        capabilities = {
            source = { types = {"image"}, scan = true },
            discover = { search = true, details = true, download = true },
        },
    }
end
M.source = {}
function M.source.scan(ctx)
    return {
        { name = names.name(), wp_type = "image", resource = "/tmp/imported.png" },
    }
end
M.discover = {}
function M.discover.search(ctx, params)
    return {
        items = {
            { id = "abc", title = names.name(), preview_url = "", author = "" },
        },
        has_more = false,
    }
end
function M.discover.details(ctx, id)
    return {
        description = names.name(),
        size = "42",
        width = 10,
        height = 20,
        tags = {"tag"},
    }
end
function M.discover.download(ctx, id)
    return {
        wp_type = "image",
        url = "https://example.invalid/" .. id,
        filename = id .. ".jpg",
        title = names.name(),
        tags = {"tag"},
        external_id = id,
        size = 42,
        width = 10,
        height = 20,
        content_rating = "Everyone",
    }
end
return M
"#,
        )
        .unwrap();

        let mut mgr = SourceManager::new().unwrap();
        let name = mgr
            .load_plugin(&plugin_path, "test.plugin", "1.0", ENTRY_VERSION)
            .unwrap();
        assert_eq!(name, "imported");
        block(async { mgr.scan_all(&HashMap::new()).await.unwrap() });
        assert_eq!(mgr.list()[0].name, "Imported");

        let search =
            block_value(async { mgr.call_discover("imported", "", "", 1, &[]).await.unwrap() });
        assert_eq!(search.items[0].wp_type, "image");

        let dl = block_value(async { mgr.call_download("imported", "abc").await.unwrap() });
        assert_eq!(dl.wp_type, "image");
        assert_eq!(dl.filename, "abc.jpg");
        assert_eq!(dl.title, "Imported");
        assert_eq!(dl.tags, vec!["tag"]);
        assert_eq!(dl.external_id, "abc");
        assert_eq!(dl.size, Some(42));
        let detail = block_value(async { mgr.call_details("imported", "abc").await.unwrap() });
        assert_eq!(detail.width, Some(10));
        assert_eq!(detail.height, Some(20));
    }

    #[test]
    fn source_item_remove_works_without_scan_capability() {
        let dir = tempfile::tempdir().unwrap();
        let item_path = dir.path().join("wallpaper.png");
        std::fs::write(&item_path, b"image").unwrap();
        let plugin_path = dir.path().join("remove.lua");
        std::fs::write(
            &plugin_path,
            r#"
local M = {}
function M.info()
    return {
        name = "remove_only",
        capabilities = {},
    }
end
M.source = {}
function M.source.remove(ctx, item)
    if item.path ~= item.resource then error("path/resource mismatch") end
    if item.relative_path ~= "wallpaper.png" then error("wrong relative path") end
    if item.external_id ~= "ext-1" then error("missing external id") end
    ctx.remove_file(item.path)
end
return M
"#,
        )
        .unwrap();

        let mut mgr = SourceManager::new().unwrap();
        let name = mgr
            .load_plugin(&plugin_path, "test.plugin", "1.0", ENTRY_VERSION)
            .unwrap();
        assert_eq!(name, "remove_only");
        assert!(mgr.supports_item_remove("remove_only"));

        let entry = WallpaperEntry {
            item_id: 42,
            name: "Wallpaper".to_string(),
            wp_type: "image".to_string(),
            resource: item_path.to_string_lossy().to_string(),
            preview: None,
            plugin_name: "remove_only".to_string(),
            library_root: dir.path().to_string_lossy().to_string(),
            description: None,
            tags: vec!["tag".to_string()],
            external_id: Some("ext-1".to_string()),
            size: None,
            width: None,
            height: None,
            content_rating: None,
            modified_at: None,
            create_at: 0,
        };
        let libraries = vec![entry.library_root.clone()];
        block_value(async { mgr.remove_item("remove_only", &entry, &libraries).await }).unwrap();
        assert!(!item_path.exists());
    }

    #[test]
    fn source_item_remove_rejects_plugins_without_remove() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_path = dir.path().join("no_remove.lua");
        std::fs::write(
            &plugin_path,
            r#"
local M = {}
function M.info()
    return {
        name = "no_remove",
        capabilities = {},
    }
end
return M
"#,
        )
        .unwrap();

        let mut mgr = SourceManager::new().unwrap();
        let name = mgr
            .load_plugin(&plugin_path, "test.plugin", "1.0", ENTRY_VERSION)
            .unwrap();
        assert_eq!(name, "no_remove");
        assert!(!mgr.supports_item_remove("no_remove"));

        let entry = WallpaperEntry {
            item_id: 7,
            name: "Wallpaper".to_string(),
            wp_type: "image".to_string(),
            resource: "/tmp/wallpaper.png".to_string(),
            preview: None,
            plugin_name: "no_remove".to_string(),
            library_root: "/tmp".to_string(),
            description: None,
            tags: Vec::new(),
            external_id: None,
            size: None,
            width: None,
            height: None,
            content_rating: None,
            modified_at: None,
            create_at: 0,
        };
        let err =
            block_value(async { mgr.remove_item("no_remove", &entry, &[]).await }).unwrap_err();
        assert!(matches!(
            err,
            Error::SourceItemRemoveUnsupported(plugin) if plugin == "no_remove"
        ));
    }

    #[test]
    fn plugin_import_rejects_path_escape() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_path = dir.path().join("main.lua");
        std::fs::write(
            &plugin_path,
            r#"
local bad = import("../outside")
return bad
"#,
        )
        .unwrap();

        let mut mgr = SourceManager::new().unwrap();
        assert!(mgr
            .load_plugin(&plugin_path, "test.plugin", "1.0", ENTRY_VERSION)
            .is_err());
    }

    #[test]
    fn unsupported_entry_version_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_path = dir.path().join("main.lua");
        std::fs::write(
            &plugin_path,
            r#"
local M = {}
function M.info()
    return {
        name = "too_new",
        capabilities = {
            discover = { search = true },
        },
    }
end
M.discover = {}
function M.discover.search(ctx, params)
    return { items = {}, has_more = false }
end
return M
"#,
        )
        .unwrap();

        let mut mgr = SourceManager::new().unwrap();
        assert!(mgr
            .load_plugin(&plugin_path, "test.plugin", "1.0", ENTRY_VERSION + 1)
            .is_err());
    }

    #[test]
    fn wallhaven_plugin_is_discover_only() {
        let plugin_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("plugins/org.waywallen.wallhaven/main.lua");

        let mut mgr = SourceManager::new().unwrap();
        let name = mgr
            .load_plugin(&plugin_path, "test.plugin", "1.0", ENTRY_VERSION)
            .unwrap();
        assert_eq!(name, "wallhaven");
        assert!(mgr.plugins().unwrap().is_empty());

        let sources = mgr.discover_sources().unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].plugin_id, "wallhaven");
        assert!(sources[0].supports_search);
        assert!(sources[0].tags.iter().any(|tag| tag == "Anime"));

        let env = mgr.plugin_lua_env(plugin_path.parent().unwrap()).unwrap();
        let import: LuaFunction = env.get("import").unwrap();
        let map: LuaTable = import.call("wallhaven.map").unwrap();
        let search_item: LuaFunction = map.get("search_item").unwrap();
        let item = mgr.lua.create_table().unwrap();
        item.set("id", "abc").unwrap();
        let mapped: LuaTable = search_item.call(item).unwrap();
        assert_eq!(mapped.get::<String>("wp_type").unwrap(), "image");
    }

    #[test]
    fn video_source_plugin_discovers_video_files() {
        let lib = tempfile::tempdir().unwrap();
        let nested = lib.path().join("album");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(lib.path().join("clip.MP4"), b"video bytes").unwrap();
        std::fs::write(lib.path().join("animated.gif"), b"image source owns gif").unwrap();
        std::fs::write(nested.join("poster.png"), b"not a video").unwrap();
        std::fs::write(nested.join("loop.webm"), b"more video bytes").unwrap();

        let plugin_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("plugins/org.waywallen.video/main.lua");

        let mut mgr = SourceManager::new().unwrap();
        let name = mgr
            .load_plugin(&plugin_path, "test.plugin", "1.0", ENTRY_VERSION)
            .unwrap();
        assert_eq!(name, "video");

        let mut libs = HashMap::new();
        libs.insert(
            "video".to_string(),
            vec![lib.path().to_string_lossy().to_string()],
        );
        block(async { mgr.scan_all(&libs).await.unwrap() });

        let entries = mgr.list();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.wp_type == "video"));
        assert!(entries.iter().all(|e| e.plugin_name == "video"));
        assert!(entries.iter().all(|e| e.preview.is_none()));
        assert!(entries.iter().all(|e| e.size.is_some()));
        assert!(entries.iter().all(|e| e.width.is_none()));
        assert!(entries.iter().all(|e| e.height.is_none()));
        assert!(entries.iter().all(|e| e.content_rating.is_none()));
        // SPAWN_VERSION 3 keeps the canonical resource path in
        // `entry.resource`.

        let clip_path = lib.path().join("clip.MP4").to_string_lossy().to_string();
        let clip = entries
            .iter()
            .find(|entry| entry.resource == clip_path)
            .unwrap()
            .clone();
        assert_eq!(clip.name, "clip");
        assert_eq!(clip.resource, clip_path);

        let extras = block_value(async { mgr.call_extras("video", &clip).await.unwrap() });
        assert_eq!(extras.get("path"), Some(&clip.resource));

        assert_eq!(mgr.list_by_type("video").len(), 2);
        assert!(mgr.list_by_type("image").is_empty());
    }
}
