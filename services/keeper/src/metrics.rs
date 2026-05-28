//! Prometheus collectors + a tiny axum surface exposing `/health` + `/metrics`.

use anyhow::{anyhow, Context, Result};
use axum::{routing::get, Router};
use prometheus::{
    Encoder, IntCounterVec, IntGauge, Registry, TextEncoder,
};
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
}

impl Metrics {
    pub fn new() -> Result<Self> {
        let registry = Registry::new();

        let tx_total = IntCounterVec::new(
            prometheus::Opts::new("keeper_txs_total", "Keeper transactions by kind and outcome"),
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

        registry.register(Box::new(tx_total.clone()))?;
        registry.register(Box::new(sim_failures.clone()))?;
        registry.register(Box::new(jobs_planned.clone()))?;
        registry.register(Box::new(tick_failed.clone()))?;
        registry.register(Box::new(account_nonce.clone()))?;
        registry.register(Box::new(pools_listed.clone()))?;

        Ok(Self {
            registry,
            tx_total,
            sim_failures,
            jobs_planned,
            tick_failed,
            account_nonce,
            pools_listed,
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
        .route("/metrics", get({
            let metrics = metrics.clone();
            move || {
                let metrics = metrics.clone();
                async move {
                    let mut buf = Vec::new();
                    let encoder = TextEncoder::new();
                    let families = metrics.registry.gather();
                    encoder
                        .encode(&families, &mut buf)
                        .unwrap_or_else(|e| {
                            tracing::warn!(target: "keeper.metrics", error = ?e, "encode metrics failed");
                        });
                    String::from_utf8(buf).unwrap_or_default()
                }
            }
        }));

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
