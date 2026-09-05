//! Pool-layer probes: the mocked owner supplies authentic positions and funded
//! amounts. These verify accounting composition, not controller authorization.

use common::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::types::{
    AccountPositionType, HubAssetKey, PoolAction, PoolBorrowEntry, PoolNetSettleEntry,
    PoolSeizeEntry, PoolStateRaw, PoolSupplyEntry, PoolWithdrawEntry, ScaledPositionRaw,
};
use flash_loan_receiver::{FlashLoanMode, FlashLoanRequest, FlashLoanTestReceiver};
use soroban_sdk::{testutils::Address as _, token, vec, xdr::ToXdr, Address};
use test_harness::{hub_asset, LendingTest};

const UNIT: i128 = 1_000_000;

fn fixture() -> LendingTest {
    let mut preset = test_harness::usdc_preset();
    preset.decimals = 6;
    preset.initial_liquidity = 0.0;
    let t = LendingTest::new().with_market(preset).build();
    let key = hub_asset(t.resolve_asset("USDC"));
    let pool = t.pool_client("USDC");
    let mut model = pool.get_sync_data(&key).params.rate_model_view();
    model.flashloan_fee = 100;
    model.is_flashloanable = true;
    model.max_utilization = RAY;
    pool.update_params(&key, &model);
    t
}

fn pos(scaled_amount: i128) -> ScaledPositionRaw {
    ScaledPositionRaw { scaled_amount }
}

fn action(key: &HubAssetKey, shares: i128, amount: i128) -> PoolAction {
    PoolAction {
        hub_asset: key.clone(),
        position: pos(shares),
        amount,
    }
}

fn books(t: &LendingTest, key: &HubAssetKey, user_supply: i128, user_debt: i128) -> PoolStateRaw {
    let state = t.pool_client("USDC").get_sync_data(key).state;
    assert_eq!(state.supplied, user_supply + state.revenue);
    assert_eq!(state.borrowed, user_debt);
    assert!(state.cash >= 0 && state.revenue >= 0 && state.revenue <= state.supplied);
    state
}

fn funded_supply(t: &LendingTest, key: &HubAssetKey, owner: &Address, amount: i128) -> i128 {
    let market = t.resolve_market("USDC");
    market.token_admin.mint(owner, &amount);
    token::Client::new(&t.env, &market.asset).transfer(owner, &market.pool, &amount);
    t.pool_client("USDC")
        .supply(&vec![
            &t.env,
            PoolSupplyEntry {
                action: action(key, 0, amount),
            },
        ])
        .get(0)
        .unwrap()
        .position
        .scaled_amount
}

