//! Native pool contract: direct supply, borrow, withdraw, repay, index updates,
//! rewards, revenue claims, and views without controller fanout.
#![no_main]
use arbitrary::Arbitrary;
use common::constants::{BPS, RAY};
use common::errors::GenericError;
use common::types::{
    HubAssetKey, InterestRateModel, MarketParamsRaw, PoolAction, PoolBorrowEntry, PoolKey,
    PoolSeizeEntry, PoolStateRaw, PoolSupplyEntry, PoolWithdrawEntry, ScaledPositionRaw,
};
use libfuzzer_sys::fuzz_target;
use pool::{LiquidityPool, LiquidityPoolClient};
use soroban_sdk::{testutils::Address as _, testutils::Ledger as _, token, Address, Env};

const ACCOUNTING_TOLERANCE_UNITS: i128 = 4;

#[derive(Debug, Arbitrary)]
struct In {
    // Interest-curve geometry (same clamping as rates_and_index).
    base_pct: u8,
    s1_pct: u8,
    s2_pct: u8,
    s3_pct: u16,
    mid_pct: u8,
    opt_pct: u8,
    // Retained for corpus-input compatibility; `make_params` derives
    // max_borrow_rate from the slope chain.
    #[allow(dead_code)]
    max_pct: u16,
    reserve_pct: u8,
    // Sequence of (price_wad, time_advance_ms, op_kind) ops.
    // op_kind dispatches direct pool actions, index updates, revenue, and views.
    // (TTL extension is exercised by update_indexes via load_synced_cache.)
    ops: [(u32, u32, u8); 8],
}

fn make_params(_env: &Env, asset: &Address, i: &In) -> MarketParamsRaw {
    let mid_pct = (i.mid_pct % 98 + 1) as i128;
    let opt_pct = (i.opt_pct as i128 % (99 - mid_pct)) + mid_pct + 1;

    // Monotone slope chain bounded by MAX_BORROW_RATE_RAY (= 2*RAY → 200 pct):
    //   0 <= base <= slope1 <= slope2 <= slope3 <= max <= 200, max > base.
    let base_pct = (i.base_pct as i128) % 51; // 0..=50
    let s1_pct = base_pct + ((i.s1_pct as i128) % (101 - base_pct)); // base..=100
    let s2_pct = s1_pct + ((i.s2_pct as i128) % (151 - s1_pct)); // s1..=150
    let s3_pct = s2_pct + ((i.s3_pct as i128) % (201 - s2_pct)); // s2..=200
    let max_pct = (s3_pct.max(base_pct + 1)).clamp(s3_pct, 200); // s3..=200, > base

    MarketParamsRaw {
        base_borrow_rate: RAY * base_pct / 100,
        slope1: RAY * s1_pct / 100,
        slope2: RAY * s2_pct / 100,
        slope3: RAY * s3_pct / 100,
        mid_utilization: RAY * mid_pct / 100,
        optimal_utilization: RAY * opt_pct / 100,
        max_borrow_rate: RAY * max_pct / 100,
        reserve_factor: (((i.reserve_pct as i128 % 51) * 100).clamp(0, BPS - 1)) as u32,
        max_utilization: RAY,
        is_flashloanable: true,
        flashloan_fee: 0,
        asset_id: asset.clone(),
        asset_decimals: 7,
    }
}

fn hub_asset(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: 1,
        asset: asset.clone(),
    }
}

fn amount_from_raw(raw: u32, lo: i128, hi: i128) -> i128 {
    let span = (hi - lo).max(1);
    lo + (raw as i128 % span)
}

fn mint_to_pool(env: &Env, asset: &Address, pool_addr: &Address, amount: i128) {
    token::StellarAssetClient::new(env, asset).mint(pool_addr, &amount);
}

fn seed_cash(env: &Env, pool_addr: &Address, hub_asset: &HubAssetKey, cash: i128) {
    env.as_contract(pool_addr, || {
        let key = PoolKey::State(hub_asset.clone());
        let mut state: PoolStateRaw = env.storage().persistent().get(&key).unwrap();
        state.cash = cash;
        env.storage().persistent().set(&key, &state);
    });
}

