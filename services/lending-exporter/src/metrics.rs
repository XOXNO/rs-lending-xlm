//! Prometheus registry, metric families, `/metrics` + `/health`.
//!
//! Every family has a `network` label (one Grafana, multiple scrape jobs).

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use axum::{extract::State, http::StatusCode, routing::get, Router};
use prometheus::{Encoder, GaugeVec, IntCounterVec, Opts, Registry, TextEncoder};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// One `(hub, asset)` reserve; `hub` display, `hub_id` stable.
const MARKET_LABELS: &[&str] = &["network", "hub_id", "hub", "asset", "symbol"];
const ORACLE_LABELS: &[&str] = &["network", "asset", "symbol"];
/// Spoke-level labels (liquidation curve, deprecation).
const SPOKE_LABELS: &[&str] = &["network", "spoke_id", "spoke"];
/// Spoke-asset labels; `spoke`/`hub` display, `*_id` stable.
const SPOKE_ASSET_LABELS: &[&str] = &["network", "spoke_id", "spoke", "hub_id", "hub", "asset", "symbol"];

pub struct Metrics {
    pub registry: Registry,

    pub market_supplied: GaugeVec,
    pub market_supplied_usd: GaugeVec,
    pub market_borrowed: GaugeVec,
    pub market_borrowed_usd: GaugeVec,
    pub market_liquidity: GaugeVec,
    pub market_liquidity_usd: GaugeVec,
    pub market_revenue: GaugeVec,
    pub market_revenue_usd: GaugeVec,
    pub market_utilization: GaugeVec,
    pub market_supply_apy: GaugeVec,
    pub market_borrow_apy: GaugeVec,
    pub market_supply_index_ray: GaugeVec,
    pub market_borrow_index_ray: GaugeVec,
    pub market_last_accrual_timestamp: GaugeVec,
    /// Seconds since pool `last_timestamp` (`get_delta_time`).
    pub market_delta_time_seconds: GaugeVec,
    /// IRM curve params (`param` label).
    pub market_param: GaugeVec,

    /// Final composed USD WAD (`MarketIndexView.price_wad`).
    pub oracle_price_usd: GaugeVec,
    /// Primary leg (`safe_price_wad` historical ABI name).
    pub oracle_primary_price_usd: GaugeVec,
    /// Secondary/anchor leg (`aggregator_price_wad` historical ABI name).
    pub oracle_anchor_price_usd: GaugeVec,
    pub oracle_deviation_bps: GaugeVec,
    /// Soft status `valid` (1 = usable for solvency).
    pub oracle_healthy: GaugeVec,
    /// Soft status `stale` flag (0/1).
    pub oracle_stale: GaugeVec,
    /// Soft status `deviation` flag (0/1).
    pub oracle_deviation_flag: GaugeVec,
    /// Soft status blend timestamp (`MarketIndexView.price_timestamp`).
    pub oracle_status_timestamp: GaugeVec,
    pub oracle_max_stale_seconds: GaugeVec,
    /// Max-stale of the soonest-to-stale provider leg (for freshness fraction).
    pub oracle_effective_max_stale_seconds: GaugeVec,
    pub oracle_tolerance_upper_bps: GaugeVec,
    pub oracle_tolerance_lower_bps: GaugeVec,
    pub oracle_sanity_min_usd: GaugeVec,
    pub oracle_sanity_max_usd: GaugeVec,
    pub oracle_strategy: GaugeVec,
    /// Provider-probe feed timestamp (worst leg).
    pub oracle_price_timestamp: GaugeVec,
    pub oracle_seconds_until_stale: GaugeVec,

    pub spoke_paused: GaugeVec,
    pub spoke_frozen: GaugeVec,
    pub spoke_collateral_enabled: GaugeVec,
    pub spoke_borrow_enabled: GaugeVec,
    pub spoke_deprecated: GaugeVec,
    pub spoke_liquidation_target_hf: GaugeVec,
    pub spoke_hf_for_max_bonus: GaugeVec,
    pub spoke_liquidation_bonus_factor_bps: GaugeVec,
    pub spoke_ltv_bps: GaugeVec,
    pub spoke_liq_threshold_bps: GaugeVec,
    pub spoke_liq_bonus_bps: GaugeVec,
    pub spoke_liq_fees_bps: GaugeVec,
    pub spoke_supply_cap: GaugeVec,
    pub spoke_borrow_cap: GaugeVec,
    pub spoke_supply_usage: GaugeVec,
    pub spoke_supply_usage_usd: GaugeVec,
    pub spoke_borrow_usage: GaugeVec,
    pub spoke_borrow_usage_usd: GaugeVec,
    pub spoke_supply_cap_utilization: GaugeVec,
    pub spoke_borrow_cap_utilization: GaugeVec,

