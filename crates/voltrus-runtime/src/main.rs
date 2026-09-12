//! CLI shell for the voltrus-runtime library: parse flags, boot, serve.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use mt_grpc::pb::plc_runtime_server::PlcRuntimeServer;
use voltrus_runtime::{bridge::VariableCache, SparkplugSettings};

#[derive(Parser)]
#[command(
    name = "voltrus-runtime",
    about = "Standalone Voltrus PLC runtime: VLT1 bundle in, gRPC PlcRuntime service (+ optional Sparkplug B) out"
)]
struct Cli {
    /// VLT1 bytecode bundle to deploy at boot.
    #[arg(long)]
    bundle: PathBuf,

    /// Scan cycle in milliseconds (RuntimeConfig.scan_cycle_ms).
    #[arg(long, default_value_t = 10)]
    scan_ms: u64,

    /// Listen address for the gRPC PlcRuntime service.
    #[arg(long, default_value = "0.0.0.0:50051")]
    listen: String,

    /// Sparkplug B broker address, `tcp://host:port`. Omit all Sparkplug
    /// flags to run without MQTT.
    #[arg(long)]
    sparkplug_broker: Option<String>,

    /// Sparkplug group id.
    #[arg(long, requires = "sparkplug_broker")]
    sparkplug_group: Option<String>,

    /// Sparkplug edge node id.
    #[arg(long, requires = "sparkplug_broker")]
    sparkplug_node: Option<String>,

    /// File persisting the Sparkplug bdSeq counter across restarts.
    #[arg(long, default_value = "/var/lib/voltrus/bd_seq")]
    sparkplug_bdseq: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let bundle_bytes = std::fs::read(&cli.bundle)
        .with_context(|| format!("reading bundle {}", cli.bundle.display()))?;
    let bundle = voltrus_runtime::bundle::load_vlt1(&bundle_bytes)
        .with_context(|| format!("parsing VLT1 bundle {}", cli.bundle.display()))?;
    let variable_names: Vec<String> = bundle
        .module
        .variable_slots
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    let io_count = bundle.io_mappings.len();

    let server = voltrus_runtime::boot(bundle.module, cli.scan_ms)
        .await
        .context("booting the runtime")?;
    tracing::info!(
        "deployed {} ({} variables, {} I/O mappings, scan {} ms)",
        cli.bundle.display(),
        variable_names.len(),
        io_count,
        cli.scan_ms
    );

    // Sparkplug publishing is optional: all three identity flags together,
    // or none. The publisher samples every VM variable of the boot bundle.
    let sparkplug = match (
        cli.sparkplug_broker,
        cli.sparkplug_group,
        cli.sparkplug_node,
    ) {
        (Some(broker), Some(group), Some(node)) => {
            let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            let cache = Arc::new(VariableCache::new(variable_names));
            let settings = SparkplugSettings {
                broker_url: broker,
                group_id: group,
                edge_node_id: node,
                bd_seq_path: cli.sparkplug_bdseq.clone(),
                scan_cycle_ms: cli.scan_ms,
            };
            let handle = tokio::spawn(voltrus_runtime::run_sparkplug(
                server.clone(),
                cache,
                settings,
                shutdown_rx,
            ));
            Some((handle, shutdown_tx))
        }
        (None, None, None) => None,
        _ => unreachable!("clap requires the group/node flags to accompany --sparkplug-broker"),
    };
    if sparkplug.is_none() {
        tracing::info!("Sparkplug publishing disabled (no --sparkplug-broker)");
    }

    voltrus_runtime::start_scan(&server).await?;

    let addr: std::net::SocketAddr = cli.listen.parse().context("parsing --listen")?;
    tracing::info!("serving gRPC PlcRuntime on {addr}");
    tonic::transport::Server::builder()
        .add_service(PlcRuntimeServer::from_arc(server.clone()))
        .serve_with_shutdown(addr, async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutdown signal received");
        })
        .await
        .context("gRPC server")?;

    if let Some((handle, shutdown_tx)) = sparkplug {
        let _ = shutdown_tx.send(true);
        let _ = handle.await;
    }
    Ok(())
}
