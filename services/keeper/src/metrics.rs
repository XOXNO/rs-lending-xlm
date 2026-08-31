use anyhow::{anyhow, Context, Result};
use axum::{extract::State, http::StatusCode, routing::get, Router};
use prometheus::{Encoder, GaugeVec, IntCounterVec, IntGauge, IntGaugeVec, Registry, TextEncoder};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::info;

pub struct Metrics {
    pub registry: Registry,
    pub tx_total: IntCounterVec,
    pub sim_failures: IntCounterVec,
    pub jobs_planned: IntCounterVec,
    pub tick_failed: IntCounterVec,
    pub max_account_id: IntGauge,
    pub entries_archived: IntGauge,

    /// Lowest remaining TTL in a `(contract, group)`, in ledgers. This is the
    /// pacing item: the tick when the keeper first has to act on that group.
    pub entry_ttl_ledgers_min: IntGaugeVec,

    /// Entry counts per `(contract, group, state)`, `state` being `live` or
    /// `absent`. An all-`absent` group is how a wrong key encoding looks.
    pub entries: IntGaugeVec,

    /// Safety margin and current ledger, so a panel can draw the action
    /// threshold and convert remaining ledgers into absolute dates.
    pub safety_margin_ledgers: IntGauge,
    pub current_ledger: IntGauge,

    /// Wall-clock time of the last completed discovery tick. The TTL loop runs
    /// every `ttl_tick_seconds` (6h on mainnet), so every storage gauge can be
    /// that stale; without this a dashboard cannot tell a quiet protocol from a
    /// dead keeper.
    pub last_tick_timestamp_seconds: IntGauge,

    /// Resource fee a simulated extend or restore would cost, in stroops. This
    /// is measured from simulation, not modelled from entry size.
    pub sim_resource_fee_stroops: GaugeVec,
}

impl Metrics {
    /// `network` becomes a const label on every family. Without it, a testnet
    /// and a mainnet keeper scraped into the same Prometheus collide on
    /// identical `(contract, group)` label sets, and a `_min` gauge silently
    /// reports whichever scrape landed last.
    pub fn new(network: &str) -> Result<Self> {
        let registry = Registry::new_custom(
            None,
            Some(HashMap::from([(
                "network".to_string(),
                network.to_string(),
            )])),
        )?;

        let tx_total = IntCounterVec::new(
            prometheus::Opts::new(
                "keeper_txs_total",
                "Keeper transactions by kind and outcome",
            ),
            &["kind", "status"],
        )?;
        let sim_failures = IntCounterVec::new(
            prometheus::Opts::new(
                "keeper_sim_failures_total",
                "Simulation failures by kind and bucketed reason",
            ),
            &["kind", "reason"],
        )?;
        let jobs_planned = IntCounterVec::new(
            prometheus::Opts::new("keeper_jobs_planned_total", "Jobs planned per loop tick"),
            &["loop"],
        )?;
        let tick_failed = IntCounterVec::new(
            prometheus::Opts::new("keeper_tick_failed_total", "Tick failures per loop"),
            &["loop"],
        )?;
        let max_account_id = IntGauge::with_opts(prometheus::Opts::new(
            "keeper_max_account_id",
            "Highest position-NFT token id minted, i.e. the largest existing account id",
        ))?;
        let entries_archived = IntGauge::with_opts(prometheus::Opts::new(
            "keeper_entries_archived",
            "Discovered keep-alive entries currently archived (awaiting restore)",
        ))?;

        let entry_ttl_ledgers_min = IntGaugeVec::new(
            prometheus::Opts::new(
                "keeper_entry_ttl_ledgers_min",
                "Lowest remaining TTL in ledgers across a contract's key group",
            ),
            &["contract", "group"],
        )?;
        let entries = IntGaugeVec::new(
            prometheus::Opts::new(
                "keeper_entries",
                "Discovered entries per contract, key group and state (live/absent)",
            ),
            &["contract", "group", "state"],
        )?;
        let safety_margin_ledgers = IntGauge::with_opts(prometheus::Opts::new(
            "keeper_safety_margin_ledgers",
            "Ledgers of headroom below which the keeper extends an entry",
        ))?;
        let current_ledger = IntGauge::with_opts(prometheus::Opts::new(
            "keeper_current_ledger",
            "Ledger sequence the most recent discovery tick observed",
        ))?;
        let last_tick_timestamp_seconds = IntGauge::with_opts(prometheus::Opts::new(
            "keeper_last_tick_timestamp_seconds",
            "Unix time of the last completed discovery tick",
        ))?;
        let sim_resource_fee_stroops = GaugeVec::new(
            prometheus::Opts::new(
                "keeper_sim_resource_fee_stroops",
                "Resource fee of the last simulated job of this kind, in stroops",
            ),
            &["kind"],
        )?;

        registry.register(Box::new(entry_ttl_ledgers_min.clone()))?;
        registry.register(Box::new(entries.clone()))?;
        registry.register(Box::new(safety_margin_ledgers.clone()))?;
        registry.register(Box::new(current_ledger.clone()))?;
        registry.register(Box::new(last_tick_timestamp_seconds.clone()))?;
        registry.register(Box::new(sim_resource_fee_stroops.clone()))?;
        registry.register(Box::new(tx_total.clone()))?;
        registry.register(Box::new(sim_failures.clone()))?;
        registry.register(Box::new(jobs_planned.clone()))?;
        registry.register(Box::new(tick_failed.clone()))?;
        registry.register(Box::new(max_account_id.clone()))?;
        registry.register(Box::new(entries_archived.clone()))?;

        Ok(Self {
            registry,
            tx_total,
            sim_failures,
            jobs_planned,
            tick_failed,
            max_account_id,
            entries_archived,
            entry_ttl_ledgers_min,
            entries,
            safety_margin_ledgers,
            current_ledger,
            last_tick_timestamp_seconds,
            sim_resource_fee_stroops,
        })
    }
}

pub async fn serve(
    bind: SocketAddr,
    metrics: Arc<Metrics>,
    cancel: CancellationToken,
) -> Result<()> {
    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(scrape))
        .with_state(metrics);

    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("bind metrics listener on {bind}"))?;
    info!(target: "keeper.metrics", %bind, "metrics + /health surface online");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            cancel.cancelled().await;
        })
        .await
        .map_err(|e| anyhow!("axum serve: {e}"))
}

async fn health() -> &'static str {
    "ok\n"
}

async fn scrape(State(metrics): State<Arc<Metrics>>) -> Result<String, StatusCode> {
    let mut buf = Vec::new();
    TextEncoder::new()
        .encode(&metrics.registry.gather(), &mut buf)
        .map_err(|e| {
            tracing::error!(target: "keeper.metrics", error = ?e, "encode metrics failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    String::from_utf8(buf).map_err(|e| {
        tracing::error!(target: "keeper.metrics", error = ?e, "metrics buffer not utf-8");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}
