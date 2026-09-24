use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use prometheus::GaugeVec;
use stellar_xdr::{LedgerEntryData, ScVal};
use tracing::{debug, info, warn};

use crate::config::{ExporterConfig, ResolvedContracts, ResolvedMarket};
use crate::contract::{controller, oracle, pool};
use crate::keys::{
    asset_oracle_ledger_key, hub_asset_key_sc_val, hub_asset_vec_sc_val, HubAssetKey,
};
use crate::metrics::Metrics;
use crate::model;
use crate::scval;
use crate::stellar::{simulate_view, simulations_sent, RpcClient, ViewError};

/// Keys per `get_market_indexes_detailed` simulation; 3 stays under the
/// mainnet CPU budget for every hub, 5 does not.
const INDEX_CHUNK_SIZE: usize = 3;
/// How long a (spoke, hub, asset) key whose `get_spoke_asset` call reverted
/// with `ASSET_NOT_IN_SPOKE` is skipped before the next probe.
const UNLISTED_SPOKE_ASSET_TTL: Duration = Duration::from_secs(10 * 60);
/// `SpokeError::AssetNotInSpoke`: `get_spoke_asset` on a pair that is not listed.
const ASSET_NOT_IN_SPOKE: &str = "307";

type UnlistedSpokeAssets = HashMap<(u32, u32, [u8; 32]), Instant>;

fn unlisted_spoke_assets() -> &'static Mutex<UnlistedSpokeAssets> {
    static CACHE: OnceLock<Mutex<UnlistedSpokeAssets>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub async fn resolve_pool_id(client: &RpcClient, controller: &[u8; 32]) -> Result<[u8; 32]> {
    let scv = simulate_view(client, controller, "get_pool_address", vec![])
        .await
        .map_err(|e| anyhow!("get_pool_address: {e}"))?;
    scval::as_contract_id(&scv)
        .ok_or_else(|| anyhow!("get_pool_address did not return a contract address"))
}

pub async fn resolve_price_aggregator(
    client: &RpcClient,
    controller: &[u8; 32],
    config_fallback: Option<&[u8; 32]>,
) -> Option<[u8; 32]> {
    match simulate_view(client, controller, "price_aggregator", vec![]).await {
        Ok(scv) => {
            if let Some(id) = scval::as_contract_id(&scv) {
                return Some(id);
            }
            debug!(target: "exporter.collector", "price_aggregator view returned non-contract");
        }
        Err(e) => {
            debug!(target: "exporter.collector", error = %e, "price_aggregator view failed; trying config fallback");
        }
    }
    config_fallback.copied()
}

pub async fn scrape_once(
    client: &RpcClient,
    metrics: &Metrics,
    cfg: &ExporterConfig,
    contracts: &ResolvedContracts,
) {
    let net = cfg.network.as_str();
    let started = Instant::now();
    let simulations_before = simulations_sent();

    let now_secs = read_ledger_now(client, metrics, net).await;
    let index_rows = read_market_indexes(client, metrics, net, contracts).await;

    let pool_id = match resolve_pool_id(client, &contracts.controller).await {
        Ok(p) => Some(p),
        Err(e) => {
            warn!(target: "exporter.collector", error = %e, "pool address unresolved; skipping pool getters this cycle");
            metrics
                .rpc_errors
                .with_label_values(&[net, "get_pool_address"])
                .inc();
            None
        }
    };
    let aggregator_id = resolve_price_aggregator(
        client,
        &contracts.controller,
        contracts.price_aggregator.as_ref(),
    )
    .await;
    if aggregator_id.is_none() {
        debug!(
            target: "exporter.collector",
            "price-aggregator unresolved; oracle config/staleness gauges skipped this cycle"
        );
    }

    let mut agg = Aggregates::default();

    let mut decimals: Vec<Option<u32>> = vec![None; contracts.markets.len()];

    for (i, (market, row)) in contracts.markets.iter().zip(index_rows.iter()).enumerate() {
        let hub_name = cfg.hub_name(market.hub_id);
        let labels = market_labels(net, market, &hub_name);
        let lref: Vec<&str> = labels.iter().map(String::as_str).collect();
        let price_wad = row.as_ref().map(|r| r.final_price_wad).unwrap_or(0);

        publish_market_index_view(metrics, net, market, &lref, row);
        if let Some(pool) = &pool_id {
            if let Some(sync) = read_sync_data(client, metrics, net, pool, market).await {
                let dec = sync.params.asset_decimals;
                decimals[i] = Some(dec);
                publish_market_params(metrics, &lref, &sync);

                metrics
                    .market_last_accrual_timestamp
                    .with_label_values(&lref)
                    .set(sync.last_timestamp as f64 / 1000.0);
                publish_market_amounts(
                    client, metrics, net, pool, market, &lref, dec, price_wad, &mut agg,
                )
                .await;
                if let Some(delta_ms) =
                    read_market_scalar_u64(client, metrics, net, pool, market, "get_delta_time")
                        .await
                {
                    metrics
                        .market_delta_time_seconds
                        .with_label_values(&lref)
                        .set(delta_ms as f64 / 1000.0);
                }
            }
        }

        if let Some(agg_id) = &aggregator_id {
            publish_oracle_config_and_freshness(
                client,
                metrics,
                net,
                market,
                now_secs,
                agg_id,
                decimals[i],
            )
            .await;
        }
    }

    if let Some(v) = read_view_i128(
        client,
        metrics,
        net,
        "get_min_borrow_collateral_usd",
        "*",
        &contracts.controller,
        "get_min_borrow_collateral_usd",
        vec![],
    )
    .await
    {
        metrics
            .min_borrow_collateral_usd
            .with_label_values(&[net])
            .set(model::wad_to_f64(v));
    }

    publish_spokes(client, metrics, net, cfg, contracts, &index_rows, &decimals).await;

    metrics
        .protocol_tvl_usd
        .with_label_values(&[net])
        .set(agg.supplied_usd);
    metrics
        .protocol_borrowed_usd
        .with_label_values(&[net])
        .set(agg.borrowed_usd);
    metrics
        .protocol_liquidity_usd
        .with_label_values(&[net])
        .set(agg.liquidity_usd);
    metrics
        .protocol_revenue_usd
        .with_label_values(&[net])
        .set(agg.revenue_usd);
    metrics
        .protocol_markets
        .with_label_values(&[net])
        .set(contracts.markets.len() as f64);
    metrics
        .protocol_spokes
        .with_label_values(&[net])
        .set(cfg.spokes.len() as f64);
    metrics
        .build_info
        .with_label_values(&[net, env!("CARGO_PKG_VERSION")])
        .set(1.0);

    info!(
        target: "exporter.collector",
        network = net,
        simulations = simulations_sent() - simulations_before,
        duration_ms = started.elapsed().as_secs_f64() * 1e3,
        "scrape complete"
    );
    metrics
        .scrape_duration_seconds
        .with_label_values(&[net])
        .set(started.elapsed().as_secs_f64());
    metrics
        .last_success_timestamp
        .with_label_values(&[net])
        .set(wall_clock_secs() as f64);
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
        metrics
            .ledger_sequence
            .with_label_values(&[net])
            .set(seq as f64);
    }
    match client.latest_close_time().await {
        Ok(close) => {
            metrics
                .ledger_timestamp
                .with_label_values(&[net])
                .set(close as f64);
            metrics
                .ledger_skew_seconds
                .with_label_values(&[net])
                .set((close - wall) as f64);
            close
        }
        Err(e) => {
            warn!(target: "exporter.collector", error = %e, "ledger close-time read failed; using wall clock");
            metrics
                .rpc_errors
                .with_label_values(&[net, "latest_close_time"])
                .inc();
            metrics
                .ledger_timestamp
                .with_label_values(&[net])
                .set(wall as f64);
            wall
        }
    }
}

