use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use keeper_bot::{
    config::KeeperConfig,
    discovery::{assert_update_indexes_simulation, self_check},
    metrics::{serve as serve_metrics, Metrics},
    scheduler::run as run_scheduler,
    signer::{signer_from_mnemonic, vault::load_signer, Ed25519Signer},
    stellar::RpcClient,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "keeper-bot",
    version,
    about = "XOXNO Lending off-chain TTL keeper"
)]
struct Args {
    /// YAML config path.
    #[arg(
        short,
        long,
        env = "KEEPER_CONFIG",
        default_value = "/etc/keeper/testnet.yaml"
    )]
    config: PathBuf,

    /// Simulate planned transactions without submitting.
    #[arg(long, env = "KEEPER_DRY_RUN", default_value_t = false)]
    dry_run: bool,

    /// BIP-39 mnemonic override instead of KeyVault.
    #[arg(long, env = "KEEPER_MNEMONIC", hide_env_values = true)]
    mnemonic: Option<String>,

    /// Skip the boot-time update_indexes simulation preflight.
    #[arg(long, env = "KEEPER_SKIP_ROLE_CHECK", default_value_t = false)]
    skip_role_check: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let cfg = KeeperConfig::load(&args.config)
        .with_context(|| format!("load config at {}", args.config.display()))?;
    init_tracing(&cfg.log.level, &cfg.log.format)?;

    info!(
        target: "keeper.boot",
        network = %cfg.network,
        controller = %cfg.contracts.controller,
        dry_run = args.dry_run,
        "keeper-bot starting"
    );

    let client = Arc::new(RpcClient::new(&cfg.rpc)?);
    let signer = Arc::new(resolve_signer(&args, &cfg).await?);
    let metrics = Arc::new(Metrics::new()?);
    let cancel = CancellationToken::new();

    // Pre-flight: encoding self-check + optional update_indexes simulation.
    let pools = self_check(&cfg.contracts)?;
    info!(target: "keeper.boot", n_assets = pools.len(), "self-check passed");

    let signer_pk = signer.public_key_strkey();
    if args.skip_role_check {
        warn!(
            target: "keeper.boot",
            signer = %signer_pk,
            "DEV: skipping update_indexes simulation (--skip-role-check)"
        );
    } else if !cfg.schedule.enable_index_refresh {
        info!(
            target: "keeper.boot",
            signer = %signer_pk,
            "pure-TTL mode (enable_index_refresh=false); no invoke preflight"
        );
    } else if let Err(e) =
        assert_update_indexes_simulation(client.as_ref(), &cfg.contracts.controller, &signer_pk)
            .await
    {
        error!(target: "keeper.boot", error = ?e, "update_indexes simulation failed — aborting");
        return Err(e);
    } else {
        info!(target: "keeper.boot", signer = %signer_pk, "update_indexes simulation passed");
    }

    // Spawn metrics surface.
    let metrics_handle = {
        let metrics = metrics.clone();
        let cancel = cancel.clone();
        let bind = cfg.metrics.bind;
        tokio::spawn(async move {
            if let Err(e) = serve_metrics(bind, metrics, cancel).await {
                error!(target: "keeper.metrics", error = ?e, "metrics surface stopped");
            }
        })
    };

    // Spawn scheduler loops.
    let cfg_arc = Arc::new(cfg);
    let scheduler = run_scheduler(
        cfg_arc.clone(),
        client.clone(),
        signer.clone(),
        metrics.clone(),
        cancel.clone(),
        args.dry_run,
    )
    .await?;

    wait_for_shutdown().await;
    info!(target: "keeper.boot", "shutdown signal received, cancelling loops");
    cancel.cancel();

    // Best-effort shutdown — we don't block forever on stuck loops.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let _ = scheduler.ttl_task.await;
        if let Some(index) = scheduler.index_task {
            let _ = index.await;
        }
        let _ = metrics_handle.await;
    })
    .await;

    info!(target: "keeper.boot", "stopped cleanly");
    Ok(())
}

async fn wait_for_shutdown() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            warn!(target: "keeper.boot", error = ?e, "could not install SIGTERM handler; using Ctrl-C only");
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    let mut sigint = match signal(SignalKind::interrupt()) {
        Ok(s) => s,
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = sigterm.recv() => {},
        _ = sigint.recv() => {},
    }
}

async fn resolve_signer(args: &Args, cfg: &KeeperConfig) -> Result<Ed25519Signer> {
    if let Some(mnemonic) = &args.mnemonic {
        warn!(
            target: "keeper.boot",
            "DEV: using --mnemonic override; KeyVault NOT consulted"
        );
        return signer_from_mnemonic(mnemonic, &cfg.signer.derivation_path);
    }
    load_signer(&cfg.keyvault, &cfg.signer).await
}

fn init_tracing(level: &str, format: &str) -> Result<()> {
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info,keeper=debug"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match format {
        "json" => {
            builder.json().with_current_span(false).init();
        }
        _ => {
            builder.init();
        }
    }
    Ok(())
}