    pub protocol_tvl_usd: GaugeVec,
    pub protocol_borrowed_usd: GaugeVec,
    pub protocol_liquidity_usd: GaugeVec,
    pub protocol_revenue_usd: GaugeVec,
    pub protocol_markets: GaugeVec,
    pub protocol_spokes: GaugeVec,
    pub min_borrow_collateral_usd: GaugeVec,

    pub ledger_timestamp: GaugeVec,
    pub ledger_sequence: GaugeVec,
    pub ledger_skew_seconds: GaugeVec,
    pub scrape_duration_seconds: GaugeVec,
    pub last_success_timestamp: GaugeVec,
    pub build_info: GaugeVec,
    pub rpc_errors: IntCounterVec,
    pub view_failures: IntCounterVec,
}

fn register_gauge_vec(reg: &Registry, name: &str, help: &str, labels: &[&str]) -> Result<GaugeVec> {
    let g = GaugeVec::new(Opts::new(name, help), labels)?;
    reg.register(Box::new(g.clone()))?;
    Ok(g)
}

fn register_counter_vec(reg: &Registry, name: &str, help: &str, labels: &[&str]) -> Result<IntCounterVec> {
    let c = IntCounterVec::new(Opts::new(name, help), labels)?;
    reg.register(Box::new(c.clone()))?;
    Ok(c)
}

