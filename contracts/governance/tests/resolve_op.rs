//! Tests for the `resolve_op` arms, the `resolve_oracle` key variants and the
//! `apply_self_op` rejection path.
//!
//! Each resolve case pins the target contract, the function and the delay tier.
//! A `Sensitive` operation resolved as `Standard` executes after a shorter delay.
extern crate std;

use super::*;

use crate::constants::TIMELOCK_MIN_DELAY_LEDGERS;
use crate::test_support::register_governance;
use common::constants::RAY;
use common::types::{
    HubAssetKey, IndependencePolicy, InterestRateModel, MarketParamsRaw, OracleTolerance,
    PositionLimits,
};
use price_aggregator::PriceAggregatorClient as SeedClient;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{symbol_short, BytesN, String as SorobanString};

/// A governance instance with a native controller registered, plus that
/// controller's address. `resolve_op` reads the controller out of instance
/// storage, so every call below runs inside `env.as_contract(&gov_id, ..)`.
fn gov_with_controller(env: &Env) -> (Address, Address) {
    env.mock_all_auths();
    let (_admin, gov_id, gov) = register_governance(env);
    let controller_id = env.register(controller::Controller, (gov_id.clone(),));
    gov.set_controller(&controller_id);
    (gov_id, controller_id)
}

fn nft_args(env: &Env, hash: [u8; 32]) -> DeployPositionNftArgs {
    DeployPositionNftArgs {
        wasm_hash: BytesN::from_array(env, &hash),
        uri: SorobanString::from_str(env, "https://xoxno.com/nft/"),
        name: SorobanString::from_str(env, "XOXNO Position"),
        symbol: SorobanString::from_str(env, "XPOS"),
    }
}

#[test]
fn deploy_position_nft_resolves_to_controller_with_standard_delay() {
    let env = Env::default();
    let (gov_id, controller_id) = gov_with_controller(&env);
    let args = nft_args(&env, [0x11; 32]);
    let op = AdminOperation::DeployPositionNft(args.clone());

    let resolved = env.as_contract(&gov_id, || resolve_op(&env, &op));

    assert_eq!(resolved.target, controller_id);
    assert_eq!(resolved.function, Symbol::new(&env, "deploy_position_nft"));
    assert_eq!(
        resolved.args,
        vec![
            &env,
            args.wasm_hash.into_val(&env),
            args.uri.into_val(&env),
            args.name.into_val(&env),
            args.symbol.into_val(&env),
        ]
    );
    // Standard is safe: the controller reverts with `PositionNftAlreadyDeployed`
    // once an NFT exists. `UpgradePositionNft` has no such guard and is Sensitive.
    assert_eq!(resolved.delay_tier, DelayTier::Standard);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn deploy_position_nft_rejects_zero_wasm_hash() {
    let env = Env::default();
    let (gov_id, _controller_id) = gov_with_controller(&env);
    let op = AdminOperation::DeployPositionNft(nft_args(&env, [0u8; 32]));

    env.as_contract(&gov_id, || resolve_op(&env, &op));
}

#[test]
fn upgrade_position_nft_resolves_to_controller_with_sensitive_delay() {
    let env = Env::default();
    let (gov_id, controller_id) = gov_with_controller(&env);
    let hash = BytesN::from_array(&env, &[0x22; 32]);
    let op = AdminOperation::UpgradePositionNft(hash.clone());

    let resolved = env.as_contract(&gov_id, || resolve_op(&env, &op));

    assert_eq!(resolved.target, controller_id);
    assert_eq!(resolved.function, Symbol::new(&env, "upgrade_position_nft"));
    assert_eq!(resolved.args, vec![&env, hash.into_val(&env)]);
    // Replacing the NFT code under live positions is a Sensitive operation.
    assert_eq!(resolved.delay_tier, DelayTier::Sensitive);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn upgrade_position_nft_rejects_zero_wasm_hash() {
    let env = Env::default();
    let (gov_id, _controller_id) = gov_with_controller(&env);
    let op = AdminOperation::UpgradePositionNft(BytesN::from_array(&env, &[0u8; 32]));

    env.as_contract(&gov_id, || resolve_op(&env, &op));
}

/// `UpgradePriceAggregator` resolves to the aggregator's `upgrade` on the
/// Sensitive tier. Oracle configuration (`ConfigureAssetOracle`) stays Standard.
#[test]
fn upgrade_price_aggregator_resolves_to_the_aggregator_with_sensitive_delay() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, gov_id, gov) = register_governance(&env);
    let agg_id = env.register(price_aggregator::PriceAggregator, (gov_id.clone(),));
    gov.set_price_aggregator(&agg_id);

    let hash = BytesN::from_array(&env, &[0x33; 32]);
    let op = AdminOperation::UpgradePriceAggregator(hash.clone());
    let resolved = env.as_contract(&gov_id, || resolve_op(&env, &op));

    assert_eq!(resolved.target, agg_id, "must target the price aggregator");
    assert_eq!(resolved.function, Symbol::new(&env, "upgrade"));
    assert_eq!(resolved.args, vec![&env, hash.into_val(&env)]);
    assert_eq!(resolved.delay_tier, DelayTier::Sensitive);
}

