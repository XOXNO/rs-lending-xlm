//! Native pool contract: direct supply, borrow, withdraw, repay, index updates,
//! rewards, revenue claims, and views without controller fanout.
#![no_main]
use arbitrary::Arbitrary;
use common::constants::{BPS, RAY};
use common::types::{
    InterestRateModel, MarketParamsRaw, PoolAction, PoolBorrowEntry, PoolKey, PoolStateRaw,
    PoolSupplyEntry, PoolWithdrawEntry, ScaledPositionRaw,
};
use libfuzzer_sys::fuzz_target;
use pool::{LiquidityPool, LiquidityPoolClient};
use soroban_sdk::{testutils::Address as _, testutils::Ledger as _, token, Address, Env};

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
        base_borrow_rate_ray: RAY * base_pct / 100,
        slope1_ray: RAY * s1_pct / 100,
        slope2_ray: RAY * s2_pct / 100,
        slope3_ray: RAY * s3_pct / 100,
        mid_utilization_ray: RAY * mid_pct / 100,
        optimal_utilization_ray: RAY * opt_pct / 100,
        max_borrow_rate_ray: RAY * max_pct / 100,
        reserve_factor_bps: (((i.reserve_pct as i128 % 51) * 100).clamp(0, BPS - 1)) as u32,
        max_utilization_ray: RAY,
        asset_id: asset.clone(),
        asset_decimals: 7,
    }
}

fn amount_from_raw(raw: u32, lo: i128, hi: i128) -> i128 {
    let span = (hi - lo).max(1);
    lo + (raw as i128 % span)
}

fn mint_to_pool(env: &Env, asset: &Address, pool_addr: &Address, amount: i128) {
    token::StellarAssetClient::new(env, asset).mint(pool_addr, &amount);
}

fn seed_cash(env: &Env, pool_addr: &Address, asset: &Address, cash: i128) {
    env.as_contract(pool_addr, || {
        let key = PoolKey::State(asset.clone());
        let mut state: PoolStateRaw = env.storage().persistent().get(&key).unwrap();
        state.cash = cash;
        env.storage().persistent().set(&key, &state);
    });
}

fn pool_state(pool: &LiquidityPoolClient<'_>, asset: &Address) -> PoolStateRaw {
    pool.get_sync_data(asset).state
}

fn pool_balance(env: &Env, asset: &Address, pool_addr: &Address) -> i128 {
    token::Client::new(env, asset).balance(pool_addr)
}

fn assert_cash_matches_balance(env: &Env, pool: &Address, asset: &Address, state: &PoolStateRaw) {
    assert_eq!(
        pool_balance(env, asset, pool),
        state.cash,
        "token balance and cash diverged"
    );
}

