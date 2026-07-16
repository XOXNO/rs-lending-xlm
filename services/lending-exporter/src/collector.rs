//! One scrape cycle: read pool/controller/oracle views over RPC and set gauges.
//!
//! Every read is isolated — a revert or RPC error on one market/asset/spoke sets
//! a failure counter and leaves the rest of the board live. The controller's
//! bulk index view traps the whole batch on any bad key, so on batch failure we
//! retry each key alone (mirroring api-v2's `simulateMarketIndexesResilient`).

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use stellar_xdr::curr::{LedgerEntryData, ScVal};
use tracing::{debug, warn};

use crate::config::{ExporterConfig, ResolvedContracts, ResolvedMarket};
use crate::contract::{controller, oracle, pool};
use crate::keys::{asset_oracle_ledger_key, hub_asset_key_sc_val, hub_asset_vec_sc_val, HubAssetKey};
use crate::metrics::Metrics;
use crate::model;
use crate::scval;
use crate::stellar::{simulate_view, RpcClient, ViewError};

/// Resolves the central pool contract id via `controller.get_pool_address()`.
pub async fn resolve_pool_id(client: &RpcClient, controller: &[u8; 32]) -> Result<[u8; 32]> {
    let scv = simulate_view(client, controller, "get_pool_address", vec![])
        .await
        .map_err(|e| anyhow!("get_pool_address: {e}"))?;
    scval::as_contract_id(&scv).ok_or_else(|| anyhow!("get_pool_address did not return a contract address"))
}

/// Runs one full scrape cycle. Never returns an error: partial failures are
/// counted and the successful reads are still published.
pub async fn scrape_once(
    client: &RpcClient,
    metrics: &Metrics,
    cfg: &ExporterConfig,
    contracts: &ResolvedContracts,
) {
    let net = cfg.network.as_str();
    let started = Instant::now();

    let now_secs = read_ledger_now(client, metrics, net).await;
    let index_rows = read_market_indexes(client, metrics, net, contracts).await;

    // The pool getters live on the central pool; resolve it each cycle so a
    // transient RPC failure self-heals instead of crash-looping the container.
    let pool_id = match resolve_pool_id(client, &contracts.controller).await {
        Ok(p) => Some(p),
        Err(e) => {
            warn!(target: "exporter.collector", error = %e, "pool address unresolved; skipping pool getters this cycle");
            metrics.rpc_errors.with_label_values(&[net, "get_pool_address"]).inc();
            None
        }
    };

    let mut agg = Aggregates::default();
    // Per-market token decimals captured for the spoke pass (cap/usage denomination).
    let mut decimals: Vec<Option<u32>> = vec![None; contracts.markets.len()];

    for (i, (market, row)) in contracts.markets.iter().zip(index_rows.iter()).enumerate() {
        let hub_name = cfg.hub_name(market.hub_id);
        let labels = market_labels(net, market, &hub_name);
        let lref: Vec<&str> = labels.iter().map(String::as_str).collect();
        let price_wad = row.as_ref().map(|r| r.final_price_wad).unwrap_or(0);

        publish_oracle_prices(metrics, net, market, row);
        if let Some(r) = row {
            metrics.market_supply_index_ray.with_label_values(&lref).set(model::ray_to_f64(r.supply_index_ray));
            metrics.market_borrow_index_ray.with_label_values(&lref).set(model::ray_to_f64(r.borrow_index_ray));
        }

        // Pool-side amounts/params need the pool id and the token decimals.
        if let Some(pool) = &pool_id {
            if let Some(sync) = read_sync_data(client, metrics, net, pool, market).await {
                let dec = sync.params.asset_decimals;
                decimals[i] = Some(dec);
                publish_market_params(metrics, &lref, &sync);
                metrics.market_last_accrual_timestamp.with_label_values(&lref).set(sync.last_timestamp as f64);
                publish_market_amounts(client, metrics, net, pool, market, &lref, dec, price_wad, &mut agg).await;
            }
        }

        publish_oracle_staleness(client, metrics, net, market, now_secs, contracts).await;
    }

    if let Some(v) = read_view_i128(client, metrics, net, "get_min_borrow_collateral_usd", "*", &contracts.controller, "get_min_borrow_collateral_usd", vec![]).await {
        metrics.min_borrow_collateral_usd.with_label_values(&[net]).set(model::wad_to_f64(v));
    }

    publish_spokes(client, metrics, net, cfg, contracts, &index_rows, &decimals).await;

    metrics.protocol_tvl_usd.with_label_values(&[net]).set(agg.supplied_usd);
    metrics.protocol_borrowed_usd.with_label_values(&[net]).set(agg.borrowed_usd);
    metrics.protocol_liquidity_usd.with_label_values(&[net]).set(agg.liquidity_usd);
    metrics.protocol_revenue_usd.with_label_values(&[net]).set(agg.revenue_usd);
    metrics.protocol_markets.with_label_values(&[net]).set(contracts.markets.len() as f64);
    metrics.protocol_spokes.with_label_values(&[net]).set(cfg.spokes.len() as f64);
    metrics.build_info.with_label_values(&[net, env!("CARGO_PKG_VERSION")]).set(1.0);

    metrics.scrape_duration_seconds.with_label_values(&[net]).set(started.elapsed().as_secs_f64());
    metrics.last_success_timestamp.with_label_values(&[net]).set(wall_clock_secs() as f64);
}