/// A zero wasm hash is rejected with `InvalidWasmHash`, as for every other
/// upgrade variant.
#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn upgrade_price_aggregator_rejects_zero_wasm_hash() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, gov_id, gov) = register_governance(&env);
    let agg_id = env.register(price_aggregator::PriceAggregator, (gov_id.clone(),));
    gov.set_price_aggregator(&agg_id);

    let op = AdminOperation::UpgradePriceAggregator(BytesN::from_array(&env, &[0u8; 32]));
    env.as_contract(&gov_id, || resolve_op(&env, &op));
}

#[test]
fn force_socialize_bad_debt_resolves_to_controller_with_sensitive_delay() {
    let env = Env::default();
    let (gov_id, controller_id) = gov_with_controller(&env);
    let op = AdminOperation::ForceSocializeBadDebt(42);

    let resolved = env.as_contract(&gov_id, || resolve_op(&env, &op));

    assert_eq!(resolved.target, controller_id);
    assert_eq!(
        resolved.function,
        Symbol::new(&env, "force_socialize_bad_debt")
    );
    assert_eq!(resolved.args.len(), 1);
    // Socializing an account's debt into the supply index is a Sensitive operation.
    assert_eq!(resolved.delay_tier, DelayTier::Sensitive);
}

fn relax_args(env: &Env, expected_epoch: u64) -> RelaxSpokeAssetFlagsArgs {
    RelaxSpokeAssetFlagsArgs {
        spoke_id: 1,
        hub_asset: HubAssetKey {
            hub_id: 0,
            asset: Address::generate(env),
        },
        expected_epoch,
        paused: false,
        frozen: true,
        no_seize: false,
    }
}

#[test]
fn relax_spoke_asset_flags_resolves_to_controller_with_standard_delay() {
    let env = Env::default();
    let (gov_id, controller_id) = gov_with_controller(&env);
    let args = relax_args(&env, 0);
    let op = AdminOperation::RelaxSpokeAssetFlags(args.clone());

    let resolved = env.as_contract(&gov_id, || resolve_op(&env, &op));

    assert_eq!(resolved.target, controller_id);
    assert_eq!(
        resolved.function,
        Symbol::new(&env, "relax_spoke_asset_flags")
    );
    assert_eq!(
        resolved.args,
        vec![
            &env,
            args.spoke_id.into_val(&env),
            args.hub_asset.into_val(&env),
            args.expected_epoch.into_val(&env),
            args.paused.into_val(&env),
            args.frozen.into_val(&env),
            args.no_seize.into_val(&env),
        ]
    );
    assert_eq!(resolved.delay_tier, DelayTier::Standard);
}

