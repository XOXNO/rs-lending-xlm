//! Lending metrics exporter entrypoint: load config, connect RPC, serve
//! `/metrics`, and scrape the protocol on a timer until shutdown.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::time::{interval, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use lending_exporter::collector::scrape_once;
use lending_exporter::config::ExporterConfig;
use lending_exporter::metrics::{self, Metrics};
use lending_exporter::stellar::RpcClient;

#[derive(Debug, Parser)]
#[command(name = "lending-exporter", about = "Read-only Prometheus exporter for XOXNO Lending")]
struct Args {
    /// Path to the per-network YAML config.
    #[arg(long, env = "EXPORTER_CONFIG", default_value = "/etc/lending-exporter/testnet.yaml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let cfg = ExporterConfig::load(&args.config)
        .with_context(|| format!("load config {}", args.config.display()))?;

    init_tracing(&cfg.log.level, &cfg.log.format);
    info!(target: "exporter", network = %cfg.network, "starting lending exporter");

    let contracts = cfg.resolve().context("resolve contract addresses")?;
    let client = Arc::new(RpcClient::new(&cfg.rpc).context("build RPC client")?);
    let metrics = Arc::new(Metrics::new().context("build metrics registry")?);
    let cancel = CancellationToken::new();

    let metrics_task = tokio::spawn(metrics::serve(cfg.metrics.bind, metrics.clone(), cancel.clone()));

    let scrape_task = {
        let client = client.clone();
        let metrics = metrics.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            run_scrape_loop(&client, &metrics, &cfg, &contracts, cancel).await;
        })
    };

    wait_for_shutdown().await;
    info!(target: "exporter", "shutdown signal received; draining");
    cancel.cancel();

    let _ = tokio::time::timeout(Duration::from_secs(10), async {
        let _ = scrape_task.await;
        let _ = metrics_task.await;
    })
    .await;

    info!(target: "exporter", "stopped");
    Ok(())
}

async fn run_scrape_loop(
    client: &RpcClient,
    metrics: &Metrics,
    cfg: &ExporterConfig,
    contracts: &lending_exporter::config::ResolvedContracts,
    cancel: CancellationToken,
) {
    let mut tick = interval(Duration::from_secs(cfg.scrape_interval_seconds.max(1)));
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!(target: "exporter.collector", "scrape loop cancelled");
                return;
            }
            _ = tick.tick() => {
                scrape_once(client, metrics, cfg, contracts).await;
            }
        }
    }
}

fn init_tracing(level: &str, format: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    let registry = tracing_subscriber::registry().with(filter);
    if format.eq_ignore_ascii_case("json") {
        registry.with(fmt::layer().json()).init();
    } else {
        registry.with(fmt::layer()).init();
    }
}

async fn wait_for_shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                error!(target: "exporter", error = %e, "failed to install SIGTERM handler; falling back to Ctrl-C");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