fn pool_state(pool: &LiquidityPoolClient<'_>, hub_asset: &HubAssetKey) -> PoolStateRaw {
    pool.get_sync_data(hub_asset).state
}

fn pool_balance(env: &Env, asset: &Address, pool_addr: &Address) -> i128 {
    token::Client::new(env, asset).balance(pool_addr)
}

fn assert_cash_matches_balance(env: &Env, pool: &Address, asset: &Address, state: &PoolStateRaw) {
    // The pool tracks deposits via internal `cash`, not its live token balance,
    // and deliberately ignores tokens it never accounted for (donation
    // resistance). The real invariant is therefore `cash <= balance`: the pool
    // must never claim more cash than it actually holds — claiming more would
    // enable over-withdrawal. balance may legitimately exceed cash: a donation,
    // or (in this harness) tokens minted into the pool for an op that then
    // reverts, orphaning them outside the pool's accounting.
    let balance = pool_balance(env, asset, pool);
    assert!(
        state.cash <= balance,
        "cash exceeds token balance: cash={} balance={}",
        state.cash,
        balance,
    );
}

fn assert_pool_invariants(
    env: &Env,
    pool: &LiquidityPoolClient<'_>,
    pool_addr: &Address,
    asset: &Address,
    market: &HubAssetKey,
) {
    let state = pool_state(pool, market);
    let supplied = pool.get_supplied_amount(market);
    let borrowed = pool.get_borrowed_amount(market);
    let revenue = pool.get_revenue(market);

    assert_cash_matches_balance(env, pool_addr, asset, &state);
    assert!(state.cash >= 0, "negative tracked cash: {}", state.cash);
    assert!(
        state.supplied >= 0,
        "negative scaled supply: {}",
        state.supplied
    );
    assert!(
        state.borrowed >= 0,
        "negative scaled debt: {}",
        state.borrowed
    );
    assert!(
        state.revenue >= 0,
        "negative scaled revenue: {}",
        state.revenue
    );
    assert!(state.borrow_index >= RAY, "borrow index below RAY");
    // Borrow-side bad-debt seizure socializes losses by reducing the supply
    // index, so RAY is not a valid global floor after that operation.
    assert!(state.supply_index > 0, "supply index is non-positive");
    assert!(revenue <= supplied + ACCOUNTING_TOLERANCE_UNITS);
    assert!(
        state.cash + borrowed + ACCOUNTING_TOLERANCE_UNITS >= supplied,
        "pool insolvent: cash={} borrowed={} supplied={}",
        state.cash,
        borrowed,
        supplied
    );
}

fn assert_state_eq(before: &PoolStateRaw, after: &PoolStateRaw) {
    assert_eq!(before.supplied, after.supplied);
    assert_eq!(before.borrowed, after.borrowed);
    assert_eq!(before.revenue, after.revenue);
    assert_eq!(before.borrow_index, after.borrow_index);
    assert_eq!(before.supply_index, after.supply_index);
    assert_eq!(before.last_timestamp, after.last_timestamp);
    assert_eq!(before.cash, after.cash);
}

fn flatten_contract_result<T>(
    result: Result<
        Result<T, soroban_sdk::ConversionError>,
        Result<soroban_sdk::Error, soroban_sdk::InvokeError>,
    >,
) -> Result<T, soroban_sdk::Error> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => panic!("contract call succeeded but output conversion failed: {err:?}"),
        Err(invoke) => Err(invoke.expect("expected contract error, got host-level InvokeError")),
    }
}

fn supply_entry(hub_asset: &HubAssetKey, scaled_amount: i128, amount: i128) -> PoolSupplyEntry {
    PoolSupplyEntry {
        action: PoolAction {
            position: ScaledPositionRaw { scaled_amount },
            amount,
            hub_asset: hub_asset.clone(),
        },
    }
}

