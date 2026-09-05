//! Local audit probes. No production changes. Exact scaled-share reconciliation
//! includes every position NFT, including fresh share-credit receivers.

use common::types::{AccountPositionRaw, ControllerKey, DebtPositionRaw, HubAssetKey, SeizeMode};
use position_nft::PositionNftClient;
use soroban_sdk::{token, Map};
use test_harness::{hub_asset, usd_cents, LendingTest, ALICE, BOB, CAROL, LIQUIDATOR};

const ASSETS: [&str; 3] = ["USDC", "ETH", "WBTC"];

fn account_totals(t: &LendingTest, key: &HubAssetKey) -> (i128, i128) {
    let nft = PositionNftClient::new(&t.env, &t.position_nft);
    let ids: std::vec::Vec<u64> = (0..nft.total_supply())
        .map(|i| u64::from(nft.get_token_id(&i)))
        .collect();
    t.env.as_contract(&t.controller, || {
        let storage = t.env.storage().persistent();
        let mut supplied = 0i128;
        let mut borrowed = 0i128;
        for id in ids {
            if let Some(book) = storage
                .get::<_, Map<HubAssetKey, AccountPositionRaw>>(&ControllerKey::SupplyPositions(id))
            {
                supplied += book.get(key.clone()).map_or(0, |p| p.scaled_amount);
            }
            if let Some(book) = storage
                .get::<_, Map<HubAssetKey, DebtPositionRaw>>(&ControllerKey::BorrowPositions(id))
            {
                borrowed += book.get(key.clone()).map_or(0, |p| p.scaled_amount);
            }
        }
        (supplied, borrowed)
    })
}

fn reconcile(t: &LendingTest) -> [(i128, i128, i128); 3] {
    ASSETS.map(|name| {
        let market = t.resolve_market(name);
        let key = hub_asset(market.asset.clone());
        let state = t.pool_client(name).get_sync_data(&key).state;
        let (supplied, borrowed) = account_totals(t, &key);
        assert!(state.revenue >= 0 && state.revenue <= state.supplied);
        assert!(state.cash >= 0 && state.borrowed >= 0);
        let custody = token::Client::new(&t.env, &market.asset).balance(&market.pool);
        assert_eq!(
            token::Client::new(&t.env, &market.asset).balance(&t.controller),
            0
        );
        (
            state.supplied - state.revenue - supplied,
            state.borrowed - borrowed,
            custody - state.cash,
        )
    })
}

fn fixture() -> LendingTest {
    let mut usdc = test_harness::usdc_preset();
    usdc.decimals = 6;
    let mut eth = test_harness::eth_preset();
    eth.decimals = 18;
    let mut wbtc = test_harness::wbtc_preset();
    wbtc.decimals = 8;
    let mut t = LendingTest::new()
        .with_market(usdc)
        .with_market(eth)
        .with_market(wbtc)
        .with_position_limits(3, 3)
        .build();
    t.supply(BOB, "USDC", 200_000.0);
    t.supply(BOB, "ETH", 100.0);
    t.supply(BOB, "WBTC", 10.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 0.3);
    t.borrow(ALICE, "ETH", 3.0);
    t.supply(CAROL, "USDC", 12_000.0);
    t.supply(CAROL, "WBTC", 0.01);
    t.borrow(CAROL, "ETH", 3.5);
    t
}

