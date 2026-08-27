//! The `resolve_op` arms, `resolve_oracle` key variants and `apply_self_op`
//! rejection path that no other governance test reaches.
//!
//! Each resolve case pins the three properties the timelock depends on: the
//! contract the operation is dispatched to, the function it calls, and the
//! delay tier that gates it. The tier is the security-relevant one -- an
//! operation classified `Standard` when it should be `Sensitive` executes
//! after a shorter timelock than its blast radius warrants.
extern crate std;

use super::*;

use crate::test_support::register_governance;
use common::types::{IndependencePolicy, OracleTolerance};
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
    let op = AdminOperation::DeployPositionNft(nft_args(&env, [0x11; 32]));

    let resolved = env.as_contract(&gov_id, || resolve_op(&env, &op));

    assert_eq!(resolved.target, controller_id);
    assert_eq!(resolved.function, Symbol::new(&env, "deploy_position_nft"));
    // wasm_hash, uri, name, symbol -- dropping one would silently deploy an
    // NFT with a shifted argument list.
    assert_eq!(resolved.args.len(), 4);
    // Standard, while UpgradePositionNft next door is Sensitive. That asymmetry
    // is deliberate and safe: markets::deploy_position_nft asserts
    // try_get_position_nft(env).is_none() and reverts with
    // PositionNftAlreadyDeployed otherwise, so this is a one-shot bootstrap
    // that cannot re-point the NFT of a protocol with live positions. The
    // upgrade path has no such guard, which is why it carries the longer delay.
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
    let op = AdminOperation::UpgradePositionNft(BytesN::from_array(&env, &[0x22; 32]));

    let resolved = env.as_contract(&gov_id, || resolve_op(&env, &op));

    assert_eq!(resolved.target, controller_id);
    assert_eq!(resolved.function, Symbol::new(&env, "upgrade_position_nft"));
    assert_eq!(resolved.args.len(), 1);
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

/// The price aggregator is the only source of prices the controller reads, so
/// replacing its code is at least as consequential as an NFT or pool upgrade.
/// Oracle *configuration* runs on the Standard tier; code replacement must not,
/// which is why it resolves through `sensitive_price_aggregator_operation`
/// rather than the plain `price_aggregator_operation` used by
/// `ConfigureAssetOracle`.
#[test]
fn upgrade_price_aggregator_resolves_to_the_aggregator_with_sensitive_delay() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, gov_id, gov) = register_governance(&env);
    let agg_id = env.register(price_aggregator::PriceAggregator, (gov_id.clone(),));
    gov.set_price_aggregator(&agg_id);

    let op = AdminOperation::UpgradePriceAggregator(BytesN::from_array(&env, &[0x33; 32]));
    let resolved = env.as_contract(&gov_id, || resolve_op(&env, &op));

    assert_eq!(resolved.target, agg_id, "must target the price aggregator");
    assert_eq!(resolved.function, Symbol::new(&env, "upgrade"));
    assert_eq!(resolved.args.len(), 1);
    assert_eq!(resolved.delay_tier, DelayTier::Sensitive);
}

/// Same guard every other upgrade variant carries: a zero hash would brick the
/// contract it is applied to.
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
    // Writing off debt against the protocol's reserves must not be reachable
    // on the Standard delay.
    assert_eq!(resolved.delay_tier, DelayTier::Sensitive);
}

#[test]
fn resolve_oracle_zeroes_decimals_for_a_ref_key() {
    let env = Env::default();
    // A `Ref` key names a synthetic quote with no token contract behind it, so
    // there are no on-chain decimals to fetch. The caller's value is discarded
    // rather than trusted: a non-zero input here must still resolve to 0.
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
    // `Unpause` resolves to the controller, so the timelock never routes it
    // here. Reaching this arm means the dispatch in lifecycle.rs and the match
    // in apply_self_op have drifted apart, which is an internal invariant
    // failure rather than a caller error.
    env.as_contract(&gov_id, || apply_self_op(&env, &AdminOperation::Unpause));
}