#[derive(Default)]
struct Aggregates {
    supplied_usd: f64,
    borrowed_usd: f64,
    liquidity_usd: f64,
    revenue_usd: f64,
}

fn market_labels(net: &str, market: &ResolvedMarket, hub_name: &str) -> [String; 5] {
    [
        net.to_string(),
        market.hub_id.to_string(),
        hub_name.to_string(),
        market.asset_strkey.clone(),
        market.symbol.clone(),
    ]
}

async fn read_ledger_now(client: &RpcClient, metrics: &Metrics, net: &str) -> i64 {
    let wall = wall_clock_secs();
    if let Ok(seq) = client.latest_ledger().await {
        metrics.ledger_sequence.with_label_values(&[net]).set(seq as f64);
    }
    match client.latest_close_time().await {
        Ok(close) => {
            metrics.ledger_timestamp.with_label_values(&[net]).set(close as f64);
            metrics.ledger_skew_seconds.with_label_values(&[net]).set((close - wall) as f64);
            close
        }
        Err(e) => {
            warn!(target: "exporter.collector", error = %e, "ledger close-time read failed; using wall clock");
            metrics.rpc_errors.with_label_values(&[net, "latest_close_time"]).inc();
            metrics.ledger_timestamp.with_label_values(&[net]).set(wall as f64);
            wall
        }
    }
}

/// Reads the bulk index view, retrying per-key on batch failure so one bad feed
/// cannot zero the whole board. Returns one slot per market (index-aligned).
async fn read_market_indexes(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    contracts: &ResolvedContracts,
) -> Vec<Option<controller::MarketIndexView>> {
    let keys: Vec<HubAssetKey> = contracts
        .markets
        .iter()
        .map(|m| HubAssetKey { hub_id: m.hub_id, asset: m.asset_id })
        .collect();
    if keys.is_empty() {
        return Vec::new();
    }

    if let Some(rows) = try_index_batch(client, &contracts.controller, &keys).await {
        if rows.len() == keys.len() {
            return rows.into_iter().map(Some).collect();
        }
    }

    let mut out = Vec::with_capacity(keys.len());
    for (market, key) in contracts.markets.iter().zip(keys.iter()) {
        let single = try_index_batch(client, &contracts.controller, std::slice::from_ref(key)).await;
        match single.and_then(|mut v| v.pop()) {
            Some(row) => out.push(Some(row)),
            None => {
                metrics
                    .view_failures
                    .with_label_values(&[net, "get_market_indexes_detailed", &market.asset_strkey, "batch_key"])
                    .inc();
                out.push(None);
            }
        }
    }
    out
}

async fn try_index_batch(
    client: &RpcClient,
    controller: &[u8; 32],
    keys: &[HubAssetKey],
) -> Option<Vec<controller::MarketIndexView>> {
    let arg = hub_asset_vec_sc_val(keys).ok()?;
    match simulate_view(client, controller, "get_market_indexes_detailed", vec![arg]).await {
        Ok(scv) => controller::decode_market_indexes(&scv).ok(),
        Err(_) => None,
    }
}

