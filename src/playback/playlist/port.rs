use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::error::Result;
use crate::wallframe::scheduler::DisplayId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplySharing {
    Independent,
    Shared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplySource {
    Activation,
    Rotation,
    Jump,
    Rebuild,
    Attach,
}

#[derive(Debug, Clone)]
pub struct ApplyRequest {
    pub source: ApplySource,
    pub entry_id: String,
    pub display_ids: Vec<DisplayId>,
    pub sharing: ApplySharing,
    pub first_frame_timeout: Option<Duration>,
}

type ApplyFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;
type ApplyHandler = dyn Fn(ApplyRequest) -> ApplyFuture + Send + Sync;

#[derive(Clone)]
pub struct ApplyPort(Arc<ApplyHandler>);

impl ApplyPort {
    pub fn new<F, Fut>(handler: F) -> Self
    where
        F: Fn(ApplyRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        Self(Arc::new(move |request| Box::pin(handler(request))))
    }

    pub async fn apply(&self, request: ApplyRequest) -> Result<()> {
        (self.0)(request).await
    }
}