#[test]
fn pool_all_money_paths_preserve_books_and_shared_token_custody() {
    let t = fixture();
    let env = &t.env;
    let market = t.resolve_market("USDC");
    let key = hub_asset(market.asset.clone());
    let second = HubAssetKey {
        hub_id: 2,
        asset: market.asset.clone(),
    };
    let pool = t.pool_client("USDC");
    pool.create_market(&second.hub_id, &pool.get_sync_data(&key).params);
    let token = token::Client::new(env, &market.asset);
    let payer = Address::generate(env);
    let receiver = Address::generate(env);
    let secondary_supply = funded_supply(&t, &second, &payer, 100 * UNIT);
    let mut supply = funded_supply(&t, &key, &payer, 1_000 * UNIT);
    let mut debt = 0;

    // An unsolicited donation belongs to no market's cash book.
    market.token_admin.mint(&payer, &(7 * UNIT));
    token.transfer(&payer, &market.pool, &(7 * UNIT));
    let check = |supply, debt, label: &str| {
        let state = books(&t, &key, supply, debt);
        let other = books(&t, &second, secondary_supply, 0);
        assert_eq!(other.cash, 100 * UNIT);
        assert_eq!(
            token.balance(&market.pool),
            state.cash + other.cash + 7 * UNIT
        );
        println!(
            "{label}: cash={}, supply={}, debt={}, revenue={}",
            state.cash, state.supplied, state.borrowed, state.revenue
        );
        state
    };
    check(supply, debt, "supply and donation");

    let result = pool
        .borrow(
            &receiver,
            &vec![
                env,
                PoolBorrowEntry {
                    action: action(&key, debt, 100 * UNIT),
                },
            ],
        )
        .get(0)
        .unwrap();
    debt = result.position.scaled_amount;
    assert_eq!(token.balance(&receiver), 100 * UNIT);
    assert_eq!(check(supply, debt, "borrow").cash, 900 * UNIT);

    // Partial and excessive payments both use funded amounts; refund must go
    // to payer while receiver's earlier proceeds remain untouched.
    for (payment, expected_used) in [(40 * UNIT, 40 * UNIT), (100 * UNIT, 60 * UNIT)] {
        market.token_admin.mint(&payer, &payment);
        let before = token.balance(&payer);
        token.transfer(&payer, &market.pool, &payment);
        let result = pool
            .repay(&payer, &vec![env, action(&key, debt, payment)])
            .get(0)
            .unwrap();
        debt = result.position.scaled_amount;
        assert_eq!(result.actual_amount, expected_used);
        assert_eq!(token.balance(&payer), before - expected_used);
        assert_eq!(token.balance(&receiver), 100 * UNIT);
        check(supply, debt, "repayment/refund");
    }

    let result = pool
        .withdraw(
            &receiver,
            &false,
            &vec![
                env,
                PoolWithdrawEntry {
                    action: action(&key, supply, 100 * UNIT),
                    protocol_fee: 0,
                },
            ],
        )
        .get(0)
        .unwrap();
    supply = result.position.scaled_amount;
    assert_eq!(result.actual_amount, 100 * UNIT);
    assert_eq!(token.balance(&receiver), 200 * UNIT);
    check(supply, debt, "withdraw");

    let before = token.balance(&receiver);
    let result = pool.create_strategy(&receiver, &action(&key, debt, 100 * UNIT), &true);
    debt = result.position.scaled_amount;
    assert_eq!(result.actual_amount, 100 * UNIT);
    assert_eq!(result.amount_received, 99 * UNIT);
    assert_eq!(token.balance(&receiver), before + 99 * UNIT);
    assert_eq!(check(supply, debt, "strategy fee").revenue, RAY);

    let before = token.balance(&market.pool);
    let before_supply = supply;
    let before_debt = debt;
    let result = pool.net_settle(&PoolNetSettleEntry {
        hub_asset: key.clone(),
        amount: 50 * UNIT,
        supply_position: pos(supply),
        debt_position: pos(debt),
    });
    supply = result.supply_position.scaled_amount;
    debt = result.debt_position.scaled_amount;
    assert_eq!(result.settled_amount, 50 * UNIT);
    assert_eq!(supply, before_supply - 50 * RAY);
    assert_eq!(debt, before_debt - 50 * RAY);
    assert_eq!(token.balance(&market.pool), before);
    check(supply, debt, "net settlement");

    pool.seize_positions(&vec![
        env,
        PoolSeizeEntry {
            hub_asset: key.clone(),
            side: AccountPositionType::Deposit,
            position: pos(10 * RAY),
        },
    ]);
    supply -= 10 * RAY;
    assert_eq!(token.balance(&market.pool), before);
    assert_eq!(
        check(supply, debt, "deposit reclassification").revenue,
        11 * RAY
    );

    let before_owner = token.balance(&t.controller);
    let claimed = pool.claim_revenue(&key).actual_amount;
    assert_eq!(claimed, 11 * UNIT);
    assert_eq!(token.balance(&t.controller), before_owner + claimed);
    assert_eq!(check(supply, debt, "revenue claim").revenue, 0);

    // Liquidation fees mint claims backed by retained cash, unlike deposit
    // seizure above, which only reclassifies existing shares.
    let before_receiver = token.balance(&receiver);
    let before_supply = supply;
    let result = pool
        .withdraw(
            &receiver,
            &true,
            &vec![
                env,
                PoolWithdrawEntry {
                    action: action(&key, supply, 10 * UNIT),
                    protocol_fee: UNIT,
                },
            ],
        )
        .get(0)
        .unwrap();
    supply = result.position.scaled_amount;
    assert_eq!(supply, before_supply - 10 * RAY);
    assert_eq!(result.actual_amount, 10 * UNIT);
    assert_eq!(token.balance(&receiver), before_receiver + 9 * UNIT);
    assert_eq!(check(supply, debt, "liquidation fee").revenue, RAY);

    let flash_receiver = env.register(FlashLoanTestReceiver, ());
    market.token_admin.mint(&flash_receiver, &UNIT);
    let before = check(supply, debt, "before flash");
    let request = FlashLoanRequest {
        mode: FlashLoanMode::Success,
    }
    .to_xdr(env);
    let fee = pool.flash_loan(&key, &payer, &flash_receiver, &(100 * UNIT), &request);
    assert_eq!(fee, UNIT);
    assert_eq!(token.balance(&flash_receiver), 0);
    let after = check(supply, debt, "flash fee");
    assert_eq!(after.cash, before.cash + fee);
    assert_eq!(after.borrowed, before.borrowed);
    assert_eq!(after.supplied, before.supplied + RAY);
    assert_eq!(after.revenue, before.revenue + RAY);

    let custody_before = token.balance(&market.pool);
    pool.seize_positions(&vec![
        env,
        PoolSeizeEntry {
            hub_asset: key.clone(),
            side: AccountPositionType::Borrow,
            position: pos(debt),
        },
    ]);
    debt = 0;
    let loss = check(supply, debt, "debt write-down");
    assert!(loss.supply_index < RAY);
    assert_eq!(token.balance(&market.pool), custody_before);

    // This write-down is fully covered: recapitalization refunds the whole input.
    market.token_admin.mint(&payer, &(3 * UNIT));
    let before_payer = token.balance(&payer);
    token.transfer(&payer, &market.pool, &(3 * UNIT));
    assert_eq!(
        pool.recapitalize(&key, &payer, &(3 * UNIT)).actual_amount,
        0
    );
    assert_eq!(token.balance(&payer), before_payer);
    check(supply, debt, "healthy recap refund");
}