fn publish_oracle_prices(
    metrics: &Metrics,
    net: &str,
    market: &ResolvedMarket,
    row: &Option<controller::MarketIndexView>,
) {
    let olabels = [net, market.asset_strkey.as_str(), market.symbol.as_str()];
    match row {
        Some(r) => {
            metrics.oracle_price_usd.with_label_values(&olabels).set(model::wad_to_f64(r.final_price_wad));
            metrics.oracle_primary_price_usd.with_label_values(&olabels).set(model::wad_to_f64(r.primary_price_wad));
            metrics.oracle_anchor_price_usd.with_label_values(&olabels).set(model::wad_to_f64(r.anchor_price_wad));
            if let Some(dev) = model::deviation_bps(r.primary_price_wad, r.anchor_price_wad) {
                metrics.oracle_deviation_bps.with_label_values(&olabels).set(dev);
            }
            metrics.oracle_healthy.with_label_values(&olabels).set(1.0);
        }
        None => {
            metrics.oracle_healthy.with_label_values(&olabels).set(0.0);
        }
    }
}

fn publish_market_params(metrics: &Metrics, lref: &[&str], sync: &pool::MarketSync) {
    let p = &sync.params;
    let set = |param: &str, value: f64| {
        let mut labels = lref.to_vec();
        labels.push(param);
        metrics.market_param.with_label_values(&labels).set(value);
    };
    set("base_borrow_rate", model::ray_to_f64(p.base_borrow_rate_ray));
    set("max_borrow_rate", model::ray_to_f64(p.max_borrow_rate_ray));
    set("slope1", model::ray_to_f64(p.slope1_ray));
    set("slope2", model::ray_to_f64(p.slope2_ray));
    set("slope3", model::ray_to_f64(p.slope3_ray));
    set("mid_utilization", model::ray_to_f64(p.mid_utilization_ray));
    set("optimal_utilization", model::ray_to_f64(p.optimal_utilization_ray));
    set("max_utilization", model::ray_to_f64(p.max_utilization_ray));
    set("reserve_factor_bps", model::bps_to_ratio(p.reserve_factor_bps));
    set("flashloan_fee_bps", model::bps_to_ratio(p.flashloan_fee_bps));
    set("is_flashloanable", if p.is_flashloanable { 1.0 } else { 0.0 });
}

#[allow(clippy::too_many_arguments)]
async fn publish_market_amounts(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    pool_id: &[u8; 32],
    market: &ResolvedMarket,
    lref: &[&str],
    dec: u32,
    price_wad: i128,
    agg: &mut Aggregates,
) {
    if let Some(v) = read_market_scalar(client, metrics, net, pool_id, market, "get_supplied_amount").await {
        let usd = model::token_usd(v, dec, price_wad);
        metrics.market_supplied.with_label_values(lref).set(model::token_to_f64(v, dec));
        metrics.market_supplied_usd.with_label_values(lref).set(usd);
        agg.supplied_usd += usd;
    }
    if let Some(v) = read_market_scalar(client, metrics, net, pool_id, market, "get_borrowed_amount").await {
        let usd = model::token_usd(v, dec, price_wad);
        metrics.market_borrowed.with_label_values(lref).set(model::token_to_f64(v, dec));
        metrics.market_borrowed_usd.with_label_values(lref).set(usd);
        agg.borrowed_usd += usd;
    }
    if let Some(v) = read_market_scalar(client, metrics, net, pool_id, market, "get_reserves").await {
        let usd = model::token_usd(v, dec, price_wad);
        metrics.market_liquidity.with_label_values(lref).set(model::token_to_f64(v, dec));
        metrics.market_liquidity_usd.with_label_values(lref).set(usd);
        agg.liquidity_usd += usd;
    }
    if let Some(v) = read_market_scalar(client, metrics, net, pool_id, market, "get_revenue").await {
        let usd = model::token_usd(v, dec, price_wad);
        metrics.market_revenue.with_label_values(lref).set(model::token_to_f64(v, dec));
        metrics.market_revenue_usd.with_label_values(lref).set(usd);
        agg.revenue_usd += usd;
    }
    if let Some(v) = read_market_scalar(client, metrics, net, pool_id, market, "get_utilisation").await {
        metrics.market_utilization.with_label_values(lref).set(model::ray_to_f64(v));
    }
    if let Some(v) = read_market_scalar(client, metrics, net, pool_id, market, "get_deposit_rate").await {
        metrics.market_supply_apy.with_label_values(lref).set(model::apy_from_per_ms_ray(v));
    }
    if let Some(v) = read_market_scalar(client, metrics, net, pool_id, market, "get_borrow_rate").await {
        metrics.market_borrow_apy.with_label_values(lref).set(model::apy_from_per_ms_ray(v));
    }
}

