//! relay-agent — outbound device agent for Voltrus PLC Cloud.
//!
//! Usage: relay-agent --server ws://localhost:8787/api/relay \
//!                    --device-id <id> --pairing-token <token> \
//!                    [--driver echo|grpc] [--grpc-addr http://127.0.0.1:50051]

use std::time::Duration;

use anyhow::{bail, Result};
use clap::Parser;
use relay_agent::driver::{echo::EchoDriver, DeviceDriver};
use relay_agent::session::{run_with_reconnect, SessionConfig};

#[cfg(feature = "grpc")]
use relay_agent::driver::grpc::GrpcDriver;

#[derive(Debug, Parser)]
#[command(name = "relay-agent", about = "Voltrus PLC Cloud device relay agent")]
struct Args {
    /// Relay base URL, e.g. ws://localhost:8787/api/relay or wss://ide.voltrus.id/api/relay
    #[arg(long)]
    server: String,

    /// Device id from `POST /api/devices`
    #[arg(long)]
    device_id: String,

    /// Pairing token shown once at device creation
    #[arg(long)]
    pairing_token: String,

    /// Which PLC backend to expose: "echo" (simulated) or "grpc" (runtime)
    #[arg(long, default_value = "echo")]
    driver: String,

    /// gRPC address of the local PLC runtime (driver = grpc)
    #[arg(long, default_value = "http://127.0.0.1:50051")]
    grpc_addr: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if !args.server.starts_with("ws://") && !args.server.starts_with("wss://") {
        bail!("--server must be a ws:// or wss:// URL, got {}", args.server);
    }

    let config = SessionConfig {
        server: args.server.clone(),
        device_id: args.device_id.clone(),
        pairing_token: args.pairing_token.clone(),
    };
    println!("connecting {} as device {} (driver: {})", config.ws_url(), args.device_id, args.driver);

    match args.driver.as_str() {
        "echo" => run(config, EchoDriver::new).await,
        #[cfg(feature = "grpc")]
        "grpc" => {
            let addr = args.grpc_addr.clone();
            run(config, move || GrpcDriver::new(addr.clone())).await
        }
        #[cfg(not(feature = "grpc"))]
        "grpc" => bail!("built without the grpc feature"),
        other => bail!("unknown driver {other} (expected echo or grpc)"),
    }
}

async fn run<D: DeviceDriver + Clone, F: Fn() -> D>(config: SessionConfig, make_driver: F) -> ! {
    run_with_reconnect(&config, make_driver, Duration::from_secs(1), Duration::from_secs(30)).await
}