#[test]
fn pool_loss_floor_recapitalization_returns_only_unused_funding() {
    let t = fixture();
    let env = &t.env;
    let market = t.resolve_market("USDC");
    let key = hub_asset(market.asset.clone());
    let pool = t.pool_client("USDC");
    let token = token::Client::new(env, &market.asset);
    let payer = Address::generate(env);
    let mut supply = funded_supply(&t, &key, &payer, 1_000 * UNIT);
    let debt = pool
        .borrow(
            &payer,
            &vec![
                env,
                PoolBorrowEntry {
                    action: action(&key, 0, 900 * UNIT),
                },
            ],
        )
        .get(0)
        .unwrap()
        .position
        .scaled_amount;
    supply = pool
        .withdraw(
            &payer,
            &false,
            &vec![
                env,
                PoolWithdrawEntry {
                    action: action(&key, supply, 100 * UNIT),
                    protocol_fee: 0,
                },
            ],
        )
        .get(0)
        .unwrap()
        .position
        .scaled_amount;
    assert_eq!(books(&t, &key, supply, debt).cash, 0);
    pool.seize_positions(&vec![
        env,
        PoolSeizeEntry {
            hub_asset: key.clone(),
            side: AccountPositionType::Borrow,
            position: pos(debt),
        },
    ]);
    let wiped = books(&t, &key, supply, 0);
    assert_eq!(wiped.supply_index, SUPPLY_INDEX_FLOOR_RAW);
    assert_eq!(wiped.cash, 0);
    assert_eq!(token.balance(&market.pool), 0);

    // Claims are 900 tokens * 0.001 = 0.9 token. Fill 0.4, then offer
    // another 2.0: only 0.5 must stay; payer receives exactly 1.5 back.
    for (payment, applied) in [(400_000, 400_000), (2 * UNIT, 500_000)] {
        market.token_admin.mint(&payer, &payment);
        let before = token.balance(&payer);
        token.transfer(&payer, &market.pool, &payment);
        assert_eq!(
            pool.recapitalize(&key, &payer, &payment).actual_amount,
            applied
        );
        assert_eq!(token.balance(&payer), before - applied);
        let state = books(&t, &key, supply, 0);
        assert_eq!(state.supply_index, SUPPLY_INDEX_FLOOR_RAW);
        assert_eq!(token.balance(&market.pool), state.cash);
    }
    let payer_before = token.balance(&payer);
    let paid = pool
        .withdraw(
            &payer,
            &false,
            &vec![
                env,
                PoolWithdrawEntry {
                    action: action(&key, supply, i128::MAX),
                    protocol_fee: 0,
                },
            ],
        )
        .get(0)
        .unwrap();
    assert_eq!(paid.actual_amount, 900_000);
    assert_eq!(token.balance(&payer), payer_before + 900_000);
    assert_eq!(paid.position.scaled_amount, 0);
    assert_eq!(books(&t, &key, 0, 0).cash, 0);
    assert_eq!(token.balance(&market.pool), 0);
}