#[test]
fn relax_spoke_asset_flags_keeps_each_flag_in_its_argument_position() {
    let env = Env::default();
    let (gov_id, _controller_id) = gov_with_controller(&env);

    for (paused, frozen, no_seize) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let args = RelaxSpokeAssetFlagsArgs {
            paused,
            frozen,
            no_seize,
            ..relax_args(&env, 0)
        };
        let op = AdminOperation::RelaxSpokeAssetFlags(args.clone());

        let resolved = env.as_contract(&gov_id, || resolve_op(&env, &op));

        assert_eq!(
            resolved.args,
            vec![
                &env,
                args.spoke_id.into_val(&env),
                args.hub_asset.into_val(&env),
                args.expected_epoch.into_val(&env),
                paused.into_val(&env),
                frozen.into_val(&env),
                no_seize.into_val(&env),
            ]
        );
    }
}

#[test]
#[should_panic(expected = "Error(Contract, #319)")]
fn relax_spoke_asset_flags_rejects_a_future_epoch_at_proposal() {
    let env = Env::default();
    let (gov_id, _controller_id) = gov_with_controller(&env);
    let op = AdminOperation::RelaxSpokeAssetFlags(relax_args(&env, 1));

    env.as_contract(&gov_id, || resolve_op(&env, &op));
}

#[test]
fn resolve_oracle_zeroes_decimals_for_a_ref_key() {
    let env = Env::default();
    // A `Ref` key has no token contract to read decimals from, so the input
    // value is discarded and a non-zero input still resolves to 0.
    let oracle = AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds: 900,
        sources: soroban_sdk::vec![&env],
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: 1,
        max_sanity_price_wad: i128::MAX,
    };

    let resolved = resolve_oracle(&env, &PriceKey::Ref(symbol_short!("XLMUSD")), &oracle);

    assert_eq!(resolved.asset_decimals, 0);
    // Everything other than the decimals is carried through untouched.
    assert_eq!(resolved.max_price_stale_seconds, 900);
    assert_eq!(resolved.min_sanity_price_wad, 1);
}

#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn apply_self_op_rejects_an_operation_that_does_not_target_governance() {
    let env = Env::default();
    let (gov_id, _controller_id) = gov_with_controller(&env);
    // `Unpause` resolves to the controller, so `execute_self` never routes it
    // here. Reaching this arm is an internal invariant failure (`InternalError`).
    env.as_contract(&gov_id, || apply_self_op(&env, &AdminOperation::Unpause));
}

fn token_oracle(env: &Env, asset_decimals: u32) -> AssetOracle {
    AssetOracle {
        asset_decimals,
        max_price_stale_seconds: 900,
        sources: vec![env],
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: 1,
        max_sanity_price_wad: i128::MAX,
    }
}

fn pool_args(asset: &Address, asset_decimals: u32) -> CreatePoolArgs {
    CreatePoolArgs {
        hub_id: 1,
        asset: asset.clone(),
        params: MarketParamsRaw {
            max_borrow_rate: RAY,
            base_borrow_rate: 0,
            slope1: RAY / 100,
            slope2: RAY / 10,
            slope3: RAY / 2,
            mid_utilization: RAY / 2,
            optimal_utilization: 8 * RAY / 10,
            max_utilization: 95 * RAY / 100,
            reserve_factor: 1_000,
            is_flashloanable: false,
            flashloan_fee: 0,
            asset_id: asset.clone(),
            asset_decimals,
        },
    }
}

fn gov_with_relabelled_sac(env: &Env, listed: u32) -> (Address, Address, Address) {
    env.mock_all_auths();
    let (_admin, gov_id, gov) = register_governance(env);
    let controller_id = env.register(controller::Controller, (gov_id.clone(),));
    gov.set_controller(&controller_id);
    let agg_id = env.register(price_aggregator::PriceAggregator, (gov_id.clone(),));
    gov.set_price_aggregator(&agg_id);
    let sac = env
        .register_stellar_asset_contract_v2(Address::generate(env))
        .address();
    SeedClient::new(env, &agg_id)
        .seed_oracle(&PriceKey::Token(sac.clone()), &token_oracle(env, listed));
    (gov_id, controller_id, sac)
}