fn assert_state_eq(before: &PoolStateRaw, after: &PoolStateRaw) {
    assert_eq!(before.supplied_ray, after.supplied_ray);
    assert_eq!(before.borrowed_ray, after.borrowed_ray);
    assert_eq!(before.revenue_ray, after.revenue_ray);
    assert_eq!(before.borrow_index_ray, after.borrow_index_ray);
    assert_eq!(before.supply_index_ray, after.supply_index_ray);
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

fn supply_entry(asset: &Address, scaled_amount_ray: i128, amount: i128) -> PoolSupplyEntry {
    PoolSupplyEntry {
        action: PoolAction {
            position: ScaledPositionRaw { scaled_amount_ray },
            amount,
            asset: asset.clone(),
        },
        supply_cap: i128::MAX,
    }
}

fn borrow_entry(asset: &Address, scaled_amount_ray: i128, amount: i128) -> PoolBorrowEntry {
    PoolBorrowEntry {
        action: PoolAction {
            position: ScaledPositionRaw { scaled_amount_ray },
            amount,
            asset: asset.clone(),
        },
        borrow_cap: i128::MAX,
    }
}

fn withdraw_entry(asset: &Address, scaled_amount_ray: i128, amount: i128) -> PoolWithdrawEntry {
    PoolWithdrawEntry {
        action: PoolAction {
            position: ScaledPositionRaw { scaled_amount_ray },
            amount,
            asset: asset.clone(),
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
        max_borrow_rate_ray: RAY * s3_pct / 100,
        base_borrow_rate_ray: RAY * base_pct / 100,
        slope1_ray: RAY * s1_pct / 100,
        slope2_ray: RAY * s2_pct / 100,
        slope3_ray: RAY * s3_pct / 100,
        mid_utilization_ray: RAY * mid_pct / 100,
        optimal_utilization_ray: RAY * opt_pct / 100,
        max_utilization_ray: RAY,
        reserve_factor_bps: (((i.reserve_pct as i128 + salt as i128) % 51) * 100).clamp(0, BPS - 1)
            as u32,
    };

    if salt & 1 == 0 {
        model.max_borrow_rate_ray = (model.max_borrow_rate_ray + (RAY / 100)).min(2 * RAY);
    } else {
        model.optimal_utilization_ray = model.mid_utilization_ray;
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

    let params = make_params(&env, &asset, &i);

    // Register the pool natively at a fresh address; `__constructor` sets the owner.
    let pool_addr = env.register(LiquidityPool, (admin,));
    let pool = LiquidityPoolClient::new(&env, &pool_addr);
    pool.create_market(&params);

    let receiver = Address::generate(&env);
    let payer = Address::generate(&env);

    let initial_cash = 100_000_000_000_000i128;
    mint_to_pool(&env, &asset, &pool_addr, initial_cash);
    seed_cash(&env, &pool_addr, &asset, initial_cash);

    // Bootstrap a non-trivial live market so the direct pool ops can mutate
    // supply, debt, and revenue without controller fanout.
    let bootstrap_supply = amount_from_raw(i.base_pct as u32, 10_000_000_000, 50_000_000_000);
    mint_to_pool(&env, &asset, &pool_addr, bootstrap_supply);
    let bootstrap_supply_out = flatten_contract_result(pool.try_supply(&soroban_sdk::vec![
        &env,
        supply_entry(&asset, 0, bootstrap_supply),
    ]))
    .expect("bootstrap supply should succeed");
    let mut supply_scaled = bootstrap_supply_out
        .get_unchecked(0)
        .position
        .scaled_amount_ray;
    assert_cash_matches_balance(&env, &pool_addr, &asset, &pool_state(&pool, &asset));

    let bootstrap_borrow = amount_from_raw(i.reserve_pct as u32, 1_000_000_000, 10_000_000_000);
    let bootstrap_borrow_out = flatten_contract_result(pool.try_borrow(
        &receiver,
        &soroban_sdk::vec![&env, borrow_entry(&asset, 0, bootstrap_borrow)],
    ))
    .expect("bootstrap borrow should succeed");
    let mut borrow_scaled = bootstrap_borrow_out
        .get_unchecked(0)
        .position
        .scaled_amount_ray;
    assert_cash_matches_balance(&env, &pool_addr, &asset, &pool_state(&pool, &asset));

    // Track ledger time in seconds — Soroban's TestLedger timestamp is seconds.
    let mut cur_ts_s: u64 = env.ledger().timestamp();

    for (price_raw, dt_raw, op_kind) in i.ops.iter() {
        // Time advance: up to 100 days per step (scaled from u32).
        let dt_s: u64 = (*dt_raw as u64) % (100 * 86_400);
        cur_ts_s = cur_ts_s.saturating_add(dt_s);
        env.ledger().set_timestamp(cur_ts_s);

        let before = pool_state(&pool, &asset);
        match op_kind % 11 {
            0 => {
                // Direct supply: pre-fund the pool as the controller would.
                let amount = amount_from_raw(*price_raw, 1_000_000, 10_000_000_000);
                mint_to_pool(&env, &asset, &pool_addr, amount);
                let result = flatten_contract_result(pool.try_supply(&soroban_sdk::vec![
                    &env,
                    supply_entry(&asset, supply_scaled, amount),
                ]));
                match result {
                    Ok(out) => {
                        let updated = out.get_unchecked(0);
                        assert!(
                            updated.position.scaled_amount_ray >= supply_scaled,
                            "supply position regressed: prev={} new={}",
                            supply_scaled,
                            updated.position.scaled_amount_ray
                        );
                        supply_scaled = updated.position.scaled_amount_ray;
                        let after = pool_state(&pool, &asset);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        assert!(after.supplied_ray >= before.supplied_ray);
                        assert!(after.borrow_index_ray >= before.borrow_index_ray);
                        assert!(after.supply_index_ray >= before.supply_index_ray);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &asset)),
                }
            }
            1 => {
                // Borrow against the live pool balance. Every other case
                // deliberately overshoots reserves to exercise the rejection.
                let reserves = pool.reserves(&asset).max(1);
                let amount = if op_kind & 1 == 0 {
                    amount_from_raw(*price_raw, 1_000_000, reserves.min(10_000_000_000))
                } else {
                    reserves + amount_from_raw(*price_raw, 1, 10_000_000)
                };
                let result = flatten_contract_result(pool.try_borrow(
                    &receiver,
                    &soroban_sdk::vec![&env, borrow_entry(&asset, borrow_scaled, amount)],
                ));
                match result {
                    Ok(out) => {
                        let updated = out.get_unchecked(0);
                        assert!(
                            updated.position.scaled_amount_ray >= borrow_scaled,
                            "borrow position regressed: prev={} new={}",
                            borrow_scaled,
                            updated.position.scaled_amount_ray
                        );
                        borrow_scaled = updated.position.scaled_amount_ray;
                        let after = pool_state(&pool, &asset);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        assert!(after.borrowed_ray >= before.borrowed_ray);
                        assert!(after.borrow_index_ray >= before.borrow_index_ray);
                        assert!(after.supply_index_ray >= before.supply_index_ray);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &asset)),
                }
            }
            2 => {
                // Withdraw from the live supply position. Half the time the
                // request is valid; half the time it intentionally exceeds the
                // current supply to keep the revert path alive.
                let supplied = pool.supplied_amount(&asset).max(1);
                let amount = if op_kind & 1 == 0 {
                    amount_from_raw(*price_raw, 1_000_000, supplied.min(10_000_000_000))
                } else {
                    supplied + amount_from_raw(*price_raw, 1, 10_000_000)
                };
                let result = flatten_contract_result(pool.try_withdraw(
                    &receiver,
                    &false,
                    &soroban_sdk::vec![&env, withdraw_entry(&asset, supply_scaled, amount)],
                ));
                match result {
                    Ok(out) => {
                        let updated = out.get_unchecked(0);
                        assert!(
                            updated.position.scaled_amount_ray <= supply_scaled,
                            "withdraw position increased: prev={} new={}",
                            supply_scaled,
                            updated.position.scaled_amount_ray
                        );
                        supply_scaled = updated.position.scaled_amount_ray;
                        let after = pool_state(&pool, &asset);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        assert!(after.borrow_index_ray >= before.borrow_index_ray);
                        assert!(after.supply_index_ray >= before.supply_index_ray);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &asset)),
                }
            }
            3 => {
                // Repay the current borrow position. We keep this mostly
                // successful so the borrow index and scaled debt path move.
                let debt = pool.borrowed_amount(&asset).max(1);
                let amount = amount_from_raw(*price_raw, 1_000_000, debt.min(10_000_000_000));
                mint_to_pool(&env, &asset, &pool_addr, amount);
                let result = flatten_contract_result(pool.try_repay(
                    &payer,
                    &soroban_sdk::vec![
                        &env,
                        PoolAction {
                            position: ScaledPositionRaw {
                                scaled_amount_ray: borrow_scaled
                            },
                            amount,
                            asset: asset.clone(),
                        }
                    ],
                ));
                match result {
                    Ok(out) => {
                        let updated = out.get_unchecked(0);
                        assert!(
                            updated.position.scaled_amount_ray <= borrow_scaled,
                            "repay position increased: prev={} new={}",
                            borrow_scaled,
                            updated.position.scaled_amount_ray
                        );
                        borrow_scaled = updated.position.scaled_amount_ray;
                        let after = pool_state(&pool, &asset);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        assert!(after.borrowed_ray <= before.borrowed_ray);
                        assert!(after.borrow_index_ray >= before.borrow_index_ray);
                        assert!(after.supply_index_ray >= before.supply_index_ray);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &asset)),
                }
            }
            4 => {
                let _ = pool.try_update_indexes(&asset);
            }
            5 => {
                // add_rewards — pre-fund the pool, then exercise the supply
                // index uplift path.
                let amount = amount_from_raw(*price_raw, 1_000_000, 10_000_000_000);
                mint_to_pool(&env, &asset, &pool_addr, amount);
                let result = flatten_contract_result(pool.try_add_rewards(&asset, &amount));
                match result {
                    Ok(idx) => {
                        assert!(idx.supply_index_ray >= before.supply_index_ray);
                        let after = pool_state(&pool, &asset);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        assert!(after.revenue_ray >= before.revenue_ray);
                        assert!(after.supply_index_ray >= before.supply_index_ray);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &asset)),
                }
            }
            6 => {
                let model = rate_model_from_input(&i, *price_raw);
                let result = flatten_contract_result(pool.try_update_params(&asset, &model));
                match result {
                    Ok(()) => {
                        let after = pool_state(&pool, &asset);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        assert!(after.borrow_index_ray >= before.borrow_index_ray);
                        assert!(after.supply_index_ray >= before.supply_index_ray);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &asset)),
                }
            }
            7 => {
                let result = flatten_contract_result(pool.try_claim_revenue(&asset));
                match result {
                    Ok(amount_mut) => {
                        assert!(amount_mut.actual_amount >= 0);
                        let after = pool_state(&pool, &asset);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        assert!(after.revenue_ray <= before.revenue_ray);
                        assert!(after.borrow_index_ray >= before.borrow_index_ray);
                        assert!(after.supply_index_ray >= before.supply_index_ray);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &asset)),
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
                        scaled_amount_ray: borrow_scaled,
                    }
                } else {
                    ScaledPositionRaw {
                        scaled_amount_ray: supply_scaled,
                    }
                };
                let result =
                    flatten_contract_result(pool.try_seize_position(&asset, &side, &position));
                match result {
                    Ok(out) => {
                        assert_eq!(out.position.scaled_amount_ray, 0);
                        let after = pool_state(&pool, &asset);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        match side {
                            common::types::AccountPositionType::Borrow => {
                                assert!(after.borrowed_ray <= before.borrowed_ray);
                                assert!(after.supply_index_ray <= before.supply_index_ray);
                                borrow_scaled = 0;
                            }
                            common::types::AccountPositionType::Deposit => {
                                assert!(after.revenue_ray >= before.revenue_ray);
                                supply_scaled = 0;
                            }
                        }
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &asset)),
                }
            }
            9 => {
                let amount = amount_from_raw(*price_raw, 1_000_000, 10_000_000_000);
                let fee = amount / 10;
                let receiver = Address::generate(&env);
                let action = PoolAction {
                    position: ScaledPositionRaw {
                        scaled_amount_ray: 0,
                    },
                    amount,
                    asset: asset.clone(),
                };
                let result = flatten_contract_result(pool.try_create_strategy(
                    &receiver,
                    &action,
                    &fee,
                    &i128::MAX,
                ));
                match result {
                    Ok(out) => {
                        assert!(out.position.scaled_amount_ray >= 0);
                        assert!(out.amount_received <= amount);
                        let after = pool_state(&pool, &asset);
                        assert_cash_matches_balance(&env, &pool_addr, &asset, &after);
                        assert!(after.borrowed_ray >= before.borrowed_ray);
                        assert!(after.revenue_ray >= before.revenue_ray);
                    }
                    Err(_) => assert_state_eq(&before, &pool_state(&pool, &asset)),
                }
            }
            10 => {
                // Pure-view sweep — read-only functions shouldn't fail
                // under fresh-pool state; assert cross-function invariants.
                let util = pool.capital_utilisation(&asset);
                let reserves = pool.reserves(&asset);
                let deposit = pool.deposit_rate(&asset);
                let borrow = pool.borrow_rate(&asset);
                let rev = pool.protocol_revenue(&asset);
                let supplied = pool.supplied_amount(&asset);
                let borrowed = pool.borrowed_amount(&asset);
                let _dt = pool.delta_time(&asset);
                let _sync = pool.get_sync_data(&asset);

                assert!(util >= 0, "negative utilization: {}", util);
                assert!(reserves >= 0, "negative reserves: {}", reserves);
                assert!(rev >= 0, "negative protocol revenue: {}", rev);
                assert!(deposit >= 0, "negative deposit rate: {}", deposit);
                assert!(borrow >= 0, "negative borrow rate: {}", borrow);
                assert!(supplied >= 0, "negative supplied amount: {}", supplied);
                assert!(borrowed >= 0, "negative borrowed amount: {}", borrowed);
            }
            _ => unreachable!(),
        }
    }
});