impl Metrics {
    pub fn new() -> Result<Self> {
        let registry = Registry::new();
        Ok(Self {
            market_supplied: register_gauge_vec(&registry, "lending_market_supplied_total", "Total supplied underlying (whole tokens)", MARKET_LABELS)?,
            market_supplied_usd: register_gauge_vec(&registry, "lending_market_supplied_total_usd", "Total supplied in USD", MARKET_LABELS)?,
            market_borrowed: register_gauge_vec(&registry, "lending_market_borrowed_total", "Total borrowed underlying (whole tokens)", MARKET_LABELS)?,
            market_borrowed_usd: register_gauge_vec(&registry, "lending_market_borrowed_total_usd", "Total borrowed in USD", MARKET_LABELS)?,
            market_liquidity: register_gauge_vec(&registry, "lending_market_available_liquidity", "Available cash (whole tokens)", MARKET_LABELS)?,
            market_liquidity_usd: register_gauge_vec(&registry, "lending_market_available_liquidity_usd", "Available cash in USD", MARKET_LABELS)?,
            market_revenue: register_gauge_vec(&registry, "lending_market_revenue", "Accrued protocol revenue (whole tokens)", MARKET_LABELS)?,
            market_revenue_usd: register_gauge_vec(&registry, "lending_market_revenue_usd", "Accrued protocol revenue in USD", MARKET_LABELS)?,
            market_utilization: register_gauge_vec(&registry, "lending_market_utilization_ratio", "Capital utilization (0..1)", MARKET_LABELS)?,
            market_supply_apy: register_gauge_vec(&registry, "lending_market_supply_apy", "Supply APY (fraction, daily-compounded)", MARKET_LABELS)?,
            market_borrow_apy: register_gauge_vec(&registry, "lending_market_borrow_apy", "Borrow APY (fraction, daily-compounded)", MARKET_LABELS)?,
            market_supply_index_ray: register_gauge_vec(&registry, "lending_market_supply_index_ray", "Live supply index (RAY as ratio)", MARKET_LABELS)?,
            market_borrow_index_ray: register_gauge_vec(&registry, "lending_market_borrow_index_ray", "Live borrow index (RAY as ratio)", MARKET_LABELS)?,
            market_last_accrual_timestamp: register_gauge_vec(&registry, "lending_market_last_accrual_timestamp", "Unix seconds of last on-chain accrual checkpoint (pool ms/1000)", MARKET_LABELS)?,
            market_delta_time_seconds: register_gauge_vec(&registry, "lending_market_delta_time_seconds", "Seconds since last pool accrual checkpoint (get_delta_time ms/1000)", MARKET_LABELS)?,
            market_param: register_gauge_vec(&registry, "lending_market_param", "IRM curve params by `param` (rates/util as ratio, fees as ratio, bool as 0/1)", &["network", "hub_id", "hub", "asset", "symbol", "param"])?,

            oracle_price_usd: register_gauge_vec(&registry, "lending_oracle_price_usd", "Final blended USD price (MarketIndexView.price_wad)", ORACLE_LABELS)?,
            oracle_primary_price_usd: register_gauge_vec(&registry, "lending_oracle_primary_price_usd", "Primary oracle leg USD (safe_price_wad)", ORACLE_LABELS)?,
            oracle_anchor_price_usd: register_gauge_vec(&registry, "lending_oracle_anchor_price_usd", "Secondary/anchor oracle leg USD (aggregator_price_wad)", ORACLE_LABELS)?,
            oracle_deviation_bps: register_gauge_vec(&registry, "lending_oracle_deviation_bps", "Primary vs anchor deviation (bps)", ORACLE_LABELS)?,
            oracle_healthy: register_gauge_vec(&registry, "lending_oracle_healthy", "1 if soft status valid (fresh, in-band, in sanity)", ORACLE_LABELS)?,
            oracle_stale: register_gauge_vec(&registry, "lending_oracle_stale", "1 if soft status stale flag is set", ORACLE_LABELS)?,
            oracle_deviation_flag: register_gauge_vec(&registry, "lending_oracle_deviation_flag", "1 if soft status dual-source deviation flag is set", ORACLE_LABELS)?,
            oracle_status_timestamp: register_gauge_vec(&registry, "lending_oracle_status_timestamp_seconds", "Soft status blend price timestamp (Unix s)", ORACLE_LABELS)?,
            oracle_max_stale_seconds: register_gauge_vec(&registry, "lending_oracle_max_stale_seconds", "Configured market-level max price staleness (s)", ORACLE_LABELS)?,
            oracle_effective_max_stale_seconds: register_gauge_vec(&registry, "lending_oracle_effective_max_stale_seconds", "Max-stale of the soonest-to-stale provider leg (s)", ORACLE_LABELS)?,
            oracle_tolerance_upper_bps: register_gauge_vec(&registry, "lending_oracle_tolerance_upper_bps", "Configured upper deviation band (bps)", ORACLE_LABELS)?,
            oracle_tolerance_lower_bps: register_gauge_vec(&registry, "lending_oracle_tolerance_lower_bps", "Configured lower deviation band (bps)", ORACLE_LABELS)?,
            oracle_sanity_min_usd: register_gauge_vec(&registry, "lending_oracle_sanity_min_usd", "Configured min sanity price (USD)", ORACLE_LABELS)?,
            oracle_sanity_max_usd: register_gauge_vec(&registry, "lending_oracle_sanity_max_usd", "Configured max sanity price (USD)", ORACLE_LABELS)?,
            oracle_strategy: register_gauge_vec(&registry, "lending_oracle_strategy", "Oracle strategy (0 single, 1 primary+anchor)", ORACLE_LABELS)?,
            oracle_price_timestamp: register_gauge_vec(&registry, "lending_oracle_price_timestamp_seconds", "Provider-probe feed timestamp of worst leg (Unix s)", ORACLE_LABELS)?,
            oracle_seconds_until_stale: register_gauge_vec(&registry, "lending_oracle_seconds_until_stale", "Seconds until the worst provider leg goes stale (negative if already stale)", ORACLE_LABELS)?,

            spoke_paused: register_gauge_vec(&registry, "lending_spoke_paused", "1 if the spoke-asset is paused", SPOKE_ASSET_LABELS)?,
            spoke_frozen: register_gauge_vec(&registry, "lending_spoke_frozen", "1 if the spoke-asset is frozen", SPOKE_ASSET_LABELS)?,
            spoke_collateral_enabled: register_gauge_vec(&registry, "lending_spoke_collateral_enabled", "1 if collateral is enabled", SPOKE_ASSET_LABELS)?,
            spoke_borrow_enabled: register_gauge_vec(&registry, "lending_spoke_borrow_enabled", "1 if borrowing is enabled", SPOKE_ASSET_LABELS)?,
            spoke_deprecated: register_gauge_vec(&registry, "lending_spoke_deprecated", "1 if the owning spoke is deprecated", SPOKE_ASSET_LABELS)?,
            spoke_liquidation_target_hf: register_gauge_vec(&registry, "lending_spoke_liquidation_target_hf", "Spoke liquidation target health factor (WAD as ratio)", SPOKE_LABELS)?,
            spoke_hf_for_max_bonus: register_gauge_vec(&registry, "lending_spoke_hf_for_max_bonus", "Spoke HF at which max liquidation bonus applies (WAD as ratio)", SPOKE_LABELS)?,
            spoke_liquidation_bonus_factor_bps: register_gauge_vec(&registry, "lending_spoke_liquidation_bonus_factor_bps", "Spoke liquidation bonus factor (bps)", SPOKE_LABELS)?,
            spoke_ltv_bps: register_gauge_vec(&registry, "lending_spoke_ltv_bps", "Loan-to-value (bps)", SPOKE_ASSET_LABELS)?,
            spoke_liq_threshold_bps: register_gauge_vec(&registry, "lending_spoke_liquidation_threshold_bps", "Liquidation threshold (bps)", SPOKE_ASSET_LABELS)?,
            spoke_liq_bonus_bps: register_gauge_vec(&registry, "lending_spoke_liquidation_bonus_bps", "Liquidation bonus (bps)", SPOKE_ASSET_LABELS)?,
            spoke_liq_fees_bps: register_gauge_vec(&registry, "lending_spoke_liquidation_fees_bps", "Liquidation protocol fee (bps)", SPOKE_ASSET_LABELS)?,
            spoke_supply_cap: register_gauge_vec(&registry, "lending_spoke_supply_cap", "Supply cap (whole tokens; 0 = uncapped)", SPOKE_ASSET_LABELS)?,
            spoke_borrow_cap: register_gauge_vec(&registry, "lending_spoke_borrow_cap", "Borrow cap (whole tokens; 0 = uncapped)", SPOKE_ASSET_LABELS)?,
            spoke_supply_usage: register_gauge_vec(&registry, "lending_spoke_supply_usage", "Supply usage (whole tokens)", SPOKE_ASSET_LABELS)?,
            spoke_supply_usage_usd: register_gauge_vec(&registry, "lending_spoke_supply_usage_usd", "Supply usage in USD", SPOKE_ASSET_LABELS)?,
            spoke_borrow_usage: register_gauge_vec(&registry, "lending_spoke_borrow_usage", "Borrow usage (whole tokens)", SPOKE_ASSET_LABELS)?,
            spoke_borrow_usage_usd: register_gauge_vec(&registry, "lending_spoke_borrow_usage_usd", "Borrow usage in USD", SPOKE_ASSET_LABELS)?,
            spoke_supply_cap_utilization: register_gauge_vec(&registry, "lending_spoke_supply_cap_utilization", "Supply usage / supply cap (0..1)", SPOKE_ASSET_LABELS)?,
            spoke_borrow_cap_utilization: register_gauge_vec(&registry, "lending_spoke_borrow_cap_utilization", "Borrow usage / borrow cap (0..1)", SPOKE_ASSET_LABELS)?,

            protocol_tvl_usd: register_gauge_vec(&registry, "lending_protocol_tvl_usd", "Sum of supplied USD across markets", &["network"])?,
            protocol_borrowed_usd: register_gauge_vec(&registry, "lending_protocol_total_borrowed_usd", "Sum of borrowed USD across markets", &["network"])?,
            protocol_liquidity_usd: register_gauge_vec(&registry, "lending_protocol_total_liquidity_usd", "Sum of available cash USD across markets", &["network"])?,
            protocol_revenue_usd: register_gauge_vec(&registry, "lending_protocol_total_revenue_usd", "Sum of revenue USD across markets", &["network"])?,
            protocol_markets: register_gauge_vec(&registry, "lending_protocol_markets_count", "Number of scraped markets", &["network"])?,
            protocol_spokes: register_gauge_vec(&registry, "lending_protocol_spokes_count", "Number of scraped spokes", &["network"])?,
            min_borrow_collateral_usd: register_gauge_vec(&registry, "lending_min_borrow_collateral_usd", "Controller min LTV-weighted borrow collateral (USD)", &["network"])?,

            ledger_timestamp: register_gauge_vec(&registry, "lending_ledger_timestamp_seconds", "Latest ledger close time used as `now` (Unix s)", &["network"])?,
            ledger_sequence: register_gauge_vec(&registry, "lending_ledger_sequence", "Latest ledger sequence", &["network"])?,
            ledger_skew_seconds: register_gauge_vec(&registry, "lending_exporter_ledger_skew_seconds", "Ledger close time minus exporter wall clock (s)", &["network"])?,
            scrape_duration_seconds: register_gauge_vec(&registry, "lending_exporter_scrape_duration_seconds", "Duration of the last scrape cycle (s)", &["network"])?,
            last_success_timestamp: register_gauge_vec(&registry, "lending_exporter_last_success_timestamp", "Unix seconds of the last completed scrape", &["network"])?,
            build_info: register_gauge_vec(&registry, "lending_exporter_build_info", "Build info; value is always 1", &["network", "version"])?,
            rpc_errors: register_counter_vec(&registry, "lending_exporter_rpc_errors_total", "RPC transport errors by op", &["network", "op"])?,
            view_failures: register_counter_vec(&registry, "lending_exporter_view_failures_total", "Contract view failures by view/asset/code", &["network", "view", "asset", "code"])?,

            registry,
        })
    }
}

pub async fn serve(bind: SocketAddr, metrics: Arc<Metrics>, cancel: CancellationToken) -> Result<()> {
    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(scrape))
        .with_state(metrics);

    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("bind metrics listener on {bind}"))?;
    info!(target: "exporter.metrics", %bind, "metrics + /health surface online");

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
            tracing::error!(target: "exporter.metrics", error = ?e, "encode metrics failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    String::from_utf8(buf).map_err(|e| {
        tracing::error!(target: "exporter.metrics", error = ?e, "metrics buffer not utf-8");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}
