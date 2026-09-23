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

    /// Serve the browser IDE (SPA + local project-file API). Requires the
    /// `ide` cargo feature.
    #[cfg(feature = "ide")]
    #[arg(long)]
    ide: bool,

    /// Directory holding the built IDE SPA, served under /ide/.
    #[cfg(feature = "ide")]
    #[arg(long, requires = "ide")]
    ide_dir: Option<PathBuf>,

    /// Root directory of disk-backed projects (one subdirectory per project).
    #[cfg(feature = "ide")]
    #[arg(long, requires = "ide")]
    projects_dir: Option<PathBuf>,

    /// Listen address for the IDE HTTP server.
    #[cfg(feature = "ide")]
    #[arg(long, requires = "ide")]
    ide_listen: Option<String>,
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

    // Browser IDE (feature `ide`): serve the SPA + project-file API on its own
    // port; ctrl-c tears both servers down through one shutdown channel.
    #[cfg(feature = "ide")]
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    #[cfg(feature = "ide")]
    let ide_handle = if cli.ide {
        let config = voltrus_runtime::ide::IdeConfig {
            ide_dir: cli.ide_dir.clone().unwrap_or_else(|| PathBuf::from("./ide")),
            projects_dir: cli
                .projects_dir
                .clone()
                .map(|p| expand_tilde(&p))
                .unwrap_or_default(),
        };
        let ide_addr: std::net::SocketAddr = cli
            .ide_listen
            .clone()
            .unwrap_or_else(|| "0.0.0.0:8090".to_string())
            .parse()
            .context("parsing --ide-listen")?;
        let ide_listener = tokio::net::TcpListener::bind(ide_addr)
            .await
            .with_context(|| format!("binding --ide-listen {ide_addr}"))?;
        tracing::info!(
            "serving IDE on http://{ide_addr}/ide/ (projects: {})",
            config.projects_dir.display()
        );
        Some(tokio::spawn(voltrus_runtime::ide::serve(
            config,
            ide_listener,
            shutdown_rx.clone(),
        )))
    } else {
        None
    };

    #[cfg(feature = "ide")]
    let shutdown_signal = {
        let shutdown_tx = shutdown_tx;
        async move {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutdown signal received");
            let _ = shutdown_tx.send(true);
        }
    };
    #[cfg(not(feature = "ide"))]
    let shutdown_signal = async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutdown signal received");
    };

    let addr: std::net::SocketAddr = cli.listen.parse().context("parsing --listen")?;
    tracing::info!("serving gRPC PlcRuntime on {addr}");
    tonic::transport::Server::builder()
        .add_service(PlcRuntimeServer::from_arc(server.clone()))
        .serve_with_shutdown(addr, shutdown_signal)
        .await
        .context("gRPC server")?;

    #[cfg(feature = "ide")]
    if let Some(handle) = ide_handle {
        if let Ok(Err(e)) = handle.await {
            tracing::warn!("IDE server error: {e:#}");
        }
    }

    if let Some((handle, shutdown_tx)) = sparkplug {
        let _ = shutdown_tx.send(true);
        let _ = handle.await;
    }
    Ok(())
}

/// Expand a leading `~/` against `$HOME` (the runtime targets Linux boards;
/// shells don't expand arguments like `~/voltrus/projects` for us).
#[cfg(feature = "ide")]
fn expand_tilde(path: &std::path::Path) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    match text.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(rest),
            None => path.to_path_buf(),
        },
        _ => path.to_path_buf(),
    }
}