fn borrow_entry(hub_asset: &HubAssetKey, scaled_amount: i128, amount: i128) -> PoolBorrowEntry {
    PoolBorrowEntry {
        action: PoolAction {
            position: ScaledPositionRaw { scaled_amount },
            amount,
            hub_asset: hub_asset.clone(),
        },
    }
}

fn withdraw_entry(hub_asset: &HubAssetKey, scaled_amount: i128, amount: i128) -> PoolWithdrawEntry {
    PoolWithdrawEntry {
        action: PoolAction {
            position: ScaledPositionRaw { scaled_amount },
            amount,
            hub_asset: hub_asset.clone(),
        },
        protocol_fee: 0,
    }
}

fn rate_model_from_input(i: &In, salt: u32) -> InterestRateModel {
    let base_pct = ((i.base_pct as i128 + salt as i128) % 51).max(0);
    let s1_pct = base_pct + ((i.s1_pct as i128 + salt as i128) % (101 - base_pct));
    let s2_pct = s1_pct + ((i.s2_pct as i128 + salt as i128) % (151 - s1_pct));
    let s3_pct = s2_pct + ((i.s3_pct as i128 + salt as i128) % (201 - s2_pct));
    let mid_pct = (i.mid_pct as i128 + salt as i128) % 98 + 1;
    let opt_pct = ((i.opt_pct as i128 + salt as i128) % (99 - mid_pct)) + mid_pct + 1;
    let mut model = InterestRateModel {
        max_borrow_rate: RAY * s3_pct / 100,
        base_borrow_rate: RAY * base_pct / 100,
        slope1: RAY * s1_pct / 100,
        slope2: RAY * s2_pct / 100,
        slope3: RAY * s3_pct / 100,
        mid_utilization: RAY * mid_pct / 100,
        optimal_utilization: RAY * opt_pct / 100,
        max_utilization: RAY,
        reserve_factor: (((i.reserve_pct as i128 + salt as i128) % 51) * 100).clamp(0, BPS - 1)
            as u32,
        is_flashloanable: false,
        flashloan_fee: 0,
    };

    if salt & 1 == 0 {
        model.max_borrow_rate = (model.max_borrow_rate + (RAY / 100)).min(2 * RAY);
    } else {
        model.optimal_utilization = model.mid_utilization;
    }

    model
}