#[test]
fn as_audit_credit_receivers_repay_refunds_and_cleanup_conserve_exact_books() {
    let mut t = fixture();
    let baseline = reconcile(&t);
    t.advance_time(86_400 * 17);
    t.update_indexes_for(&ASSETS);
    assert_eq!(reconcile(&t), baseline);
    t.set_price("USDC", usd_cents(60));

    let credit = t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 0.5, SeizeMode::Credit(0));
    assert!(credit > 0);
    assert_eq!(reconcile(&t), baseline);
    t.assert_spoke_usage_matches_positions();

    // The same receiver accumulates a second seizure while another account is
    // liquidated through the underlying-token path.
    t.liquidate_with_mode(LIQUIDATOR, CAROL, "ETH", 0.5, SeizeMode::Credit(credit));
    t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 0.4, SeizeMode::Transfer);
    assert_eq!(reconcile(&t), baseline);
    t.assert_spoke_usage_matches_positions();

    // Large repay overpayments must not consume either collateral or old cash.
    t.repay(ALICE, "ETH", 20.0);
    t.repay(CAROL, "ETH", 20.0);
    assert_eq!(reconcile(&t), baseline);
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    let keys = t.ctrl_client().get_account_positions(&credit).0.keys();
    let withdrawals = soroban_sdk::Vec::from_iter(&t.env, keys.iter().map(|key| (key, 0i128)));
    t.ctrl_client()
        .withdraw(&liquidator, &credit, &withdrawals, &None);
    assert_eq!(reconcile(&t), baseline);
    t.assert_spoke_usage_matches_positions();

    // A small account enters through normal origination, then an external price
    // fall makes permissionless cleanup possible. No storage injection.
    t.set_price("USDC", usd_cents(100));
    t.supply("dust", "USDC", 20.0);
    t.borrow("dust", "ETH", 0.005);
    t.set_price("USDC", usd_cents(1));
    let before = t
        .pool_client("ETH")
        .get_sync_data(&hub_asset(t.resolve_asset("ETH")))
        .state;
    t.clean_bad_debt_for("dust");
    let after = t
        .pool_client("ETH")
        .get_sync_data(&hub_asset(t.resolve_asset("ETH")))
        .state;
    assert!(after.supply_index < before.supply_index);
    assert_eq!(reconcile(&t), baseline);
    t.assert_spoke_usage_matches_positions();

    // Post-loss receipt/refund accounting is tested on the changed index.
    let payer = t.get_or_create_user("recap");
    let market = t.resolve_market("ETH");
    let amount = 10i128.pow(market.decimals);
    market.token_admin.mint(&payer, &amount);
    t.ctrl_client()
        .recapitalize(&payer, &hub_asset(market.asset.clone()), &amount);
    t.supply("post_loss", "ETH", 2.0);
    t.withdraw_all("post_loss", "ETH");
    assert_eq!(reconcile(&t), baseline);
    t.assert_spoke_usage_matches_positions();
}

#[test]
fn as_audit_stateful_credit_and_transfer_sequences_keep_exact_books() {
    let mut t = fixture();
    let baseline = reconcile(&t);
    let mut rng = 0xa57a99613335u64;
    let mut ok = [0u32; 8];
    let mut rejected = [0u32; 8];
    for step in 0..160 {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let op = (rng % 8) as usize;
        let user = if (rng >> 8) & 1 == 0 { ALICE } else { CAROL };
        let accepted = match op {
            0 => t
                .try_supply(user, "USDC", 50.0 + (rng % 1500) as f64)
                .is_ok(),
            1 => t
                .try_borrow(user, "ETH", 0.01 + (rng % 40) as f64 / 100.0)
                .is_ok(),
            2 => t
                .try_repay(user, "ETH", 0.01 + (rng % 600) as f64 / 100.0)
                .is_ok(),
            3 => t
                .try_withdraw(user, "USDC", 1.0 + (rng % 2000) as f64)
                .is_ok(),
            4 | 5 => t
                .try_liquidate_with_mode(
                    LIQUIDATOR,
                    user,
                    "ETH",
                    0.05 + (rng % 200) as f64 / 100.0,
                    if op == 4 {
                        SeizeMode::Transfer
                    } else {
                        SeizeMode::Credit(0)
                    },
                )
                .is_ok(),
            6 => {
                t.advance_time(1 + rng % 86_400);
                t.set_price("USDC", usd_cents(35 + (rng % 85) as i128));
                t.try_update_indexes_for(&ASSETS).is_ok()
            }
            _ => t.try_claim_revenue("ETH").is_ok(),
        };
        if accepted {
            ok[op] += 1;
        } else {
            rejected[op] += 1;
        }
        assert_eq!(reconcile(&t), baseline, "step {step}, operation {op}");
        t.assert_spoke_usage_matches_positions();
    }
    println!("160 transitions: successful={ok:?}, rejected={rejected:?}");
    assert!(
        ok[4] > 0 && ok[5] > 0,
        "both liquidation modes must execute"
    );
    assert!(ok[0] > 0 && ok[1] > 0 && ok[2] > 0 && ok[3] > 0);
}
