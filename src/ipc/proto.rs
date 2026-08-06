pub use crate::ipc::generated::{
    AudioStreamFormat, AudioWindow, BindFailure, BufferAllocationFailureKind, BufferDirective,
    BufferFormat, BufferMemorySource, BufferPath, BufferPool, ControlTransition, DecodeError,
    DrmNode as WireDrmNode, Event as EventMsg, EventIn as ControlMsg, EventSubscription,
    EventSubscriptionResult, EventSubscriptionStatus, Extent, Frame, InitRejection,
    MediaPlaybackState, MprisSnapshot as WireMprisSnapshot, PointerAxis, PointerAxisSource,
    PointerButton, PointerButtonState, PointerMotion, ProducerCapabilities, RendererInit,
    RendererState, RgbaColor, PROTOCOL_NAME, PROTOCOL_VERSION,
};