fuzz_target!(|i: In| {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);

    // Register a real Stellar Asset Contract so view functions like
    // `reserves()` (which calls `asset_token.balance(pool)`) succeed
    // instead of panicking with `Storage, MissingValue`. No tokens are
    // actually minted; the SAC returns balance 0 for the pool, which satisfies
    // the pre-activity invariants asserted by this target.
    let asset = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address()
        .clone();
    let market = hub_asset(&asset);

    let params = make_params(&env, &asset, &i);

    let pool_addr = env.register(LiquidityPool, (admin,));
    let pool = LiquidityPoolClient::new(&env, &pool_addr);
    pool.create_market(&1, &params);

    let receiver = Address::generate(&env);
    let payer = Address::generate(&env);

    let initial_cash = 100_000_000_000_000i128;
    mint_to_pool(&env, &asset, &pool_addr, initial_cash);
    seed_cash(&env, &pool_addr, &market, initial_cash);

    // Bootstrap a non-trivial live market so the direct pool ops can mutate
    // supply, debt, and revenue without controller fanout.
    let bootstrap_supply = amount_from_raw(i.base_pct as u32, 10_000_000_000, 50_000_000_000);
    mint_to_pool(&env, &asset, &pool_addr, bootstrap_supply);
    let bootstrap_supply_out = flatten_contract_result(pool.try_supply(&soroban_sdk::vec![
        &env,
        supply_entry(&market, 0, bootstrap_supply),
    ]))
    .expect("bootstrap supply should succeed");
    let mut supply_scaled = bootstrap_supply_out.get_unchecked(0).position.scaled_amount;
    assert_cash_matches_balance(&env, &pool_addr, &asset, &pool_state(&pool, &market));

    let bootstrap_borrow = amount_from_raw(i.reserve_pct as u32, 1_000_000_000, 10_000_000_000);
    let bootstrap_borrow_out = flatten_contract_result(pool.try_borrow(
        &receiver,
        &soroban_sdk::vec![&env, borrow_entry(&market, 0, bootstrap_borrow)],
    ))
    .expect("bootstrap borrow should succeed");
    let mut borrow_scaled = bootstrap_borrow_out.get_unchecked(0).position.scaled_amount;
    assert_pool_invariants(&env, &pool, &pool_addr, &asset, &market);

    // Track ledger time in seconds — Soroban's TestLedger timestamp is seconds.
    let mut cur_ts_s: u64 = env.ledger().timestamp();

    for (price_raw, dt_raw, op_kind) in i.ops.iter() {
        // Time advance: up to 100 days per step (scaled from u32).
        let dt_s: u64 = (*dt_raw as u64) % (100 * 86_400);
        cur_ts_s = cur_ts_s.saturating_add(dt_s);
        env.ledger().set_timestamp(cur_ts_s);

        let before = pool_state(&pool, &market);
        match op_kind % 11 {
            0 => {
                let amount = amount_from_raw(*price_raw, 1_000_000, 10_000_000_000);
                mint_to_pool(&env, &asset, &pool_addr, amount);
                let result = flatten_contract_result(pool.try_supply(&soroban_sdk::vec![
                    &env,
                    supply_entry(&market, supply_scaled, amount),
                ]));
                match result {
                    Ok(out) => {
                        let updated = out.get_unchecked(0);
                        assert!(
                            updated.position.scaled_amount >= supply_scaled,
                            "supply position regressed: prev={} new={}",
                            supply_scaled,
                            updated.position.scaled_amount
                        );
                        supply_scaled = updated.position.scaled_amount;
                        let after = pool_state(&pool, &market);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        assert!(after.supplied >= before.supplied);
                        assert!(after.borrow_index >= before.borrow_index);
                        assert!(after.supply_index >= before.supply_index);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &market)),
                }
            }
            1 => {
                // Borrow against the live pool balance. Every other case
                // deliberately overshoots reserves to exercise the rejection.
                let reserves = pool.get_reserves(&market).max(1);
                let amount = if op_kind & 1 == 0 {
                    amount_from_raw(*price_raw, 1_000_000, reserves.min(10_000_000_000))
                } else {
                    reserves + amount_from_raw(*price_raw, 1, 10_000_000)
                };
                let result = flatten_contract_result(pool.try_borrow(
                    &receiver,
                    &soroban_sdk::vec![&env, borrow_entry(&market, borrow_scaled, amount)],
                ));
                match result {
                    Ok(out) => {
                        let updated = out.get_unchecked(0);
                        assert!(
                            updated.position.scaled_amount >= borrow_scaled,
                            "borrow position regressed: prev={} new={}",
                            borrow_scaled,
                            updated.position.scaled_amount
                        );
                        borrow_scaled = updated.position.scaled_amount;
                        let after = pool_state(&pool, &market);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        assert!(after.borrowed >= before.borrowed);
                        assert!(after.borrow_index >= before.borrow_index);
                        assert!(after.supply_index >= before.supply_index);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &market)),
                }
            }
            2 => {
                // Withdraw from the live supply position. Half the time the
                // request is valid; half the time it intentionally exceeds the
                // current supply to keep the revert path alive.
                let supplied = pool.get_supplied_amount(&market).max(1);
                let amount = if op_kind & 1 == 0 {
                    amount_from_raw(*price_raw, 1_000_000, supplied.min(10_000_000_000))
                } else {
                    supplied + amount_from_raw(*price_raw, 1, 10_000_000)
                };
                let result = flatten_contract_result(pool.try_withdraw(
                    &receiver,
                    &false,
                    &soroban_sdk::vec![&env, withdraw_entry(&market, supply_scaled, amount)],
                ));
                match result {
                    Ok(out) => {
                        let updated = out.get_unchecked(0);
                        assert!(
                            updated.position.scaled_amount <= supply_scaled,
                            "withdraw position increased: prev={} new={}",
                            supply_scaled,
                            updated.position.scaled_amount
                        );
                        supply_scaled = updated.position.scaled_amount;
                        let after = pool_state(&pool, &market);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        assert!(after.borrow_index >= before.borrow_index);
                        assert!(after.supply_index >= before.supply_index);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &market)),
                }
            }
            3 => {
                // Repay the current borrow position. We keep this mostly
                // successful so the borrow index and scaled debt path move.
                let debt = pool.get_borrowed_amount(&market).max(1);
                let amount = amount_from_raw(*price_raw, 1_000_000, debt.min(10_000_000_000));
                mint_to_pool(&env, &asset, &pool_addr, amount);
                let result = flatten_contract_result(pool.try_repay(
                    &payer,
                    &soroban_sdk::vec![
                        &env,
                        PoolAction {
                            position: ScaledPositionRaw {
                                scaled_amount: borrow_scaled
                            },
                            amount,
                            hub_asset: market.clone(),
                        }
                    ],
                ));
                match result {
                    Ok(out) => {
                        let updated = out.get_unchecked(0);
                        assert!(
                            updated.position.scaled_amount <= borrow_scaled,
                            "repay position increased: prev={} new={}",
                            borrow_scaled,
                            updated.position.scaled_amount
                        );
                        borrow_scaled = updated.position.scaled_amount;
                        let after = pool_state(&pool, &market);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        assert!(after.borrowed <= before.borrowed);
                        assert!(after.borrow_index >= before.borrow_index);
                        assert!(after.supply_index >= before.supply_index);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &market)),
                }
            }
            4 => {
                let result = flatten_contract_result(pool.try_update_indexes(&market));
                match result {
                    Ok(_) => {
                        let after = pool_state(&pool, &market);
                        assert!(after.borrow_index >= before.borrow_index);
                        assert!(after.supply_index >= before.supply_index);
                        assert_eq!(after.cash, before.cash);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &market)),
                }
            }
            5 => {
                // add_rewards — pre-fund the pool, then exercise the supply
                // index uplift path.
                let amount = amount_from_raw(*price_raw, 1_000_000, 10_000_000_000);
                mint_to_pool(&env, &asset, &pool_addr, amount);
                let result = flatten_contract_result(pool.try_add_rewards(&market, &amount));
                match result {
                    Ok(()) => {
                        let after = pool_state(&pool, &market);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        assert!(after.revenue >= before.revenue);
                        assert!(after.supply_index >= before.supply_index);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &market)),
                }
            }
            6 => {
                let model = rate_model_from_input(&i, *price_raw);
                let result = flatten_contract_result(pool.try_update_params(&market, &model));
                match result {
                    Ok(()) => {
                        let after = pool_state(&pool, &market);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        assert!(after.borrow_index >= before.borrow_index);
                        assert!(after.supply_index >= before.supply_index);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &market)),
                }
            }
            7 => {
                let result = flatten_contract_result(pool.try_claim_revenue(&market));
                match result {
                    Ok(amount_mut) => {
                        assert!(amount_mut.actual_amount >= 0);
                        let after = pool_state(&pool, &market);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        assert!(after.revenue <= before.revenue);
                        assert!(after.borrow_index >= before.borrow_index);
                        assert!(after.supply_index >= before.supply_index);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &market)),
                }
            }
            8 => {
                let side = if op_kind & 1 == 0 {
                    common::types::AccountPositionType::Borrow
                } else {
                    common::types::AccountPositionType::Deposit
                };
                let position = if matches!(side, common::types::AccountPositionType::Borrow) {
                    ScaledPositionRaw {
                        scaled_amount: borrow_scaled,
                    }
                } else {
                    ScaledPositionRaw {
                        scaled_amount: supply_scaled,
                    }
                };
                // Sync to `now` so the seize is measured against an accrued
                // baseline. seize_positions accrues via synced_market_cache; the
                // outer `before` is captured pre-sync, so without this a
                // borrow-side seize (accrue up, then socialize down) looks like
                // it raises supply_index when it is only interest accrual.
                // Extreme fuzzer-advanced time at a high rate can overflow the
                // checked index math (MathOverflow) — a fail-closed revert, not
                // a bug — so tolerate it and skip the seize when the accrued
                // baseline can't be established (mirrors the op-4 `try_` path).
                if flatten_contract_result(pool.try_update_indexes(&market)).is_err() {
                    assert_pool_invariants(&env, &pool, &pool_addr, &asset, &market);
                    continue;
                }
                let before = pool_state(&pool, &market);
                let entry = PoolSeizeEntry {
                    hub_asset: market.clone(),
                    side,
                    position,
                };
                let result = flatten_contract_result(
                    pool.try_seize_positions(&soroban_sdk::vec![&env, entry]),
                );
                match result {
                    Ok(()) => {
                        let after = pool_state(&pool, &market);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        match side {
                            common::types::AccountPositionType::Borrow => {
                                assert!(after.borrowed <= before.borrowed);
                                assert!(after.supply_index <= before.supply_index);
                                borrow_scaled = 0;
                            }
                            common::types::AccountPositionType::Deposit => {
                                assert!(after.revenue >= before.revenue);
                                supply_scaled = 0;
                            }
                        }
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &market)),
                }
            }
            9 => {
                let amount = amount_from_raw(*price_raw, 1_000_000, 10_000_000_000);
                // Pool computes the strategy fee itself; fuzz both fee paths.
                let charge_fee = amount % 2 == 0;
                let receiver = Address::generate(&env);
                let action = PoolAction {
                    position: ScaledPositionRaw { scaled_amount: 0 },
                    amount,
                    hub_asset: market.clone(),
                };
                let result = flatten_contract_result(pool.try_create_strategy(
                    &receiver,
                    &action,
                    &charge_fee,
                ));
                match result {
                    Ok(out) => {
                        assert!(out.position.scaled_amount >= 0);
                        assert!(out.amount_received <= amount);
                        let after = pool_state(&pool, &market);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        assert!(after.borrowed >= before.borrowed);
                        assert!(after.revenue >= before.revenue);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &market)),
                }
            }
            10 => {
                // Pure-view sweep. The ratio views (utilisation, deposit rate,
                // borrow rate) are DOCUMENTED to panic MathOverflow when
                // borrowed_value * RAY / supplied_value exceeds i128 — a state
                // reachable here via deposit-seize -> claim-revenue -> dust
                // withdraw. Tolerate exactly that error and nothing else; any
                // view value that does come back must satisfy sign invariants.
                let math_overflow =
                    soroban_sdk::Error::from_contract_error(GenericError::MathOverflow as u32);
                let check_ratio_view = |label: &str,
                                            res: Result<
                    Result<i128, soroban_sdk::Error>,
                    Result<soroban_sdk::Error, soroban_sdk::InvokeError>,
                >| {
                    match res {
                        Ok(Ok(v)) => assert!(v >= 0, "negative {}: {}", label, v),
                        Err(Ok(err)) => assert_eq!(
                            err, math_overflow,
                            "{} failed with unexpected error: {:?}",
                            label, err
                        ),
                        other => panic!("unexpected {} result: {:?}", label, other),
                    }
                };
                check_ratio_view("utilization", pool.try_get_utilisation(&market));
                check_ratio_view("deposit rate", pool.try_get_deposit_rate(&market));
                check_ratio_view("borrow rate", pool.try_get_borrow_rate(&market));
                let reserves = pool.get_reserves(&market);
                let rev = pool.get_revenue(&market);
                let supplied = pool.get_supplied_amount(&market);
                let borrowed = pool.get_borrowed_amount(&market);
                let _dt = pool.get_delta_time(&market);
                let _sync = pool.get_sync_data(&market);

                assert!(reserves >= 0, "negative reserves: {}", reserves);
                assert!(rev >= 0, "negative protocol revenue: {}", rev);
                assert!(supplied >= 0, "negative supplied amount: {}", supplied);
                assert!(borrowed >= 0, "negative borrowed amount: {}", borrowed);
            }
            _ => unreachable!(),
        }
        assert_pool_invariants(&env, &pool, &pool_addr, &asset, &market);
    }
});
