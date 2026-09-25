//! Multi-position liquidation coverage.
//!
//! Liquidating an account with 5 supply and 5 borrow positions (the 5 market
//! presets the harness ships) completes without a logic or arithmetic panic.
//!
//! The native logic-panic test does not assert a transaction-budget fit. Under
//! `mock_all_auths_allowing_non_root_auth` the host meters its own auth
//! re-verification against the budget, a cost that a signed transaction does not
//! pay. The live-testnet integration suite measures the liquidation budget
//! (`flow_stress_liq_frontier` in `tests/integration/flows/stress.sh`).
//! `classify_panic` accepts a budget panic and re-raises every other panic.
//! The separate WASM Credit replay below enforces the measured CPU allowance.

use controller::constants::WAD;
use test_harness::{
    eth_preset, usdc_preset, usdt_stable_preset, wbtc_preset, xlm_preset, LendingTest, ALICE,
    LIQUIDATOR,
};

fn build_ctx() -> LendingTest {
    LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market(xlm_preset())
        .with_position_limits(5, 5)
        .with_budget_enabled()
        .build()
}

fn classify_panic(payload: Box<dyn std::any::Any + Send>) -> Result<(), std::string::String> {
    let msg = if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<std::string::String>() {
        s.clone()
    } else {
        std::format!("{:?}", payload.type_id())
    };

    let low = msg.to_lowercase();
    let is_overflow = low.contains("overflow") || low.contains("out of bounds");
    let is_budget = !is_overflow
        && (low.contains("budget exceeded")
            || low.contains("exceededlimit")
            || low.contains("cpu instruction")
            || low.contains("memory limit")
            || low.contains("read entries")
            || low.contains("write entries")
            || low.contains("tx size"));
    if is_budget {
        Ok(())
    } else {
        Err(msg)
    }
}

#[test]
fn liquidate_5_supply_5_borrow_completes_without_logic_panic() {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut t = build_ctx();

        t.supply(ALICE, "USDC", 8_000.0);
        t.supply(ALICE, "USDT", 8_000.0);
        t.supply(ALICE, "XLM", 80_000.0);
        t.supply(ALICE, "ETH", 0.01);
        t.supply(ALICE, "WBTC", 0.001);

        t.supply("BOOT", "ETH", 10.0);
        t.supply("BOOT", "WBTC", 1.0);

        t.borrow(ALICE, "ETH", 0.4);
        t.borrow(ALICE, "WBTC", 0.01);
        t.borrow(ALICE, "USDC", 10.0);
        t.borrow(ALICE, "USDT", 10.0);
        t.borrow(ALICE, "XLM", 100.0);

        t.set_price("USDC", WAD / 100);
        t.set_price("USDT", WAD / 100);
        t.set_price("XLM", WAD / 100);

        t.assert_liquidatable(ALICE);

        let payments: &[(&str, f64)] = &[
            ("USDC", 1.0),
            ("USDT", 1.0),
            ("ETH", 0.04),
            ("WBTC", 0.001),
            ("XLM", 10.0),
        ];

        t.liquidate_multi(LIQUIDATOR, ALICE, payments);
    }));

    match outcome {
        Ok(()) => {}
        Err(payload) => {
            classify_panic(payload).unwrap_or_else(|msg| {
                panic!(
                    "BENCH FAILURE: liquidate setup or call panicked outside the budget envelope: {}",
                    msg
                )
            });
        }
    }
}

/// Fails when the ctx position limits exceed the 5 legs the scenario above
/// exercises. The harness ships 5 market presets, so add presets and extend the
/// scenario before raising the limits.
#[test]
fn test_scenario_covers_every_configured_leg() {
    let scenario_legs = 5u32;
    let limits = build_ctx().get_position_limits();
    assert!(
        limits.max_supply_positions <= scenario_legs
            && limits.max_borrow_positions <= scenario_legs,
        "liquidation scenario exercises {sc}/{sc} legs but the ctx permits {}/{} — \
         add market presets and extend the scenario before widening the ctx limits",
        limits.max_supply_positions,
        limits.max_borrow_positions,
        sc = scenario_legs,
    );
}

