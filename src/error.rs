use thiserror::Error;

use crate::control_proto as pb;

/// Daemon-wide typed error. See module docs for construction guidance.
//
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum Error {
    /// Catch-all for opaque errors bubbling up from `anyhow::Result`.
    /// Code with a known category should use a specific variant.
    #[error("{0:#}")]
    Internal(#[from] anyhow::Error),

    /// Sea-ORM database access failure. Use the `?` operator on a
    /// `Result<_, sea_orm::DbErr>` to land here automatically.
    #[error("db: {0}")]
    Db(#[from] sea_orm::DbErr),

    /// Local I/O failure (file open, socket bind, …).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// Wire-side protobuf decode failure. Surfaces as
    /// `ErrorCode::Decode`.
    #[error("decode: {0}")]
    Decode(#[from] prost::DecodeError),

    /// JSON encode/decode failure.
    /// Used by `repo` for `library.metadata` and similar columns.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// `tokio::task::JoinError` from a spawned task.
    /// Panics and unexpected cancellation are daemon-internal failures.
    #[error("task join: {0}")]
    Join(#[from] tokio::task::JoinError),

    /// Lua VM error from a source-plugin callback, registry mismatch,
    /// table key lookup, or similar operation.
    #[error("lua: {0}")]
    Lua(#[from] mlua::Error),

    /// Inbound `Request.payload` was `None` — caller sent an envelope
    /// with no oneof variant set.
    #[error("{0}")]
    UnexpectedPayload(&'static str),

    /// Caller-supplied invalid argument that doesn't fit a more
    /// specific variant.
    #[error("{0}")]
    InvalidArgument(String),

    /// Precondition (e.g. "not enough free memory") that doesn't fit a
    /// more specific variant.
    #[error("{0}")]
    FailedPrecondition(String),

    /// Apply path: no display has registered with the daemon yet.
    #[error("no display registered")]
    NoDisplayRegistered,

    /// Apply path: source snapshot has no entry with this id.
    #[error("wallpaper '{0}' not found")]
    WallpaperNotFound(String),

    /// Renderer manager has no live renderer with this id.
    #[error("unknown renderer '{0}'")]
    RendererNotFound(String),

    /// Renderer registry has no manifest declaring support for this
    /// wallpaper type.
    #[error("no renderer for wallpaper type '{0}'")]
    NoRendererForType(String),

    /// The caller named a specific renderer but the wallpaper's type
    /// is not in the manifest's `types` list.
    #[error("renderer '{renderer}' does not support wallpaper type '{ty}'")]
    RendererTypeMismatch { renderer: String, ty: String },

    /// `renderer_manager.spawn` failed (fork/exec/handshake/timeout/…).
    #[error("spawn failed: {0}")]
    RendererSpawnFailed(String),

    /// `renderer_manager.send_control` failed.
    /// Usually a closed socket or missing renderer handle.
    #[error("renderer control failed: {0}")]
    RendererControlFailed(String),

    /// Apply path: the renderer connected but never produced a usable
    /// frame, or exited before doing so.
    #[error("renderer did not produce a frame: {0}")]
    RendererFrameFailed(String),

    /// Source-plugin Lua name was not in the registered set.
    #[error("source plugin '{0}' not found")]
    SourcePluginNotFound(String),

    /// Source plugin's `extras(entry)` Lua callback raised.
    /// The stringified Lua error rides in `message`.
    #[error("source_plugin '{plugin}'.extras() failed: {message}")]
    SourceExtrasFailed { plugin: String, message: String },

    /// Source plugin does not export `source.remove(ctx, item)`.
    #[error("source plugin '{0}' does not support item remove")]
    SourceItemRemoveUnsupported(String),

    /// Source plugin's `source.remove(ctx, item)` Lua callback raised.
    #[error("source_plugin '{plugin}'.remove() failed: {message}")]
    SourceItemRemoveFailed { plugin: String, message: String },

    /// Scanned item does not map to a subscription owned by its source.
    #[error("source plugin '{0}' does not support item unsubscribe")]
    SourceItemUnsubscribeUnsupported(String),

    /// Installing a plugin `.zip` failed (bad path, unreadable archive,
    /// unsafe entry, or no `plugin.toml`).
    #[error("plugin install failed: {0}")]
    PluginInstallFailed(String),

    /// Deleting a user-installed plugin failed.
    #[error("plugin delete failed: {0}")]
    PluginDeleteFailed(String),

    /// Source plugin does not export the requested discover function
    /// (`discover` / `details`), so it cannot serve a discover request.
    #[error("source plugin '{0}' does not support discover")]
    DiscoverUnsupported(String),

    /// Source plugin's `discover.*` Lua callback raised. The stringified
    /// Lua error rides in `message`.
    #[error("source_plugin '{plugin}'.discover() failed: {message}")]
    DiscoverFailed { plugin: String, message: String },

    /// Source plugin's `discover.resolve` Lua callback raised (e.g. an
    /// unclassifiable downloaded directory).
    #[error("source_plugin '{plugin}'.resolve() failed: {message}")]
    ResolveFailed { plugin: String, message: String },

    /// Caller asked an apply path to handle an unsupported `wp_type`.
    /// For example, the portal fallback only accepts images.
    #[error("wallpaper type '{0}' not supported by this apply path")]
    WallpaperTypeNotSupported(String),

    /// An `org.freedesktop.portal.Desktop` request failed (bus unavailable,
    /// no portal backend, user cancellation, or an invalid response).
    #[error("portal call failed: {0}")]
    PortalCallFailed(String),

    /// `coerce_and_validate` rejected a `SettingsSet` value.
    #[error("settings validation failed: {0}")]
    SettingsValidationFailed(String),

    /// Settings persisted, but live `ApplySettings` push failed.
    /// Carries the joined per-renderer errors.
    #[error("settings persisted but live hot-reload failed: {0}")]
    SettingsApplyFailed(String),

    /// Library row was not in the persisted set.
    #[error("library {0} not found")]
    LibraryNotFound(i64),

    /// Playlist activate / lookup found no matching row.
    #[error("playlist not found: {0}")]
    PlaylistNotFound(String),

    /// Playlist create / mutate rejected by the persistence layer
    /// (constraint violation, bad name, …).
    #[error("playlist invalid: {0}")]
    PlaylistInvalid(String),

    /// Diagnostic wrapper that attaches context without changing type.
    /// `error_code()` recurses to the source error.
    #[error("{context}: {source}")]
    WithContext {
        context: String,
        #[source]
        source: Box<Error>,
    },
}

/// Daemon-wide `Result` alias.
/// Callers explicitly import it as `use crate::error::Result;`.
#[allow(dead_code)]
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    /// Attach a diagnostic context message.
    /// The original variant's `error_code()` is preserved.
    pub fn context(self, ctx: impl std::fmt::Display) -> Self {
        Self::WithContext {
            context: ctx.to_string(),
            source: Box::new(self),
        }
    }

    /// Map this error onto its wire-level `pb::ErrorCode`.
    /// Always returns a non-`Ok` code.
    pub fn error_code(&self) -> pb::ErrorCode {
        use pb::ErrorCode as E;
        match self {
            Self::WithContext { source, .. } => source.error_code(),
            Self::Internal(_) | Self::Io(_) | Self::Json(_) | Self::Join(_) | Self::Lua(_) => {
                E::Internal
            }
            Self::Db(_) => E::Db,
            Self::Decode(_) => E::Decode,
            Self::UnexpectedPayload(_) => E::UnexpectedPayload,
            Self::InvalidArgument(_) => E::InvalidArgument,
            Self::FailedPrecondition(_) => E::FailedPrecondition,
            Self::NoDisplayRegistered => E::NoDisplayRegistered,
            Self::WallpaperNotFound(_) => E::WallpaperNotFound,
            Self::RendererNotFound(_) => E::RendererNotFound,
            Self::NoRendererForType(_) => E::NoRendererForType,
            Self::RendererTypeMismatch { .. } => E::RendererTypeMismatch,
            Self::RendererSpawnFailed(_) => E::RendererSpawnFailed,
            Self::RendererControlFailed(_) => E::RendererControlFailed,
            Self::RendererFrameFailed(_) => E::RendererFrameFailed,
            Self::SourcePluginNotFound(_) => E::SourcePluginNotFound,
            Self::SourceExtrasFailed { .. } => E::SourceExtrasFailed,
            Self::SourceItemRemoveUnsupported(_) => E::SourceItemRemoveUnsupported,
            Self::SourceItemRemoveFailed { .. } => E::SourceItemRemoveFailed,
            Self::SourceItemUnsubscribeUnsupported(_) => E::SourceItemUnsubscribeUnsupported,
            Self::PluginInstallFailed(_) => E::PluginInstallFailed,
            Self::PluginDeleteFailed(_) => E::PluginDeleteFailed,
            // Discover types map onto generic codes; the discover request
            // proto (and any dedicated codes) is owned by the transport PR.
            Self::DiscoverUnsupported(_) => E::FailedPrecondition,
            Self::DiscoverFailed { .. } => E::Internal,
            Self::ResolveFailed { .. } => E::Internal,
            Self::WallpaperTypeNotSupported(_) => E::WallpaperTypeNotSupported,
            Self::PortalCallFailed(_) => E::PortalCallFailed,
            Self::SettingsValidationFailed(_) => E::SettingsValidationFailed,
            Self::SettingsApplyFailed(_) => E::SettingsApplyFailed,
            Self::LibraryNotFound(_) => E::LibraryNotFound,
            Self::PlaylistNotFound(_) => E::PlaylistNotFound,
            Self::PlaylistInvalid(_) => E::PlaylistInvalid,
        }
    }

    /// Coarse legacy `pb::Status` derived from `error_code()`. Kept so
    /// pre-`error_code` clients see a sensible status without a daemon
    pub fn status(&self) -> pb::Status {
        use pb::ErrorCode as E;
        use pb::Status as S;
        match self.error_code() {
            E::Ok => S::Ok,
            E::Decode
            | E::InvalidArgument
            | E::UnexpectedPayload
            | E::RendererTypeMismatch
            | E::NoRendererForType
            | E::SettingsValidationFailed
            | E::WallpaperTypeNotSupported
            | E::PlaylistInvalid => S::InvalidArgument,
            E::FailedPrecondition
            | E::NoDisplayRegistered
            | E::SourceItemRemoveUnsupported
            | E::SourceItemUnsubscribeUnsupported => S::FailedPrecondition,
            E::WallpaperNotFound
            | E::RendererNotFound
            | E::SourcePluginNotFound
            | E::LibraryNotFound
            | E::PlaylistNotFound => S::NotFound,
            E::Internal
            | E::Db
            | E::RendererSpawnFailed
            | E::RendererControlFailed
            | E::RendererFrameFailed
            | E::SourceExtrasFailed
            | E::SourceItemRemoveFailed
            | E::PluginInstallFailed
            | E::PluginDeleteFailed
            | E::SettingsApplyFailed
            | E::PortalCallFailed => S::Internal,
        }
    }

    /// Build a wire `Response` for an errored dispatch. Counterpart of
    /// `ok_response` for the success path.
    pub fn to_response(&self, request_id: u64) -> pb::Response {
        pb::Response {
            request_id,
            status: self.status() as i32,
            error_code: self.error_code() as i32,
            message: self.to_string(),
            payload: None,
        }
    }
}

/// Map onto the zbus error vocabulary so the D-Bus surface
/// (`Daemon1`) carries some structure beyond the generic `Failed`.
impl From<Error> for zbus::fdo::Error {
    fn from(e: Error) -> Self {
        let msg = e.to_string();
        let code = e.error_code();
        use pb::ErrorCode as E;
        match code {
            E::WallpaperNotFound
            | E::RendererNotFound
            | E::SourcePluginNotFound
            | E::LibraryNotFound
            | E::PlaylistNotFound => zbus::fdo::Error::FileNotFound(msg),
            E::InvalidArgument
            | E::UnexpectedPayload
            | E::Decode
            | E::RendererTypeMismatch
            | E::NoRendererForType
            | E::SettingsValidationFailed
            | E::WallpaperTypeNotSupported
            | E::PlaylistInvalid => zbus::fdo::Error::InvalidArgs(msg),
            // FailedPrecondition / NoDisplayRegistered / Internal-class
            // / Db / Spawn / Control / Extras / SettingsApply — no
            _ => zbus::fdo::Error::Failed(msg),
        }
    }
}

/// Build a wire `Response` for a successful dispatch. Pins
/// `error_code = OK` and `status = OK`.
pub fn ok_response(request_id: u64, payload: pb::response::Payload) -> pb::Response {
    pb::Response {
        request_id,
        status: pb::Status::Ok as i32,
        error_code: pb::ErrorCode::Ok as i32,
        message: String::new(),
        payload: Some(payload),
    }
}

/// Extension trait for `Result<T, E>` where `E: Into<Error>`. Mirrors
/// `anyhow::Context` so callers migrating from `.with_context(...)?`
pub trait ResultExt<T> {
    /// Attach a static context. Always evaluates the context; prefer
    /// `with_context` when the context is expensive to build.
    fn context<C: std::fmt::Display>(self, ctx: C) -> Result<T, Error>;

    /// Attach a context computed lazily — only invoked on the error
    /// path. Mirrors `anyhow::Context::with_context`.
    fn with_context<C, F>(self, f: F) -> Result<T, Error>
    where
        C: std::fmt::Display,
        F: FnOnce() -> C;
}

impl<T, E> ResultExt<T> for std::result::Result<T, E>
where
    E: Into<Error>,
{
    fn context<C: std::fmt::Display>(self, ctx: C) -> Result<T, Error> {
        self.map_err(|e| e.into().context(ctx))
    }

    fn with_context<C, F>(self, f: F) -> Result<T, Error>
    where
        C: std::fmt::Display,
        F: FnOnce() -> C,
    {
        self.map_err(|e| e.into().context(f()))
    }
}