#[test]
fn resolve_oracle_keeps_the_stored_unit_of_a_relabelled_token() {
    let env = Env::default();
    let (gov_id, _controller_id, sac) = gov_with_relabelled_sac(&env, 0);

    let resolved = env.as_contract(&gov_id, || {
        resolve_oracle(&env, &PriceKey::Token(sac.clone()), &token_oracle(&env, 7))
    });

    assert_eq!(resolved.asset_decimals, 0);
}

#[test]
fn create_liquidity_pool_accepts_the_stored_unit_of_a_relabelled_token() {
    let env = Env::default();
    let (gov_id, controller_id, sac) = gov_with_relabelled_sac(&env, 0);
    let args = pool_args(&sac, 0);
    let op = AdminOperation::CreateLiquidityPool(args.clone());

    let resolved = env.as_contract(&gov_id, || resolve_op(&env, &op));

    assert_eq!(resolved.target, controller_id);
    assert_eq!(
        resolved.function,
        Symbol::new(&env, "create_liquidity_pool")
    );
    assert_eq!(resolved.delay_tier, DelayTier::Standard);
    assert_eq!(
        resolved.args,
        vec![
            &env,
            args.hub_id.into_val(&env),
            args.asset.into_val(&env),
            args.params.into_val(&env),
        ]
    );
}