async fn read_sync_data(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    pool_id: &[u8; 32],
    market: &ResolvedMarket,
) -> Option<pool::MarketSync> {
    let key = HubAssetKey { hub_id: market.hub_id, asset: market.asset_id };
    let arg = hub_asset_key_sc_val(&key).ok()?;
    let scv = read_view(client, metrics, net, "get_sync_data", &market.asset_strkey, pool_id, "get_sync_data", vec![arg], true).await?;
    pool::decode_sync_data(&scv)
        .map_err(|e| debug!(target: "exporter.collector", asset = %market.asset_strkey, error = %e, "decode sync_data failed"))
        .ok()
}

async fn read_market_scalar(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    pool_id: &[u8; 32],
    market: &ResolvedMarket,
    function: &str,
) -> Option<i128> {
    let key = HubAssetKey { hub_id: market.hub_id, asset: market.asset_id };
    let arg = hub_asset_key_sc_val(&key).ok()?;
    read_view_i128(client, metrics, net, function, &market.asset_strkey, pool_id, function, vec![arg]).await
}

#[allow(clippy::too_many_arguments)]
async fn read_view_i128(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    view_label: &str,
    asset_label: &str,
    contract: &[u8; 32],
    function: &str,
    args: Vec<ScVal>,
) -> Option<i128> {
    let scv = read_view(client, metrics, net, view_label, asset_label, contract, function, args, true).await?;
    pool::decode_i128(&scv).ok()
}

#[allow(clippy::too_many_arguments)]
async fn read_view(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    view_label: &str,
    asset_label: &str,
    contract: &[u8; 32],
    function: &str,
    args: Vec<ScVal>,
    count_reverts: bool,
) -> Option<ScVal> {
    match simulate_view(client, contract, function, args).await {
        Ok(scv) => Some(scv),
        Err(ViewError::Reverted(msg)) => {
            if count_reverts {
                let code = bucket_error_code(&msg);
                metrics.view_failures.with_label_values(&[net, view_label, asset_label, &code]).inc();
                debug!(target: "exporter.collector", view = view_label, asset = asset_label, error = %msg, "view reverted");
            }
            None
        }
        Err(ViewError::NoResult) => {
            metrics.view_failures.with_label_values(&[net, view_label, asset_label, "no_result"]).inc();
            None
        }
        Err(ViewError::Rpc(e)) => {
            metrics.rpc_errors.with_label_values(&[net, function]).inc();
            debug!(target: "exporter.collector", view = view_label, error = %e, "view rpc error");
            None
        }
    }
}

async fn publish_oracle_staleness(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    market: &ResolvedMarket,
    now_secs: i64,
    contracts: &ResolvedContracts,
) {
    let olabels = [net, market.asset_strkey.as_str(), market.symbol.as_str()];
    let Some(config) = read_oracle_config(client, metrics, net, market, contracts).await else {
        return;
    };

    metrics.oracle_max_stale_seconds.with_label_values(&olabels).set(config.max_price_stale_seconds as f64);
    metrics.oracle_tolerance_upper_bps.with_label_values(&olabels).set(config.tolerance_upper_bps as f64);
    metrics.oracle_tolerance_lower_bps.with_label_values(&olabels).set(config.tolerance_lower_bps as f64);
    metrics.oracle_sanity_min_usd.with_label_values(&olabels).set(model::wad_to_f64(config.min_sanity_price_wad));
    metrics.oracle_sanity_max_usd.with_label_values(&olabels).set(model::wad_to_f64(config.max_sanity_price_wad));
    metrics.oracle_strategy.with_label_values(&olabels).set(config.strategy as f64);

    // Poll each source; report the source that goes stale first (min headroom).
    let mut sources = vec![&config.primary];
    if let Some(anchor) = &config.anchor {
        sources.push(anchor);
    }
    let mut worst: Option<(f64, u64)> = None;
    for source in sources {
        if let Some(feed_ts) = read_feed_timestamp(client, metrics, net, market, source).await {
            let sut = model::seconds_until_stale(now_secs, feed_ts, source.max_stale_seconds);
            if worst.map(|(w, _)| sut < w).unwrap_or(true) {
                worst = Some((sut, feed_ts));
            }
        }
    }
    if let Some((sut, feed_ts)) = worst {
        metrics.oracle_price_timestamp.with_label_values(&olabels).set(feed_ts as f64);
        metrics.oracle_seconds_until_stale.with_label_values(&olabels).set(sut);
    }
}

