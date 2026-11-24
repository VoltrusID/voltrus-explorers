pub mod pb {
    tonic::include_proto!("plc_runtime");
}

pub mod client;
pub mod server;

pub use pb::*;
