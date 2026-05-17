pub mod pb {
    tonic::include_proto!("plc_runtime");
}

pub mod server;
pub mod client;

pub use pb::*;