#[test]
fn pool_parameter_upgrade_resolves_to_the_controller_with_exact_arguments() {
    let env = Env::default();
    let (gov_id, controller_id) = gov_with_controller(&env);
    let params = InterestRateModel {
        max_borrow_rate: RAY,
        base_borrow_rate: 0,
        slope1: RAY / 100,
        slope2: RAY / 10,
        slope3: RAY / 2,
        mid_utilization: RAY / 2,
        optimal_utilization: 8 * RAY / 10,
        max_utilization: 95 * RAY / 100,
        reserve_factor: 1_000,
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    let args = UpgradePoolParamsArgs {
        hub_asset: HubAssetKey {
            hub_id: 2,
            asset: Address::generate(&env),
        },
        params,
    };

    let resolved = env.as_contract(&gov_id, || {
        resolve_op(
            &env,
            &AdminOperation::UpgradeLiquidityPoolParams(args.clone()),
        )
    });

    assert_eq!(resolved.target, controller_id);
    assert_eq!(
        resolved.function,
        Symbol::new(&env, "upgrade_liquidity_pool_params")
    );
    assert_eq!(resolved.delay_tier, DelayTier::Standard);
    assert_eq!(
        resolved.args,
        vec![
            &env,
            args.hub_asset.into_val(&env),
            args.params.into_val(&env),
        ]
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn create_liquidity_pool_rejects_the_live_unit_of_a_relabelled_token() {
    let env = Env::default();
    let (gov_id, _controller_id, sac) = gov_with_relabelled_sac(&env, 0);
    let op = AdminOperation::CreateLiquidityPool(pool_args(&sac, 7));

    env.as_contract(&gov_id, || resolve_op(&env, &op));
}

#[test]
fn controller_operations_resolve_their_exact_call_and_delay() {
    let env = Env::default();
    let (gov_id, controller_id) = gov_with_controller(&env);
    let other = Address::generate(&env);
    let swap = env.register(controller::Controller, (gov_id.clone(),));
    let key = HubAssetKey {
        hub_id: 2,
        asset: other.clone(),
    };
    let hash = BytesN::from_array(&env, &[7; 32]);
    let limits = PositionLimits {
        max_supply_positions: 5,
        max_borrow_positions: 4,
    };
    let ownership = TransferOwnershipArgs {
        new_owner: controller_id.clone(),
        live_until_ledger: 1_000_000,
    };

    let cases: std::vec::Vec<(&str, AdminOperation, &str, DelayTier, Vec<Val>)> = std::vec![
        (
            "set swap aggregator",
            AdminOperation::SetSwapAggregator(swap.clone()),
            "set_swap_aggregator",
            DelayTier::Sensitive,
            vec![&env, swap.into_val(&env)],
        ),
        (
            "set accumulator",
            AdminOperation::SetAccumulator(other.clone()),
            "set_accumulator",
            DelayTier::Standard,
            vec![&env, other.clone().into_val(&env)],
        ),
        (
            "position limits",
            AdminOperation::SetPositionLimits(limits.clone()),
            "set_position_limits",
            DelayTier::Standard,
            vec![&env, limits.into_val(&env)],
        ),
        (
            "borrow collateral floor",
            AdminOperation::SetMinBorrowCollateralUsd(0),
            "set_min_borrow_collateral_usd",
            DelayTier::Standard,
            vec![&env, 0i128.into_val(&env)],
        ),
        (
            "create hub",
            AdminOperation::CreateHub,
            "create_hub",
            DelayTier::Standard,
            vec![&env],
        ),
        (
            "add spoke",
            AdminOperation::AddSpoke,
            "add_spoke",
            DelayTier::Standard,
            vec![&env],
        ),
        (
            "remove spoke",
            AdminOperation::RemoveSpoke(3),
            "remove_spoke",
            DelayTier::Standard,
            vec![&env, 3u32.into_val(&env)],
        ),
        (
            "remove spoke asset",
            AdminOperation::RemoveAssetFromSpoke(RemoveAssetFromSpokeArgs {
                hub_asset: key.clone(),
                spoke_id: 3,
            }),
            "remove_asset_from_spoke",
            DelayTier::Standard,
            vec![&env, key.clone().into_val(&env), 3u32.into_val(&env)],
        ),
        (
            "approve blend pool",
            AdminOperation::ApproveBlendPool(controller_id.clone()),
            "approve_blend_pool",
            DelayTier::Standard,
            vec![&env, controller_id.clone().into_val(&env)],
        ),
        (
            "revoke blend pool",
            AdminOperation::RevokeBlendPool(controller_id.clone()),
            "revoke_blend_pool",
            DelayTier::Standard,
            vec![&env, controller_id.clone().into_val(&env)],
        ),
        (
            "deploy pool",
            AdminOperation::DeployPool(hash.clone()),
            "deploy_pool",
            DelayTier::Standard,
            vec![&env, hash.clone().into_val(&env)],
        ),
        (
            "upgrade pool",
            AdminOperation::UpgradePool(hash.clone()),
            "upgrade_pool",
            DelayTier::Sensitive,
            vec![&env, hash.clone().into_val(&env)],
        ),
        (
            "set position manager",
            AdminOperation::SetPositionManager(other.clone(), true),
            "set_position_manager",
            DelayTier::Sensitive,
            vec![&env, other.clone().into_val(&env), true.into_val(&env)],
        ),
        (
            "upgrade controller",
            AdminOperation::UpgradeController(hash.clone()),
            "upgrade",
            DelayTier::Sensitive,
            vec![&env, hash.into_val(&env)],
        ),
        (
            "migrate controller",
            AdminOperation::MigrateController(2),
            "migrate",
            DelayTier::Standard,
            vec![&env, 2u32.into_val(&env)],
        ),
        (
            "transfer controller ownership",
            AdminOperation::TransferCtrlOwnership(ownership.clone()),
            "transfer_ownership",
            DelayTier::Sensitive,
            vec![
                &env,
                ownership.new_owner.into_val(&env),
                ownership.live_until_ledger.into_val(&env),
            ],
        ),
        (
            "force socialize bad debt",
            AdminOperation::ForceSocializeBadDebt(42),
            "force_socialize_bad_debt",
            DelayTier::Sensitive,
            vec![&env, 42u64.into_val(&env)],
        ),
        (
            "set liquidation curve",
            AdminOperation::SetSpokeLiquidationCurve(SpokeLiquidationCurveArgs {
                spoke_id: 3,
                target_hf_wad: 1_020_000_000_000_000_000,
                hf_for_max_bonus_wad: 510_000_000_000_000_000,
                liquidation_bonus_factor_bps: 10_000,
            }),
            "set_spoke_liquidation_curve",
            DelayTier::Standard,
            vec![
                &env,
                3u32.into_val(&env),
                1_020_000_000_000_000_000i128.into_val(&env),
                510_000_000_000_000_000i128.into_val(&env),
                10_000u32.into_val(&env),
            ],
        ),
        (
            "unpause",
            AdminOperation::Unpause,
            "unpause",
            DelayTier::Standard,
            vec![&env],
        ),
    ];

    for (label, op, function, delay_tier, args) in cases {
        let resolved = env.as_contract(&gov_id, || resolve_op(&env, &op));
        assert_eq!(resolved.target, controller_id, "{label}: target");
        assert_eq!(
            resolved.function,
            Symbol::new(&env, function),
            "{label}: function"
        );
        assert_eq!(resolved.delay_tier, delay_tier, "{label}: delay");
        assert_eq!(resolved.args, args, "{label}: arguments");
    }
}

#[test]
fn oracle_operations_resolve_to_the_aggregator_with_exact_arguments() {
    let env = Env::default();
    let (gov_id, _controller_id) = gov_with_controller(&env);
    let aggregator = env.register(price_aggregator::PriceAggregator, (gov_id.clone(),));
    crate::GovernanceClient::new(&env, &gov_id).set_price_aggregator(&aggregator);

    let key = PriceKey::Ref(symbol_short!("XLMUSD"));
    let oracle = token_oracle(&env, 7);
    let resolved_oracle = resolve_oracle(&env, &key, &oracle);
    assert_eq!(resolved_oracle.asset_decimals, 0);

    let cases = [
        (
            "configure oracle",
            AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
                key: key.clone(),
                oracle,
            }),
            "set_oracle",
            vec![
                &env,
                key.clone().into_val(&env),
                resolved_oracle.into_val(&env),
            ],
        ),
        (
            "edit tolerance",
            AdminOperation::EditOracleTolerance(EditToleranceArgs {
                key: key.clone(),
                tolerance: 500,
            }),
            "set_tolerance",
            vec![
                &env,
                key.into_val(&env),
                OracleTolerance {
                    upper_ratio_bps: 10_500,
                    lower_ratio_bps: 9_524,
                }
                .into_val(&env),
            ],
        ),
    ];

    for (label, op, function, args) in cases {
        let resolved = env.as_contract(&gov_id, || resolve_op(&env, &op));
        assert_eq!(resolved.target, aggregator, "{label}: target");
        assert_eq!(
            resolved.function,
            Symbol::new(&env, function),
            "{label}: function"
        );
        assert_eq!(resolved.delay_tier, DelayTier::Standard, "{label}: delay");
        assert_eq!(resolved.args, args, "{label}: arguments");
    }
}

#[test]
fn curator_operations_resolve_to_the_supplied_contract_with_exact_arguments() {
    let env = Env::default();
    let (gov_id, _controller_id) = gov_with_controller(&env);
    // Resolution only needs a deployed contract address; execution is covered
    // by the curator flow tests.
    let curator = env.register(controller::Controller, (gov_id.clone(),));
    let admin = Address::generate(&env);
    let hash = BytesN::from_array(&env, &[0x44; 32]);
    let registrar = Some(2u32);

    let cases = [
        (
            "upgrade curator",
            AdminOperation::UpgradeCurator(curator.clone(), hash.clone()),
            "upgrade",
            vec![&env, hash.into_val(&env)],
        ),
        (
            "suspend registrar",
            AdminOperation::SuspendCuratorRegistrar(curator.clone(), 2, true),
            "set_registrar_suspended",
            vec![&env, 2u32.into_val(&env), true.into_val(&env)],
        ),
        (
            "reassign hub",
            AdminOperation::ReassignCuratorHub(curator.clone(), 3, registrar),
            "reassign_hub",
            vec![&env, 3u32.into_val(&env), registrar.into_val(&env)],
        ),
        (
            "reassign spoke",
            AdminOperation::ReassignCuratorSpoke(curator.clone(), 4, None),
            "reassign_spoke",
            vec![&env, 4u32.into_val(&env), None::<u32>.into_val(&env)],
        ),
        (
            "register registrar",
            AdminOperation::RegisterCuratorRegistrar(curator.clone(), admin.clone()),
            "register_registrar",
            vec![&env, admin.clone().into_val(&env)],
        ),
        (
            "set registrar admin",
            AdminOperation::SetCuratorRegistrarAdmin(curator.clone(), 2, admin.clone()),
            "force_set_admin",
            vec![&env, 2u32.into_val(&env), admin.into_val(&env)],
        ),
    ];

    for (label, op, function, args) in cases {
        let resolved = env.as_contract(&gov_id, || resolve_op(&env, &op));
        assert_eq!(resolved.target, curator, "{label}: target");
        assert_eq!(
            resolved.function,
            Symbol::new(&env, function),
            "{label}: function"
        );
        assert_eq!(resolved.delay_tier, DelayTier::Sensitive, "{label}: delay");
        assert_eq!(resolved.args, args, "{label}: arguments");
    }
}

#[test]
fn governance_operations_resolve_to_themselves_with_exact_arguments() {
    let env = Env::default();
    let (gov_id, _controller_id) = gov_with_controller(&env);
    let aggregator = env.register(price_aggregator::PriceAggregator, (gov_id.clone(),));
    let hash = BytesN::from_array(&env, &[0x55; 32]);
    let account = Address::generate(&env);
    let role = Symbol::new(&env, "PROPOSER");
    let ownership = TransferOwnershipArgs {
        new_owner: account.clone(),
        live_until_ledger: 1_000_000,
    };

    let cases = [
        (
            "upgrade governance",
            AdminOperation::UpgradeGov(hash.clone()),
            "upgrade",
            DelayTier::Sensitive,
            vec![&env, hash.into_val(&env)],
        ),
        (
            "update delay",
            AdminOperation::UpdateGovDelay(TIMELOCK_MIN_DELAY_LEDGERS),
            "update_delay",
            DelayTier::Standard,
            vec![&env, TIMELOCK_MIN_DELAY_LEDGERS.into_val(&env)],
        ),
        (
            "grant role",
            AdminOperation::GrantGovRole(RoleArgs {
                account: account.clone(),
                role: role.clone(),
            }),
            "grant_role",
            DelayTier::Sensitive,
            vec![
                &env,
                account.clone().into_val(&env),
                role.clone().into_val(&env),
            ],
        ),
        (
            "revoke role",
            AdminOperation::RevokeGovRole(RoleArgs {
                account: account.clone(),
                role: role.clone(),
            }),
            "revoke_role",
            DelayTier::Sensitive,
            vec![&env, account.clone().into_val(&env), role.into_val(&env)],
        ),
        (
            "transfer ownership",
            AdminOperation::TransferGovOwnership(ownership.clone()),
            "transfer_ownership",
            DelayTier::Sensitive,
            vec![
                &env,
                ownership.new_owner.into_val(&env),
                ownership.live_until_ledger.into_val(&env),
            ],
        ),
        (
            "set price aggregator",
            AdminOperation::SetPriceAggregator(aggregator.clone()),
            "set_price_aggregator",
            DelayTier::Sensitive,
            vec![&env, aggregator.into_val(&env)],
        ),
    ];

    for (label, op, function, delay_tier, args) in cases {
        let resolved = env.as_contract(&gov_id, || resolve_op(&env, &op));
        assert_eq!(resolved.target, gov_id, "{label}: target");
        assert_eq!(
            resolved.function,
            Symbol::new(&env, function),
            "{label}: function"
        );
        assert_eq!(resolved.delay_tier, delay_tier, "{label}: delay");
        assert_eq!(resolved.args, args, "{label}: arguments");
    }
}
