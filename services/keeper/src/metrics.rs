//! Prometheus metrics and health endpoint.

use anyhow::{anyhow, Context, Result};
use axum::{extract::State, http::StatusCode, routing::get, Router};
use prometheus::{Encoder, IntCounterVec, IntGauge, Registry, TextEncoder};
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
    pub account_nonce: IntGauge,
    pub pools_listed: IntGauge,
    pub entries_archived: IntGauge,
}

impl Metrics {
    pub fn new() -> Result<Self> {
        let registry = Registry::new();

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
        let account_nonce = IntGauge::with_opts(prometheus::Opts::new(
            "keeper_account_nonce",
            "Last observed AccountNonce on the controller",
        ))?;
        let pools_listed = IntGauge::with_opts(prometheus::Opts::new(
            "keeper_pools_listed",
            "Number of assets in the controller's PoolsList",
        ))?;
        let entries_archived = IntGauge::with_opts(prometheus::Opts::new(
            "keeper_entries_archived",
            "Discovered keep-alive entries currently archived (awaiting restore)",
        ))?;

        registry.register(Box::new(tx_total.clone()))?;
        registry.register(Box::new(sim_failures.clone()))?;
        registry.register(Box::new(jobs_planned.clone()))?;
        registry.register(Box::new(tick_failed.clone()))?;
        registry.register(Box::new(account_nonce.clone()))?;
        registry.register(Box::new(pools_listed.clone()))?;
        registry.register(Box::new(entries_archived.clone()))?;

        Ok(Self {
            registry,
            tx_total,
            sim_failures,
            jobs_planned,
            tick_failed,
            account_nonce,
            pools_listed,
            entries_archived,
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

/// Encodes the Prometheus registry.
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