// Independent reviewer's real SAC reproduction; no market storage injection.
#[cfg(test)]
mod fee_headroom {
    use common::constants::RAY;
    use common::types::{
        HubAssetKey, MarketParamsRaw, PoolAction, PoolSupplyEntry, PoolWithdrawEntry,
        ScaledPositionRaw,
    };
    use pool::{LiquidityPool, LiquidityPoolClient};
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{token, vec, Address, Env};

    #[test]
    fn liquidation_fee_uses_headroom_freed_by_supply_burn() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|info| {
            info.protocol_version = 27;
            info.timestamp = 1_000;
            info.sequence_number = 100;
        });
        let owner = Address::generate(&env);
        let supplier = Address::generate(&env);
        let recipient = Address::generate(&env);
        let asset = env
            .register_stellar_asset_contract_v2(owner.clone())
            .address();
        let pool = env.register(LiquidityPool, (owner.clone(),));
        let client = LiquidityPoolClient::new(&env, &pool);
        let market = HubAssetKey {
            hub_id: 0,
            asset: asset.clone(),
        };
        let params = MarketParamsRaw {
            max_borrow_rate: 2 * RAY,
            base_borrow_rate: RAY / 100,
            slope1: RAY / 10,
            slope2: RAY / 5,
            slope3: RAY / 2,
            mid_utilization: RAY / 2,
            optimal_utilization: RAY * 8 / 10,
            max_utilization: RAY * 95 / 100,
            reserve_factor: 1_000,
            is_flashloanable: false,
            flashloan_fee: 0,
            asset_id: asset.clone(),
            asset_decimals: 7,
        };
        client.create_market(&0, &params);
        let ray_per_unit = RAY / 10_000_000;
        let amount = i128::MAX / ray_per_unit;
        let gross = 100 * 10_000_000;
        let fee = 10 * 10_000_000;
        token::StellarAssetClient::new(&env, &asset).mint(&supplier, &amount);
        let tok = token::Client::new(&env, &asset);
        tok.transfer(&supplier, &pool, &amount);
        let supply = client
            .supply(&vec![
                &env,
                PoolSupplyEntry {
                    action: PoolAction {
                        hub_asset: market.clone(),
                        amount,
                        position: ScaledPositionRaw { scaled_amount: 0 },
                    },
                },
            ])
            .get(0)
            .unwrap();
        let withdrawn = client
            .withdraw(
                &recipient,
                &true,
                &vec![
                    &env,
                    PoolWithdrawEntry {
                        action: PoolAction {
                            hub_asset: market.clone(),
                            amount: gross,
                            position: supply.position,
                        },
                        protocol_fee: fee,
                    },
                ],
            )
            .get(0)
            .unwrap();
        let state = client.get_sync_data(&market).state;
        assert_eq!(withdrawn.actual_amount, gross);
        assert_eq!(tok.balance(&recipient), gross - fee);
        assert_eq!(tok.balance(&pool), amount - (gross - fee));
        assert_eq!(state.cash, amount - (gross - fee));
        assert_eq!(state.revenue, 10 * RAY);
        assert_eq!(state.supplied, amount * ray_per_unit - 90 * RAY);
        assert_eq!(client.get_revenue(&market), fee);
    }
}