async fn read_market_indexes(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    contracts: &ResolvedContracts,
) -> Vec<Option<controller::MarketIndexView>> {
    let keys: Vec<HubAssetKey> = contracts
        .markets
        .iter()
        .map(|m| HubAssetKey {
            hub_id: m.hub_id,
            asset: m.asset_id,
        })
        .collect();
    if keys.is_empty() {
        return Vec::new();
    }

    // The full-market batch exceeds the mainnet simulation CPU budget, so keys
    // go in chunks of `INDEX_CHUNK_SIZE`. A chunk that fails falls back to
    // per-key reads for that chunk only.
    let mut out = Vec::with_capacity(keys.len());
    for (markets, chunk) in contracts
        .markets
        .chunks(INDEX_CHUNK_SIZE)
        .zip(keys.chunks(INDEX_CHUNK_SIZE))
    {
        if let Some(rows) = try_index_batch(client, &contracts.controller, chunk).await {
            if rows.len() == chunk.len() {
                out.extend(rows.into_iter().map(Some));
                continue;
            }
        }
        for (market, key) in markets.iter().zip(chunk.iter()) {
            let single =
                try_index_batch(client, &contracts.controller, std::slice::from_ref(key)).await;
            match single.and_then(|mut v| v.pop()) {
                Some(row) => out.push(Some(row)),
                None => {
                    metrics
                        .view_failures
                        .with_label_values(&[
                            net,
                            "get_market_indexes_detailed",
                            &market.asset_strkey,
                            "batch_key",
                        ])
                        .inc();
                    out.push(None);
                }
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

fn publish_market_index_view(
    metrics: &Metrics,
    net: &str,
    market: &ResolvedMarket,
    lref: &[&str],
    row: &Option<controller::MarketIndexView>,
) {
    let olabels = [net, market.asset_strkey.as_str(), market.symbol.as_str()];
    let b = |v: bool| if v { 1.0 } else { 0.0 };
    match row {
        Some(r) => {
            metrics
                .market_supply_index_ray
                .with_label_values(lref)
                .set(model::ray_to_f64(r.supply_index_ray));
            metrics
                .market_borrow_index_ray
                .with_label_values(lref)
                .set(model::ray_to_f64(r.borrow_index_ray));
            metrics
                .oracle_price_usd
                .with_label_values(&olabels)
                .set(model::wad_to_f64(r.final_price_wad));
            metrics
                .oracle_primary_price_usd
                .with_label_values(&olabels)
                .set(model::wad_to_f64(r.primary_price_wad));
            metrics
                .oracle_anchor_price_usd
                .with_label_values(&olabels)
                .set(model::wad_to_f64(r.anchor_price_wad));
            if let Some(dev) = model::deviation_bps(r.primary_price_wad, r.anchor_price_wad) {
                metrics
                    .oracle_deviation_bps
                    .with_label_values(&olabels)
                    .set(dev);
            }
            metrics
                .oracle_status_timestamp
                .with_label_values(&olabels)
                .set(r.price_timestamp as f64);
            metrics
                .oracle_stale
                .with_label_values(&olabels)
                .set(b(r.stale));
            metrics
                .oracle_deviation_flag
                .with_label_values(&olabels)
                .set(b(r.deviation));
            metrics
                .oracle_healthy
                .with_label_values(&olabels)
                .set(b(r.valid));
            metrics
                .oracle_error_code
                .with_label_values(&olabels)
                .set(r.error_code.unwrap_or(0) as f64);
        }
        None => {
            metrics.oracle_healthy.with_label_values(&olabels).set(0.0);
            metrics.oracle_stale.with_label_values(&olabels).set(0.0);
            metrics
                .oracle_deviation_flag
                .with_label_values(&olabels)
                .set(0.0);
            metrics
                .oracle_error_code
                .with_label_values(&olabels)
                .set(0.0);
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
    set(
        "base_borrow_rate",
        model::ray_to_f64(p.base_borrow_rate_ray),
    );
    set("max_borrow_rate", model::ray_to_f64(p.max_borrow_rate_ray));
    set("slope1", model::ray_to_f64(p.slope1_ray));
    set("slope2", model::ray_to_f64(p.slope2_ray));
    set("slope3", model::ray_to_f64(p.slope3_ray));
    set("mid_utilization", model::ray_to_f64(p.mid_utilization_ray));
    set(
        "optimal_utilization",
        model::ray_to_f64(p.optimal_utilization_ray),
    );
    set("max_utilization", model::ray_to_f64(p.max_utilization_ray));
    set(
        "reserve_factor_bps",
        model::bps_to_ratio(p.reserve_factor_bps),
    );
    set(
        "flashloan_fee_bps",
        model::bps_to_ratio(p.flashloan_fee_bps),
    );
    set(
        "is_flashloanable",
        if p.is_flashloanable { 1.0 } else { 0.0 },
    );
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
    if let Some(v) =
        read_market_scalar(client, metrics, net, pool_id, market, "get_supplied_amount").await
    {
        let usd = model::token_usd(v, dec, price_wad);
        metrics
            .market_supplied
            .with_label_values(lref)
            .set(model::token_to_f64(v, dec));
        metrics.market_supplied_usd.with_label_values(lref).set(usd);
        agg.supplied_usd += usd;
    }
    if let Some(v) =
        read_market_scalar(client, metrics, net, pool_id, market, "get_borrowed_amount").await
    {
        let usd = model::token_usd(v, dec, price_wad);
        metrics
            .market_borrowed
            .with_label_values(lref)
            .set(model::token_to_f64(v, dec));
        metrics.market_borrowed_usd.with_label_values(lref).set(usd);
        agg.borrowed_usd += usd;
    }
    if let Some(v) = read_market_scalar(client, metrics, net, pool_id, market, "get_reserves").await
    {
        let usd = model::token_usd(v, dec, price_wad);
        metrics
            .market_liquidity
            .with_label_values(lref)
            .set(model::token_to_f64(v, dec));
        metrics
            .market_liquidity_usd
            .with_label_values(lref)
            .set(usd);
        agg.liquidity_usd += usd;
    }
    if let Some(v) = read_market_scalar(client, metrics, net, pool_id, market, "get_revenue").await
    {
        let usd = model::token_usd(v, dec, price_wad);
        metrics
            .market_revenue
            .with_label_values(lref)
            .set(model::token_to_f64(v, dec));
        metrics.market_revenue_usd.with_label_values(lref).set(usd);
        agg.revenue_usd += usd;
    }
    if let Some(v) =
        read_market_scalar(client, metrics, net, pool_id, market, "get_utilisation").await
    {
        metrics
            .market_utilization
            .with_label_values(lref)
            .set(model::ray_to_f64(v));
    }
    if let Some(v) =
        read_market_scalar(client, metrics, net, pool_id, market, "get_deposit_rate").await
    {
        metrics
            .market_supply_apy
            .with_label_values(lref)
            .set(model::apy_from_annual_ray(v));
    }
    if let Some(v) =
        read_market_scalar(client, metrics, net, pool_id, market, "get_borrow_rate").await
    {
        metrics
            .market_borrow_apy
            .with_label_values(lref)
            .set(model::apy_from_annual_ray(v));
    }
}

async fn read_sync_data(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    pool_id: &[u8; 32],
    market: &ResolvedMarket,
) -> Option<pool::MarketSync> {
    let key = HubAssetKey {
        hub_id: market.hub_id,
        asset: market.asset_id,
    };
    let arg = hub_asset_key_sc_val(&key).ok()?;
    let scv = read_view(
        client,
        metrics,
        net,
        "get_sync_data",
        &market.asset_strkey,
        pool_id,
        "get_sync_data",
        vec![arg],
    )
    .await?;
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
    let key = HubAssetKey {
        hub_id: market.hub_id,
        asset: market.asset_id,
    };
    let arg = hub_asset_key_sc_val(&key).ok()?;
    read_view_i128(
        client,
        metrics,
        net,
        function,
        &market.asset_strkey,
        pool_id,
        function,
        vec![arg],
    )
    .await
}

async fn read_market_scalar_u64(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    pool_id: &[u8; 32],
    market: &ResolvedMarket,
    function: &str,
) -> Option<u64> {
    let key = HubAssetKey {
        hub_id: market.hub_id,
        asset: market.asset_id,
    };
    let arg = hub_asset_key_sc_val(&key).ok()?;
    let scv = read_view(
        client,
        metrics,
        net,
        function,
        &market.asset_strkey,
        pool_id,
        function,
        vec![arg],
    )
    .await?;
    scval::as_u64(&scv)
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
    let scv = read_view(
        client,
        metrics,
        net,
        view_label,
        asset_label,
        contract,
        function,
        args,
    )
    .await?;
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
) -> Option<ScVal> {
    match simulate_view(client, contract, function, args).await {
        Ok(scv) => Some(scv),
        Err(e) => {
            count_view_error(metrics, net, view_label, asset_label, function, &e);
            None
        }
    }
}

fn count_view_error(
    metrics: &Metrics,
    net: &str,
    view_label: &str,
    asset_label: &str,
    function: &str,
    error: &ViewError,
) {
    match error {
        ViewError::Reverted(msg) => {
            let code = bucket_error_code(msg);
            metrics
                .view_failures
                .with_label_values(&[net, view_label, asset_label, &code])
                .inc();
            debug!(target: "exporter.collector", view = view_label, asset = asset_label, error = %msg, "view reverted");
        }
        ViewError::NoResult => {
            metrics
                .view_failures
                .with_label_values(&[net, view_label, asset_label, "no_result"])
                .inc();
        }
        ViewError::Rpc(e) => {
            metrics.rpc_errors.with_label_values(&[net, function]).inc();
            debug!(target: "exporter.collector", view = view_label, error = %e, "view rpc error");
        }
    }
}

async fn publish_oracle_config_and_freshness(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    market: &ResolvedMarket,
    now_secs: i64,
    aggregator_id: &[u8; 32],
    decimals: Option<u32>,
) {
    let olabels = [net, market.asset_strkey.as_str(), market.symbol.as_str()];
    let Some(config) = read_oracle_config(client, metrics, net, market, aggregator_id).await else {
        return;
    };

    // Sole-source only: then the published price is the LP leg price.
    if let (Some(floor), Some(dec), 1) = (&config.lp_floor, decimals, config.source_count) {
        publish_lp_floor_headroom(client, metrics, net, market, &olabels, floor, dec).await;
    }

    metrics
        .oracle_max_stale_seconds
        .with_label_values(&olabels)
        .set(config.max_price_stale_seconds as f64);
    metrics
        .oracle_tolerance_upper_bps
        .with_label_values(&olabels)
        .set(config.tolerance_upper_bps as f64);
    metrics
        .oracle_tolerance_lower_bps
        .with_label_values(&olabels)
        .set(config.tolerance_lower_bps as f64);
    metrics
        .oracle_sanity_min_usd
        .with_label_values(&olabels)
        .set(model::wad_to_f64(config.min_sanity_price_wad));
    metrics
        .oracle_sanity_max_usd
        .with_label_values(&olabels)
        .set(model::wad_to_f64(config.max_sanity_price_wad));
    metrics
        .oracle_strategy
        .with_label_values(&olabels)
        .set(config.source_count.saturating_sub(1) as f64);

    let mut worst: Option<(f64, u64, u64)> = None;
    for source in &config.sources {
        if let Some(feed_ts) = read_feed_timestamp(client, metrics, net, market, source).await {
            let sut = model::seconds_until_stale(now_secs, feed_ts, source.max_stale_seconds);
            if worst.map(|(w, _, _)| sut < w).unwrap_or(true) {
                worst = Some((sut, feed_ts, source.max_stale_seconds));
            }
        }
    }
    if let Some((sut, feed_ts, effective_max)) = worst {
        metrics
            .oracle_price_timestamp
            .with_label_values(&olabels)
            .set(feed_ts as f64);
        metrics
            .oracle_seconds_until_stale
            .with_label_values(&olabels)
            .set(sut);
        metrics
            .oracle_effective_max_stale_seconds
            .with_label_values(&olabels)
            .set(effective_max as f64);
    }
}

async fn publish_lp_floor_headroom(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    market: &ResolvedMarket,
    olabels: &[&str],
    floor: &oracle::LpFloor,
    share_decimals: u32,
) {
    metrics
        .oracle_lp_pool_floor_usd
        .with_label_values(olabels)
        .set(model::wad_to_f64(floor.min_pool_value_wad));
    let Ok(pool) = crate::keys::contract_id_from_strkey(&floor.pool) else {
        return;
    };
    if let Some(shares) = read_view_i128(
        client,
        metrics,
        net,
        "get_total_shares",
        &market.asset_strkey,
        &pool,
        "get_total_shares",
        vec![],
    )
    .await
    {
        metrics
            .oracle_lp_total_shares
            .with_label_values(olabels)
            .set(shares as f64 / 10f64.powi(share_decimals as i32));
    }
}

async fn read_oracle_config(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    market: &ResolvedMarket,
    aggregator_id: &[u8; 32],
) -> Option<oracle::OracleConfig> {
    let key = asset_oracle_ledger_key(aggregator_id, &market.asset_id).ok()?;
    let entries = match client.get_ledger_entries(std::slice::from_ref(&key)).await {
        Ok(e) => e,
        Err(e) => {
            metrics
                .rpc_errors
                .with_label_values(&[net, "get_ledger_entries"])
                .inc();
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
            let scv = read_view(
                client,
                metrics,
                net,
                "lastprice",
                &market.asset_strkey,
                &contract,
                "lastprice",
                vec![arg],
            )
            .await?;
            oracle::decode_reflector_price(&scv)
                .ok()
                .flatten()
                .map(|o| o.feed_ts_secs)
        }
        oracle::OracleKind::RedStone | oracle::OracleKind::Xoxno => {
            let feed = source.feed_id.as_ref()?;
            let arg = oracle::feed_id_arg(feed).ok()?;
            let scv = read_view(
                client,
                metrics,
                net,
                "read_price_data_for_feed",
                &market.asset_strkey,
                &contract,
                "read_price_data_for_feed",
                vec![arg],
            )
            .await?;
            oracle::decode_redstone_price(&scv)
                .ok()
                .map(|o| o.feed_ts_secs)
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
        let spoke_cfg = read_spoke_config(client, metrics, net, contracts, spoke_id).await;
        let deprecated = spoke_cfg.as_ref().map(|c| c.is_deprecated);
        if let Some(c) = &spoke_cfg {
            let s = spoke_id.to_string();
            let slabels = [net, s.as_str(), spoke_name.as_str()];
            metrics
                .spoke_liquidation_target_hf
                .with_label_values(&slabels)
                .set(model::wad_to_f64(c.liquidation_target_hf_wad));
            metrics
                .spoke_hf_for_max_bonus
                .with_label_values(&slabels)
                .set(model::wad_to_f64(c.hf_for_max_bonus_wad));
            metrics
                .spoke_liquidation_bonus_factor_bps
                .with_label_values(&slabels)
                .set(c.liquidation_bonus_factor_bps as f64);
        }
        for (i, (market, row)) in contracts.markets.iter().zip(index_rows.iter()).enumerate() {
            let dec = decimals.get(i).copied().flatten();
            let hub_name = cfg.hub_name(market.hub_id);
            publish_spoke_asset(
                client,
                metrics,
                net,
                contracts,
                spoke_id,
                &spoke_name,
                &hub_name,
                market,
                row,
                dec,
                deprecated,
            )
            .await;
        }
    }
}

async fn read_spoke_config(
    client: &RpcClient,
    metrics: &Metrics,
    net: &str,
    contracts: &ResolvedContracts,
    spoke_id: u32,
) -> Option<controller::SpokeConfig> {
    read_view(
        client,
        metrics,
        net,
        "get_spoke",
        "*",
        &contracts.controller,
        "get_spoke",
        vec![ScVal::U32(spoke_id)],
    )
    .await
    .and_then(|s| controller::decode_spoke(&s).ok())
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
    deprecated: Option<bool>,
) {
    let key = HubAssetKey {
        hub_id: market.hub_id,
        asset: market.asset_id,
    };
    let Ok(hub_arg) = hub_asset_key_sc_val(&key) else {
        return;
    };
    let s = spoke_id.to_string();
    let hub = market.hub_id.to_string();
    let labels = [
        net,
        s.as_str(),
        spoke_name,
        hub.as_str(),
        hub_name,
        market.asset_strkey.as_str(),
        market.symbol.as_str(),
    ];

    // `get_spoke_asset` reverts with `AssetNotInSpoke` (#307) for every hub
    // asset the spoke does not list, and most pairs are unlisted. Cache that
    // revert and skip the pair until `UNLISTED_SPOKE_ASSET_TTL` expires.
    // ponytail: fixed TTL; invalidate on the controller's listing event if
    // a 10-minute lag after a governance listing ever matters.
    let unlisted_key = (spoke_id, market.hub_id, market.asset_id);
    {
        let unlisted = unlisted_spoke_assets()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if unlisted
            .get(&unlisted_key)
            .is_some_and(|since| since.elapsed() < UNLISTED_SPOKE_ASSET_TTL)
        {
            return;
        }
    }
    let cfg_scv = match simulate_view(
        client,
        &contracts.controller,
        "get_spoke_asset",
        vec![ScVal::U32(spoke_id), hub_arg.clone()],
    )
    .await
    {
        Ok(s) => s,
        Err(ViewError::Reverted(msg)) if bucket_error_code(&msg) == ASSET_NOT_IN_SPOKE => {
            metrics.remove_spoke_asset(&labels);
            unlisted_spoke_assets()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(unlisted_key, Instant::now());
            return;
        }
        Err(e) => {
            count_view_error(
                metrics,
                net,
                "get_spoke_asset",
                &market.asset_strkey,
                "get_spoke_asset",
                &e,
            );
            return;
        }
    };

    let cfg = match controller::decode_spoke_asset(&cfg_scv) {
        Ok(cfg) => cfg,
        Err(e) => {
            metrics
                .view_failures
                .with_label_values(&[net, "get_spoke_asset", &market.asset_strkey, "decode"])
                .inc();
            debug!(target: "exporter.collector", spoke_id, asset = %market.asset_strkey, error = %e, "decode spoke_asset failed");
            metrics.remove_spoke_asset(&labels);
            return;
        }
    };

    let b = |v: bool| if v { 1.0 } else { 0.0 };
    metrics
        .spoke_paused
        .with_label_values(&labels)
        .set(b(cfg.paused));
    metrics
        .spoke_frozen
        .with_label_values(&labels)
        .set(b(cfg.frozen));
    metrics
        .spoke_collateral_enabled
        .with_label_values(&labels)
        .set(b(cfg.is_collateralizable));
    metrics
        .spoke_borrow_enabled
        .with_label_values(&labels)
        .set(b(cfg.is_borrowable));
    if let Some(deprecated) = deprecated {
        metrics
            .spoke_deprecated
            .with_label_values(&labels)
            .set(b(deprecated));
    }

    metrics
        .spoke_supply_closed
        .with_label_values(&labels)
        .set(model::market_closed(cfg.supply_cap));
    metrics
        .spoke_borrow_closed
        .with_label_values(&labels)
        .set(model::market_closed(cfg.borrow_cap));
    metrics
        .spoke_ltv_bps
        .with_label_values(&labels)
        .set(cfg.loan_to_value_bps as f64);
    metrics
        .spoke_liq_threshold_bps
        .with_label_values(&labels)
        .set(cfg.liquidation_threshold_bps as f64);
    metrics
        .spoke_liq_bonus_bps
        .with_label_values(&labels)
        .set(cfg.liquidation_bonus_bps as f64);
    metrics
        .spoke_liq_fees_bps
        .with_label_values(&labels)
        .set(cfg.liquidation_fees_bps as f64);

    let Some(dec) = decimals else {
        return;
    };
    metrics
        .spoke_supply_cap
        .with_label_values(&labels)
        .set(model::token_to_f64(cfg.supply_cap, dec));
    metrics
        .spoke_borrow_cap
        .with_label_values(&labels)
        .set(model::token_to_f64(cfg.borrow_cap, dec));
    metrics
        .spoke_supply_closed
        .with_label_values(&labels)
        .set(model::market_closed_at(cfg.supply_cap, dec));
    metrics
        .spoke_borrow_closed
        .with_label_values(&labels)
        .set(model::market_closed_at(cfg.borrow_cap, dec));

    let (supply_index, borrow_index, price_wad) = match row {
        Some(r) => (r.supply_index_ray, r.borrow_index_ray, r.final_price_wad),
        None => return,
    };
    if let Ok(usage_scv) = simulate_view(
        client,
        &contracts.controller,
        "get_spoke_usage",
        vec![ScVal::U32(spoke_id), hub_arg],
    )
    .await
    {
        if let Ok(usage) = controller::decode_spoke_usage(&usage_scv) {
            let supply_tokens =
                model::scaled_usage_to_token(usage.supplied_scaled_ray, supply_index);
            let borrow_tokens =
                model::scaled_usage_to_token(usage.borrowed_scaled_ray, borrow_index);
            let price = model::wad_to_f64(price_wad);
            metrics
                .spoke_supply_usage
                .with_label_values(&labels)
                .set(supply_tokens);
            metrics
                .spoke_supply_usage_usd
                .with_label_values(&labels)
                .set(supply_tokens * price);
            metrics
                .spoke_borrow_usage
                .with_label_values(&labels)
                .set(borrow_tokens);
            metrics
                .spoke_borrow_usage_usd
                .with_label_values(&labels)
                .set(borrow_tokens * price);
            set_or_remove(
                &metrics.spoke_supply_cap_utilization,
                &labels,
                model::cap_utilization(supply_tokens, cfg.supply_cap, dec),
            );
            set_or_remove(
                &metrics.spoke_borrow_cap_utilization,
                &labels,
                model::cap_utilization(borrow_tokens, cfg.borrow_cap, dec),
            );
        }
    }
}

fn set_or_remove(gauge: &GaugeVec, labels: &[&str], value: Option<f64>) {
    match value {
        Some(v) => gauge.with_label_values(labels).set(v),
        None => {
            let _ = gauge.remove_label_values(labels);
        }
    }
}

fn bucket_error_code(msg: &str) -> String {
    if let Some(pos) = msg.find('#') {
        let digits: String = msg[pos + 1..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
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

#[cfg(test)]
mod spoke_asset_tests {
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::Arc;

    use axum::{extract::State, http::header, routing::post, Router};
    use serde_json::{json, Value};
    use stellar_xdr::{
        HostFunction, Int128Parts, Limits, OperationBody, ReadXdr, ScMap, ScMapEntry,
        TransactionEnvelope, WriteXdr,
    };
    use tokio::net::TcpListener;

    use super::*;
    use crate::keys::symbol;

    const LISTED: u8 = 0;
    const DELISTED: u8 = 1;
    const CAPS_ZERO: u8 = 2;
    const DECODE_BROKEN: u8 = 3;
    const OTHER_REVERT: u8 = 4;
    const RPC_DOWN: u8 = 5;
    const SPOKE_DOWN: u8 = 6;
    const RAY: i128 = 1_000_000_000_000_000_000_000_000_000;
    const WAD: i128 = 1_000_000_000_000_000_000;
    const DECIMALS: u32 = 7;

    fn i128v(v: i128) -> ScVal {
        ScVal::I128(Int128Parts {
            hi: (v >> 64) as i64,
            lo: v as u64,
        })
    }

    fn map(entries: Vec<(&str, ScVal)>) -> ScVal {
        let entries: Vec<ScMapEntry> = entries
            .into_iter()
            .map(|(k, val)| ScMapEntry {
                key: ScVal::Symbol(symbol(k).unwrap()),
                val,
            })
            .collect();
        ScVal::Map(Some(ScMap(entries.try_into().unwrap())))
    }

    fn spoke_asset(cap_tokens: i128, with_caps: bool) -> ScVal {
        let mut e = vec![
            ("is_collateralizable", ScVal::Bool(true)),
            ("is_borrowable", ScVal::Bool(true)),
            ("paused", ScVal::Bool(true)),
            ("frozen", ScVal::Bool(false)),
            ("loan_to_value", ScVal::U32(7000)),
            ("liquidation_threshold", ScVal::U32(8000)),
            ("liquidation_bonus", ScVal::U32(500)),
            ("liquidation_fees", ScVal::U32(100)),
        ];
        if with_caps {
            let cap = cap_tokens * 10i128.pow(DECIMALS);
            e.push(("supply_cap", i128v(cap)));
            e.push(("borrow_cap", i128v(cap)));
        }
        map(e)
    }

    fn sim_ok(v: ScVal) -> Value {
        json!({"latestLedger": 1, "results": [{"auth": [], "xdr": v.to_xdr_base64(Limits::none()).unwrap()}]})
    }

    fn sim_revert(msg: &str) -> Value {
        json!({"latestLedger": 1, "error": msg})
    }

    async fn rpc(
        State(phase): State<Arc<AtomicU8>>,
        body: String,
    ) -> ([(header::HeaderName, &'static str); 1], String) {
        let req: Value = serde_json::from_str(&body).unwrap();
        let tx = req["params"]["transaction"].as_str().unwrap();
        let TransactionEnvelope::Tx(env) =
            TransactionEnvelope::from_xdr_base64(tx, Limits::none()).unwrap()
        else {
            panic!("unexpected envelope")
        };
        let OperationBody::InvokeHostFunction(op) = &env.tx.operations[0].body else {
            panic!("not an invoke")
        };
        let HostFunction::InvokeContract(args) = &op.host_function else {
            panic!("not a contract invoke")
        };
        let func = args.function_name.0.to_utf8_string_lossy();
        let result = match (func.as_str(), phase.load(Ordering::SeqCst)) {
            ("get_spoke", SPOKE_DOWN) => sim_revert("HostError: Error(Contract, #1)"),
            ("get_spoke", _) => sim_ok(map(vec![("is_deprecated", ScVal::Bool(true))])),
            ("get_spoke_asset", DELISTED) => sim_revert("HostError: Error(Contract, #307)"),
            ("get_spoke_asset", OTHER_REVERT) => sim_revert("HostError: Error(Contract, #10)"),
            ("get_spoke_asset", RPC_DOWN) => {
                let resp = json!({"jsonrpc": "2.0", "id": req["id"].clone(), "error": {"code": -32603, "message": "down"}});
                return (
                    [(header::CONTENT_TYPE, "application/json")],
                    resp.to_string(),
                );
            }
            ("get_spoke_asset", CAPS_ZERO) => sim_ok(spoke_asset(0, true)),
            ("get_spoke_asset", DECODE_BROKEN) => sim_ok(spoke_asset(0, false)),
            ("get_spoke_asset", _) => sim_ok(spoke_asset(100, true)),
            ("get_spoke_usage", _) => sim_ok(map(vec![
                ("supplied_scaled_ray", i128v(97 * RAY)),
                ("borrowed_scaled_ray", i128v(97 * RAY)),
            ])),
            _ => sim_revert("HostError: Error(Contract, #1)"),
        };
        let resp = json!({"jsonrpc": "2.0", "id": req["id"].clone(), "result": result});
        (
            [(header::CONTENT_TYPE, "application/json")],
            resp.to_string(),
        )
    }

    struct Fixture {
        phase: Arc<AtomicU8>,
        client: RpcClient,
        cfg: ExporterConfig,
        contracts: ResolvedContracts,
        rows: Vec<Option<controller::MarketIndexView>>,
        metrics: Metrics,
        spoke: String,
    }

    // The unlisted cache is process-global, so every test uses its own spoke id.
    async fn fixture(spoke_id: u32) -> Fixture {
        let phase = Arc::new(AtomicU8::new(LISTED));
        let app = Router::new()
            .route("/", post(rpc))
            .with_state(phase.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let cfg: ExporterConfig = serde_yaml::from_str(&format!(
            "network: testnet\nrpc: {{url: \"http://{addr}\", passphrase: x}}\ncontracts: {{controller: C}}\nspokes: [{spoke_id}]\nmetrics: {{bind: \"127.0.0.1:0\"}}\n"
        ))
        .unwrap();
        let client = RpcClient::new(&cfg.rpc).unwrap();
        let contracts = ResolvedContracts {
            controller: [7u8; 32],
            price_aggregator: None,
            oracle_adapter: None,
            markets: vec![ResolvedMarket {
                hub_id: 1,
                asset_id: [spoke_id as u8; 32],
                asset_strkey: "CASSET".into(),
                symbol: "SYM".into(),
            }],
        };
        let rows = vec![Some(controller::MarketIndexView {
            supply_index_ray: RAY,
            borrow_index_ray: RAY,
            final_price_wad: WAD,
            primary_price_wad: 0,
            anchor_price_wad: 0,
            price_timestamp: 0,
            stale: false,
            deviation: false,
            valid: true,
            error_code: None,
        })];
        Fixture {
            phase,
            client,
            cfg,
            contracts,
            rows,
            metrics: Metrics::new().unwrap(),
            spoke: spoke_id.to_string(),
        }
    }

    impl Fixture {
        async fn scrape(&self, phase: u8) {
            self.phase.store(phase, Ordering::SeqCst);
            publish_spokes(
                &self.client,
                &self.metrics,
                "testnet",
                &self.cfg,
                &self.contracts,
                &self.rows,
                &[Some(DECIMALS)],
            )
            .await;
        }

        fn spoke_asset_series(&self) -> Vec<(String, f64)> {
            self.metrics
                .registry
                .gather()
                .iter()
                .flat_map(|f| {
                    f.get_metric()
                        .iter()
                        .filter(|m| {
                            let l = m.get_label();
                            l.iter()
                                .any(|l| l.get_name() == "spoke_id" && l.get_value() == self.spoke)
                                && l.iter().any(|l| l.get_name() == "hub_id")
                        })
                        .map(|m| (f.get_name().to_string(), m.get_gauge().get_value()))
                        .collect::<Vec<_>>()
                })
                .collect()
        }

        fn value(&self, family: &str) -> Option<f64> {
            self.spoke_asset_series()
                .into_iter()
                .find(|(name, _)| name == family)
                .map(|(_, v)| v)
        }

        fn counter(&self, family: &str, labels: &[(&str, &str)]) -> f64 {
            self.metrics
                .registry
                .gather()
                .iter()
                .filter(|f| f.get_name() == family)
                .flat_map(|f| f.get_metric().to_vec())
                .filter(|m| {
                    labels.iter().all(|(k, v)| {
                        m.get_label()
                            .iter()
                            .any(|l| l.get_name() == *k && l.get_value() == *v)
                    })
                })
                .map(|m| m.get_counter().get_value())
                .sum()
        }
    }

    fn near(v: Option<f64>, want: f64) -> bool {
        v.is_some_and(|v| (v - want).abs() < 1e-9)
    }

    #[tokio::test]
    async fn delisted_spoke_asset_drops_every_series() {
        let f = fixture(41).await;
        f.scrape(LISTED).await;
        assert_eq!(f.spoke_asset_series().len(), 19);
        assert_eq!(f.value("lending_spoke_paused"), Some(1.0));
        assert!(near(f.value("lending_spoke_supply_cap_utilization"), 0.97));

        f.scrape(DELISTED).await;
        f.scrape(DELISTED).await;
        assert_eq!(f.spoke_asset_series(), vec![]);
        assert_eq!(
            f.counter(
                "lending_exporter_view_failures_total",
                &[("view", "get_spoke_asset")]
            ),
            0.0
        );
    }

    #[tokio::test]
    async fn undecodable_spoke_asset_drops_every_series() {
        let f = fixture(42).await;
        f.scrape(LISTED).await;
        assert_eq!(f.value("lending_spoke_paused"), Some(1.0));

        f.scrape(DECODE_BROKEN).await;
        assert_eq!(f.spoke_asset_series(), vec![]);
        assert_eq!(
            f.counter(
                "lending_exporter_view_failures_total",
                &[("view", "get_spoke_asset"), ("code", "decode")]
            ),
            1.0
        );
    }

    #[tokio::test]
    async fn closed_cap_drops_cap_utilization() {
        let f = fixture(43).await;
        f.scrape(LISTED).await;
        assert!(near(f.value("lending_spoke_supply_cap_utilization"), 0.97));
        assert!(near(f.value("lending_spoke_borrow_cap_utilization"), 0.97));

        f.scrape(CAPS_ZERO).await;
        assert_eq!(f.value("lending_spoke_supply_closed"), Some(1.0));
        assert_eq!(f.value("lending_spoke_borrow_closed"), Some(1.0));
        assert_eq!(f.value("lending_spoke_supply_cap"), Some(0.0));
        assert!(near(f.value("lending_spoke_supply_usage"), 97.0));
        assert_eq!(f.value("lending_spoke_supply_cap_utilization"), None);
        assert_eq!(f.value("lending_spoke_borrow_cap_utilization"), None);

        f.scrape(LISTED).await;
        assert!(near(f.value("lending_spoke_supply_cap_utilization"), 0.97));
    }

    #[tokio::test]
    async fn unexpected_spoke_asset_failures_are_counted_and_retried() {
        let f = fixture(44).await;
        f.scrape(LISTED).await;

        f.scrape(OTHER_REVERT).await;
        f.scrape(OTHER_REVERT).await;
        assert_eq!(
            f.counter(
                "lending_exporter_view_failures_total",
                &[("view", "get_spoke_asset"), ("code", "10")]
            ),
            2.0
        );
        assert_eq!(f.value("lending_spoke_paused"), Some(1.0));

        f.scrape(RPC_DOWN).await;
        assert_eq!(
            f.counter(
                "lending_exporter_rpc_errors_total",
                &[("op", "get_spoke_asset")]
            ),
            1.0
        );
        assert_eq!(f.value("lending_spoke_paused"), Some(1.0));
    }

    #[tokio::test]
    async fn failed_get_spoke_keeps_the_last_deprecated_value() {
        let f = fixture(45).await;
        f.scrape(LISTED).await;
        assert_eq!(f.value("lending_spoke_deprecated"), Some(1.0));

        f.scrape(SPOKE_DOWN).await;
        assert_eq!(f.value("lending_spoke_paused"), Some(1.0));
        assert_eq!(f.value("lending_spoke_deprecated"), Some(1.0));
    }
}