/// Replay the same 5+5 Credit authorization against one book at zero and five
/// seconds elapsed. Production WASM meters both projection and committed accrual;
/// source-account authorization avoids the mock-auth recorder's extra CPU work.
#[test]
fn credit_5coll_5debt_same_ledger_to_next_ledger_cpu() {
    use common::types::SeizeMode;
    use soroban_sdk::testutils::{Ledger, MockAuthInvoke};
    use soroban_sdk::{xdr, Address, Bytes, Env, IntoVal, TryIntoVal, Vec};
    use test_harness::{hub_asset, usd, MarketPreset};

    let names = ["C0", "C1", "C2", "C3", "C4", "D0", "D1", "D2", "D3", "D4"];
    let mut builder = LendingTest::new().with_position_limits(5, 5);
    for name in names {
        builder = builder.with_market(MarketPreset {
            name,
            decimals: 7,
            price_wad: usd(1),
            ..usdc_preset()
        });
    }
    let mut t = builder.build();
    for name in &names[..5] {
        t.supply(ALICE, name, 1_000.0);
    }
    for name in &names[5..] {
        t.borrow(ALICE, name, 600.0);
    }
    for name in &names[..5] {
        t.set_price(name, usd(1) * 6 / 10);
    }
    let account_id = t.resolve_account_id(ALICE);
    let source = xdr::AccountId(xdr::PublicKey::PublicKeyTypeEd25519(xdr::Uint256([71; 32])));
    let liquidator: Address = xdr::ScAddress::Account(source.clone())
        .try_into_val(&t.env)
        .unwrap();
    for name in names {
        let token_name = soroban_sdk::token::Client::new(&t.env, &t.resolve_asset(name))
            .name()
            .to_string();
        let (code, issuer) = token_name.split_once(':').unwrap();
        let xdr::ScAddress::Account(issuer) =
            xdr::ScAddress::from(&Address::from_str(&t.env, issuer))
        else {
            panic!("SAC issuer must be an account")
        };
        let mut asset_code = [0; 4];
        asset_code[..code.len()].copy_from_slice(code.as_bytes());
        let asset = xdr::TrustLineAsset::CreditAlphanum4(xdr::AlphaNum4 {
            asset_code: xdr::AssetCode4(asset_code),
            issuer,
        });
        let key = xdr::LedgerKey::Trustline(xdr::LedgerKeyTrustLine {
            account_id: source.clone(),
            asset: asset.clone(),
        });
        let entry = xdr::LedgerEntry {
            last_modified_ledger_seq: 0,
            ext: xdr::LedgerEntryExt::V0,
            data: xdr::LedgerEntryData::Trustline(xdr::TrustLineEntry {
                account_id: source.clone(),
                asset,
                balance: 0,
                limit: i64::MAX,
                flags: 1,
                ext: xdr::TrustLineEntryExt::V0,
            }),
        };
        t.env
            .host()
            .add_ledger_entry(&std::rc::Rc::new(key), &std::rc::Rc::new(entry), None)
            .unwrap();
    }
    let mut payments = Vec::new(&t.env);
    for name in &names[5..] {
        let market = t.resolve_market(name);
        market.token_admin.mint(&liquidator, &2_000_000_000);
        payments.push_back((hub_asset(market.asset.clone()), 1_000_000_000i128));
    }
    t.ctrl_client()
        .liquidate(&liquidator, &account_id, &payments, &SeizeMode::Transfer);
    let positions = t.ctrl_client().get_account_positions(&account_id);
    assert_eq!((positions.0.len(), positions.1.len()), (5, 5));
    for name in names {
        assert_eq!(
            t.pool_client(name)
                .get_sync_data(&hub_asset(t.resolve_asset(name)))
                .state
                .last_timestamp,
            t.env.ledger().timestamp() * 1000,
            "simulation starts with a synchronized market: {name}"
        );
    }

    // Replace native test dispatch after setup, preserving the initialized book.
    // Fresh replay environments below discard native controller dispatch tables.
    let wasm_dir = std::env::var_os("STRESS_BUDGET_WASM_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/wasm/deploy")
        });
    for (address, filename) in [
        (&t.controller, "controller.wasm"),
        (&t.price_aggregator, "price_aggregator.wasm"),
        (&t.resolve_market("D0").pool, "pool.wasm"),
        (&t.position_nft, "position_nft.wasm"),
    ] {
        let wasm = std::fs::read(wasm_dir.join(filename)).unwrap();
        let hash = t
            .env
            .deployer()
            .upload_contract_wasm(Bytes::from_slice(&t.env, &wasm));
        t.env.as_contract(address, || {
            t.env
                .deployer()
                .update_current_contract(soroban_sdk::ContractExecutable::Wasm(hash))
        });
    }
    let snapshot = t.env.to_ledger_snapshot();
    // Validators cache parsed WASM outside the transaction's CPU allowance.
    t.env
        .host()
        .ensure_module_cache_contains_host_storage_contracts()
        .unwrap();
    let module_cache = t.env.host().take_module_cache().unwrap();
    let mut costs = std::vec::Vec::new();
    for (delay, leeway) in [
        (0, None),
        (5, None),
        (5, Some(2_000_000)),
        (5, Some(20_000_000)),
    ] {
        let env = Env::from_ledger_snapshot(snapshot.clone());
        env.host().set_module_cache(module_cache.clone()).unwrap();
        env.cost_estimate().budget().reset_unlimited();
        env.cost_estimate().disable_resource_limits();
        let rebind = |address: &Address| -> Address {
            xdr::ScAddress::from(address).try_into_val(&env).unwrap()
        };
        let controller = rebind(&t.controller);
        env.register_at(
            &rebind(&t.mock_reflector),
            test_harness::mock_reflector::MockReflector,
            (),
        );
        let signer = rebind(&liquidator);
        let mut offered = Vec::new(&env);
        let mut transfers = std::vec::Vec::new();
        let assets: std::vec::Vec<_> = names[5..]
            .iter()
            .map(|name| rebind(&t.resolve_asset(name)))
            .collect();
        let pool = rebind(&t.resolve_market("D0").pool);
        for asset in &assets {
            offered.push_back((hub_asset(asset.clone()), 1_000_000_000i128));
            transfers.push(MockAuthInvoke {
                contract: asset,
                fn_name: "transfer",
                args: (signer.clone(), pool.clone(), 1_000_000_000i128).into_val(&env),
                sub_invokes: &[],
            });
        }
        let auth = xdr::SorobanAuthorizationEntry {
            credentials: xdr::SorobanCredentials::SourceAccount,
            root_invocation: (&MockAuthInvoke {
                contract: &controller,
                fn_name: "liquidate",
                args: (
                    signer.clone(),
                    account_id,
                    offered.clone(),
                    SeizeMode::Credit(0),
                )
                    .into_val(&env),
                sub_invokes: &transfers,
            })
                .into(),
        };
        env.host().set_source_account(source.clone()).unwrap();
        env.set_auths(&[auth]);
        env.ledger().with_mut(|info| {
            info.timestamp += delay;
            info.sequence_number += (delay / 5) as u32;
        });
        env.cost_estimate().budget().reset_unlimited();
        if let Some(leeway) = leeway {
            env.cost_estimate()
                .budget()
                .reset_limits(costs[0] + leeway, 41_943_040);
            // Only uncharged test/debug auth snapshots get unlimited shadow
            // budget; normal execution CPU and memory remain bounded above.
            env.host()
                .set_shadow_budget_limits(u64::MAX, u64::MAX)
                .unwrap();
        }
        let client = controller::ControllerClient::new(&env, &controller);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.try_liquidate(&signer, &account_id, &offered, &SeizeMode::Credit(0))
        }));
        let cpu = env.cost_estimate().budget().cpu_instruction_cost();
        env.cost_estimate().budget().reset_unlimited();
        std::println!("credit_5coll_5debt delay={delay}s leeway={leeway:?} cpu={cpu}");
        if leeway == Some(2_000_000) {
            let payload = result.expect_err("old allowance must exhaust execution CPU");
            let message = payload
                .downcast_ref::<std::string::String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("");
            assert!(
                message.contains("HostError: Error(Budget, ExceededLimit)"),
                "unexpected failure: {message}"
            );
            assert!(cpu > costs[0] + 2_000_000);
            continue;
        }
        let receiver = result
            .expect("budgeted replay must not panic")
            .expect("host invocation must succeed")
            .expect("u64 result");
        if leeway.is_none() {
            costs.push(cpu);
        }
        assert!(receiver > 0);
        assert_eq!(client.get_account_positions(&receiver).0.len(), 5);
        assert_eq!(client.get_account_positions(&account_id).1.len(), 5);
    }
    let delta = costs[1]
        .checked_sub(costs[0])
        .expect("later inclusion must execute accrual");
    std::println!("credit_5coll_5debt zero_to_five_second_cpu_delta={delta}");
    assert!(
        delta > 2_000_000,
        "negative control must exceed the former leeway"
    );
    assert!(
        costs[0] + 20_000_000 <= 360_000_000,
        "retain 10% of the captured 400m network ceiling"
    );
    assert!(
        delta <= 20_000_000,
        "accrual delta exceeds native instruction leeway"
    );
}
