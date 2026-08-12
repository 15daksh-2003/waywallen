#[allow(dead_code, clippy::all)]
pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/ipc_generated.rs"));
}

pub mod proto;
pub mod uds;
