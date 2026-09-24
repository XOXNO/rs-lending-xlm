use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use keeper_bot::{
    config::{KeeperConfig, ScheduleConfig},
    discovery::{assert_update_indexes_simulation, self_check},
    metrics::{serve as serve_metrics, Metrics},
    scheduler::run as run_scheduler,
    signer::{signer_from_mnemonic, vault::load_signer, Ed25519Signer},
    stellar::client::RpcClient,
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
    #[arg(
        short,
        long,
        env = "KEEPER_CONFIG",
        default_value = "/etc/keeper/testnet.yaml"
    )]
    config: PathBuf,

    #[arg(long, env = "KEEPER_DRY_RUN", default_value_t = false)]
    dry_run: bool,

    #[arg(long, env = "KEEPER_MNEMONIC", hide_env_values = true)]
    mnemonic: Option<String>,

    #[arg(long, env = "KEEPER_SKIP_ROLE_CHECK", default_value_t = false)]
    skip_role_check: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let mut cfg = KeeperConfig::load(&args.config)
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
    let metrics = Arc::new(Metrics::new(&cfg.network)?);
    let cancel = CancellationToken::new();

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
    } else {
        let preflight = assert_update_indexes_simulation(
            client.as_ref(),
            &cfg.contracts.controller,
            &signer_pk,
        )
        .await;
        disable_index_refresh_on_failed_preflight(&mut cfg.schedule, preflight);
    }

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

/// Turns index refresh off when its boot simulation failed. The TTL loop starts
/// either way.
fn disable_index_refresh_on_failed_preflight(schedule: &mut ScheduleConfig, preflight: Result<()>) {
    match preflight {
        Ok(()) => info!(target: "keeper.boot", "update_indexes simulation passed"),
        Err(e) => {
            error!(
                target: "keeper.boot",
                error = ?e,
                "update_indexes simulation failed; index refresh disabled, TTL keepalive continues"
            );
            schedule.enable_index_refresh = false;
        }
    }
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

const DEFAULT_LOG_FILTER: &str = "info,keeper=debug";

/// Returns the log filter directive: `RUST_LOG` when it is set and valid, else
/// `log.level` from the config, else `DEFAULT_LOG_FILTER`. An invalid value
/// never fails startup.
fn log_filter_directive(rust_log: Option<&str>, level: &str) -> String {
    for candidate in [rust_log.unwrap_or("").trim(), level.trim()] {
        if !candidate.is_empty() && EnvFilter::try_new(candidate).is_ok() {
            return candidate.to_string();
        }
    }
    DEFAULT_LOG_FILTER.to_string()
}

fn init_tracing(level: &str, format: &str) -> Result<()> {
    let directive = log_filter_directive(std::env::var("RUST_LOG").ok().as_deref(), level);
    let filter =
        EnvFilter::try_new(&directive).unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    fn schedule(enable_index_refresh: bool) -> ScheduleConfig {
        ScheduleConfig {
            ttl_tick_seconds: 21_600,
            index_tick_seconds: 3_600,
            ttl_safety_margin_days: 21,
            asset_chunk: 20,
            max_txs_per_tick: 80,
            enable_index_refresh,
            scan_users: true,
            max_accounts_scan: 50_000,
        }
    }

    /// A failed `update_indexes` preflight turns the index loop off instead of
    /// stopping the process, so the TTL loop still starts.
    #[test]
    fn a_failed_index_preflight_disables_index_refresh_only() {
        let mut failed = schedule(true);
        disable_index_refresh_on_failed_preflight(
            &mut failed,
            Err(anyhow!("HostError: Error(Auth, InvalidAction)")),
        );
        assert!(!failed.enable_index_refresh);

        let mut passed = schedule(true);
        disable_index_refresh_on_failed_preflight(&mut passed, Ok(()));
        assert!(passed.enable_index_refresh);
    }

    #[test]
    fn rust_log_overrides_the_config_level() {
        assert_eq!(log_filter_directive(Some("warn"), "info"), "warn");
    }

    #[test]
    fn config_level_applies_when_rust_log_is_absent_or_blank() {
        assert_eq!(log_filter_directive(None, "debug"), "debug");
        assert_eq!(log_filter_directive(Some("   "), "debug"), "debug");
    }

    #[test]
    fn unusable_values_fall_back_instead_of_failing_startup() {
        // A malformed RUST_LOG must not shadow a good config level.
        assert_eq!(
            log_filter_directive(Some("!!not a filter!!"), "info"),
            "info"
        );
        assert_eq!(
            log_filter_directive(Some("!!bad!!"), "!!also bad!!"),
            DEFAULT_LOG_FILTER
        );
    }
}