async fn read_oracle_config(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    market: &ResolvedMarket,
    contracts: &ResolvedContracts,
) -> Option<oracle::OracleConfig> {
    let key = asset_oracle_ledger_key(&contracts.controller, &market.asset_id).ok()?;
    let entries = match client.get_ledger_entries(std::slice::from_ref(&key)).await {
        Ok(e) => e,
        Err(e) => {
            metrics.rpc_errors.with_label_values(&[net, "get_ledger_entries"]).inc();
            debug!(target: "exporter.collector", asset = %market.asset_strkey, error = %e, "oracle config read failed");
            return None;
        }
    };
    let value = entries.into_iter().next()?.value?;
    let LedgerEntryData::ContractData(cd) = value else {
        return None;
    };
    oracle::decode_oracle_config(&cd.val)
        .map_err(|e| debug!(target: "exporter.collector", asset = %market.asset_strkey, error = %e, "decode oracle config failed"))
        .ok()
}

async fn read_feed_timestamp(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    market: &ResolvedMarket,
    source: &oracle::OracleSource,
) -> Option<u64> {
    let contract = crate::keys::contract_id_from_strkey(&source.contract).ok()?;
    match source.kind {
        oracle::OracleKind::Reflector => {
            let asset_ref = source.asset_ref.as_ref()?;
            let arg = oracle::oracle_asset_ref_to_reflector_arg(asset_ref).ok()?;
            let scv = read_view(client, metrics, net, "lastprice", &market.asset_strkey, &contract, "lastprice", vec![arg], true).await?;
            oracle::decode_reflector_price(&scv).ok().flatten().map(|o| o.feed_ts_secs)
        }
        oracle::OracleKind::RedStone | oracle::OracleKind::Xoxno => {
            let feed = source.feed_id.as_ref()?;
            let arg = oracle::feed_id_arg(feed).ok()?;
            let scv = read_view(client, metrics, net, "read_price_data_for_feed", &market.asset_strkey, &contract, "read_price_data_for_feed", vec![arg], true).await?;
            oracle::decode_redstone_price(&scv).ok().map(|o| o.feed_ts_secs)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn publish_spokes(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    cfg: &ExporterConfig,
    contracts: &ResolvedContracts,
    index_rows: &[Option<controller::MarketIndexView>],
    decimals: &[Option<u32>],
) {
    for &spoke_id in &cfg.spokes {
        let spoke_name = cfg.spoke_name(spoke_id);
        let deprecated = read_spoke_deprecated(client, metrics, net, contracts, spoke_id).await;
        for (i, (market, row)) in contracts.markets.iter().zip(index_rows.iter()).enumerate() {
            let dec = decimals.get(i).copied().flatten();
            let hub_name = cfg.hub_name(market.hub_id);
            publish_spoke_asset(client, metrics, net, contracts, spoke_id, &spoke_name, &hub_name, market, row, dec, deprecated).await;
        }
    }
}

async fn read_spoke_deprecated(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    contracts: &ResolvedContracts,
    spoke_id: u32,
) -> bool {
    read_view(client, metrics, net, "get_spoke", "*", &contracts.controller, "get_spoke", vec![ScVal::U32(spoke_id)], true)
        .await
        .and_then(|s| controller::decode_spoke(&s).ok())
        .map(|c| c.is_deprecated)
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
async fn publish_spoke_asset(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    contracts: &ResolvedContracts,
    spoke_id: u32,
    spoke_name: &str,
    hub_name: &str,
    market: &ResolvedMarket,
    row: &Option<controller::MarketIndexView>,
    decimals: Option<u32>,
    deprecated: bool,
) {
    let key = HubAssetKey { hub_id: market.hub_id, asset: market.asset_id };
    let Ok(hub_arg) = hub_asset_key_sc_val(&key) else {
        return;
    };
    // Most (spoke, market) pairs are not listed; a revert here is expected, so
    // skip silently rather than counting it as a failure.
    let cfg_scv = match simulate_view(client, &contracts.controller, "get_spoke_asset", vec![ScVal::U32(spoke_id), hub_arg.clone()]).await {
        Ok(s) => s,
        Err(_) => return,
    };
    let Ok(cfg) = controller::decode_spoke_asset(&cfg_scv) else {
        return;
    };

    let s = spoke_id.to_string();
    let hub = market.hub_id.to_string();
    let labels = [net, s.as_str(), spoke_name, hub.as_str(), hub_name, market.asset_strkey.as_str(), market.symbol.as_str()];
    let b = |v: bool| if v { 1.0 } else { 0.0 };
    metrics.spoke_paused.with_label_values(&labels).set(b(cfg.paused));
    metrics.spoke_frozen.with_label_values(&labels).set(b(cfg.frozen));
    metrics.spoke_collateral_enabled.with_label_values(&labels).set(b(cfg.is_collateralizable));
    metrics.spoke_borrow_enabled.with_label_values(&labels).set(b(cfg.is_borrowable));
    metrics.spoke_deprecated.with_label_values(&labels).set(b(deprecated));
    metrics.spoke_ltv_bps.with_label_values(&labels).set(cfg.loan_to_value_bps as f64);
    metrics.spoke_liq_threshold_bps.with_label_values(&labels).set(cfg.liquidation_threshold_bps as f64);
    metrics.spoke_liq_bonus_bps.with_label_values(&labels).set(cfg.liquidation_bonus_bps as f64);
    metrics.spoke_liq_fees_bps.with_label_values(&labels).set(cfg.liquidation_fees_bps as f64);

    // Cap and usage denomination need the token decimals; without them (the
    // market's sync read failed this cycle) skip the token-space metrics.
    let Some(dec) = decimals else {
        return;
    };
    metrics.spoke_supply_cap.with_label_values(&labels).set(model::token_to_f64(cfg.supply_cap, dec));
    metrics.spoke_borrow_cap.with_label_values(&labels).set(model::token_to_f64(cfg.borrow_cap, dec));

    let (supply_index, borrow_index, price_wad) = match row {
        Some(r) => (r.supply_index_ray, r.borrow_index_ray, r.final_price_wad),
        None => return,
    };
    if let Ok(usage_scv) = simulate_view(client, &contracts.controller, "get_spoke_usage", vec![ScVal::U32(spoke_id), hub_arg]).await {
        if let Ok(usage) = controller::decode_spoke_usage(&usage_scv) {
            let supply_tokens = model::scaled_usage_to_token(usage.supplied_scaled_ray, supply_index);
            let borrow_tokens = model::scaled_usage_to_token(usage.borrowed_scaled_ray, borrow_index);
            let price = model::wad_to_f64(price_wad);
            metrics.spoke_supply_usage.with_label_values(&labels).set(supply_tokens);
            metrics.spoke_supply_usage_usd.with_label_values(&labels).set(supply_tokens * price);
            metrics.spoke_borrow_usage.with_label_values(&labels).set(borrow_tokens);
            metrics.spoke_borrow_usage_usd.with_label_values(&labels).set(borrow_tokens * price);
            if let Some(u) = model::cap_utilization(supply_tokens, cfg.supply_cap, dec) {
                metrics.spoke_supply_cap_utilization.with_label_values(&labels).set(u);
            }
            if let Some(u) = model::cap_utilization(borrow_tokens, cfg.borrow_cap, dec) {
                metrics.spoke_borrow_cap_utilization.with_label_values(&labels).set(u);
            }
        }
    }
}

/// Parses a bucketed contract error code out of a revert message, e.g.
/// `Error(Contract, #210)` -> `210`.
fn bucket_error_code(msg: &str) -> String {
    if let Some(pos) = msg.find('#') {
        let digits: String = msg[pos + 1..].chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            return digits;
        }
    }
    "unknown".to_string()
}

fn wall_clock_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_contract_error_code() {
        assert_eq!(bucket_error_code("HostError: Error(Contract, #210)"), "210");
        assert_eq!(bucket_error_code("no code here"), "unknown");
        assert_eq!(bucket_error_code("#30 trailing"), "30");
    }
}
