use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use keeper_bot::{
    config::KeeperConfig,
    discovery::{assert_keeper_role, self_check},
    metrics::{serve as serve_metrics, Metrics},
    scheduler::run as run_scheduler,
    signer::{signer_from_mnemonic, vault::load_signer, Ed25519Signer},
    stellar::RpcClient,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "keeper-bot", version, about = "XOXNO Lending off-chain TTL keeper")]
struct Args {
    /// Path to the YAML config (one of testnet.yaml / mainnet.yaml).
    #[arg(short, long, env = "KEEPER_CONFIG", default_value = "/etc/keeper/testnet.yaml")]
    config: PathBuf,

    /// Simulate every planned tx but never submit. Useful for staging.
    #[arg(long, env = "KEEPER_DRY_RUN", default_value_t = false)]
    dry_run: bool,

    /// Local-dev override: skip KeyVault and use this BIP-39 mnemonic
    /// directly. INTENDED FOR DEVELOPMENT ONLY — production must source
    /// the mnemonic from KeyVault.
    #[arg(long, env = "KEEPER_MNEMONIC", hide_env_values = true)]
    mnemonic: Option<String>,

    /// Skip the boot-time KEEPER role check. Only useful when probing the
    /// service with a throwaway signer that does not (and should not) hold
    /// the role on-chain. DEV ONLY.
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

    // Pre-flight: encoding self-check + KEEPER role gate.
    let pools = self_check(client.as_ref(), &cfg.contracts.controller).await?;
    info!(target: "keeper.boot", n_assets = pools.len(), "self-check passed");

    let signer_pk = signer.public_key_strkey();
    if args.skip_role_check {
        warn!(
            target: "keeper.boot",
            signer = %signer_pk,
            "DEV: skipping KEEPER role check (--skip-role-check)"
        );
    } else if let Err(e) =
        assert_keeper_role(client.as_ref(), &cfg.contracts.controller, &signer_pk).await
    {
        error!(target: "keeper.boot", error = ?e, "KEEPER role check failed — aborting");
        return Err(e);
    } else {
        info!(target: "keeper.boot", signer = %signer_pk, "KEEPER role check passed");
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
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        async {
            let _ = scheduler.ttl_task.await;
            let _ = scheduler.index_task.await;
            let _ = metrics_handle.await;
        },
    )
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
    let filter =
        EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info,keeper=debug"));
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
