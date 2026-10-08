//! RV break pass: DeFindex adapter value integrity under halt states, a
//! fee-on-transfer asset, constructor misconfiguration and the donation
//! vector. Every assertion encodes the SAFE expectation: a failure would mean
//! the attack works.

extern crate std;

use defindex_strategy::{DataKey, DeFindexStrategyError, Strategy, StrategyClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, IntoVal, InvokeError, Val, Vec};
use test_harness::errors::{
    CONTRACT_PAUSED, NOT_AUTHORIZED, SPOKE_ASSET_FROZEN, SPOKE_ASSET_PAUSED, SPOKE_DEPRECATED,
    SPOKE_NOT_FOUND, SPOKE_SUPPLY_CAP_REACHED,
};
use test_harness::presets::unconstrained_test_cap;
use test_harness::{
    eth_preset, hub_asset, usdc_preset, LendingTest, ALICE, BOB, HARNESS_HUB, HARNESS_SPOKE,
};

const UNIT: i128 = 10_000_000;
const DAY: u64 = 60 * 60 * 24;

/// Extracts the contract error code from a `try_*` result, whether the error
/// is the adapter's own enum or a nested controller/pool/OZ code.
fn code<T: core::fmt::Debug, E: core::fmt::Debug>(
    result: Result<Result<T, E>, Result<DeFindexStrategyError, InvokeError>>,
) -> u32 {
    match result {
        Err(Ok(err)) => err as u32,
        Err(Err(InvokeError::Contract(code))) => code,
        other => panic!("expected a contract error, got {other:?}"),
    }
}

struct Fx {
    t: LendingTest,
    strategy: Address,
    vault: Address,
    asset: Address,
}

impl Fx {
    fn build(t: LendingTest, spoke_id: u32) -> Self {
        Self::build_with_hub(t, HARNESS_HUB, spoke_id)
    }

    fn build_with_hub(mut t: LendingTest, hub_id: u32, spoke_id: u32) -> Self {
        t.supply(ALICE, "USDC", 10_000.0);
        t.supply(BOB, "ETH", 100.0);
        t.borrow(BOB, "USDC", 400.0);

        let asset = t.resolve_asset("USDC");
        let init_args: Vec<Val> = vec![
            &t.env,
            t.controller.clone().into_val(&t.env),
            hub_id.into_val(&t.env),
            spoke_id.into_val(&t.env),
        ];
        let strategy = t.env.register(Strategy, (asset.clone(), init_args));
        let vault = Address::generate(&t.env);
        t.resolve_market("USDC")
            .token_admin
            .mint(&vault, &(100_000 * UNIT));
        Self {
            t,
            strategy,
            vault,
            asset,
        }
    }

    fn new() -> Self {
        let t = LendingTest::new()
            .with_market(usdc_preset())
            .with_market(eth_preset())
            .build();
        Self::build(t, HARNESS_SPOKE)
    }

    fn client(&self) -> StrategyClient<'_> {
        StrategyClient::new(&self.t.env, &self.strategy)
    }

    fn usdc(&self, of: &Address) -> i128 {
        token::Client::new(&self.t.env, &self.asset).balance(of)
    }

    fn stored_id(&self) -> u64 {
        let env = &self.t.env;
        env.as_contract(&self.strategy, || {
            env.storage()
                .persistent()
                .get(&DataKey::VaultAccount(self.vault.clone()))
                .unwrap_or(0)
        })
    }

    /// Full exit after a halt: the vault must get `balance` or `balance - 1`
    /// (floor on close), the mapping must clear, the account must be removed,
    /// and nothing may strand on the adapter.
    fn assert_full_exit_works(&self) {
        let sink = Address::generate(&self.t.env);
        let balance = self.client().balance(&self.vault);
        assert!(balance > 0, "fixture must hold a balance");
        let account_id = self.stored_id();
        assert!(account_id != 0);

        let left = self.client().withdraw(&balance, &self.vault, &sink);
        assert_eq!(left, 0, "terminal close must report zero");
        let paid = self.usdc(&sink);
        assert!(
            paid == balance || paid == balance - 1,
            "close pays floor: paid {paid}, balance {balance}"
        );
        assert_eq!(self.stored_id(), 0, "mapping must clear on terminal close");
        assert!(
            !self.t.account_exists(account_id),
            "account must be removed on terminal close"
        );
        assert_eq!(
            self.usdc(&self.strategy),
            0,
            "nothing strands on the adapter"
        );
        assert_eq!(self.client().balance(&self.vault), 0);
    }
}

/// D-6 (global pause): `supply` is pause-gated, `withdraw` is not. Deposits
/// revert atomically (vault keeps its tokens, mapping untouched) while partial
/// and full exits keep working; `harvest`/`balance` stay readable.
#[test]
fn d6_global_pause_blocks_deposit_but_exits_and_views_stay_open() {
    let mut fx = Fx::new();
    fx.client().deposit(&(1_000 * UNIT), &fx.vault);
    fx.t.advance_time(30 * DAY);
    let account_id = fx.stored_id();
    let vault_tokens_before = fx.usdc(&fx.vault);

    fx.t.pause();

    let err = code(fx.client().try_deposit(&(10 * UNIT), &fx.vault));
    assert_eq!(err, CONTRACT_PAUSED, "deposit must fail closed under pause");
    assert_eq!(
        fx.usdc(&fx.vault),
        vault_tokens_before,
        "failed deposit is atomic"
    );
    assert_eq!(fx.usdc(&fx.strategy), 0);
    assert_eq!(
        fx.stored_id(),
        account_id,
        "mapping untouched by a failed deposit"
    );

    // Views and harvest are not pause-gated.
    let balance = fx.client().balance(&fx.vault);
    assert!(balance > 1_000 * UNIT);
    fx.client().harvest(&fx.vault, &None);

    // Partial exit under pause.
    let sink = Address::generate(&fx.t.env);
    let remaining = fx.client().withdraw(&(100 * UNIT), &fx.vault, &sink);
    assert_eq!(fx.usdc(&sink), 100 * UNIT);
    assert_eq!(remaining, balance - 100 * UNIT);
    assert_eq!(fx.stored_id(), account_id);

    // Full exit under pause.
    fx.assert_full_exit_works();
}

/// D-6 (frozen listing): entry rejects `frozen`, exit tolerates it
/// (INV-HALT-02). Set through the owner ratchet.
#[test]
fn d6_frozen_listing_blocks_deposit_but_full_exit_works() {
    let mut fx = Fx::new();
    fx.client().deposit(&(1_000 * UNIT), &fx.vault);
    fx.t.advance_time(30 * DAY);
    let vault_tokens_before = fx.usdc(&fx.vault);

    fx.t.set_spoke_asset_flags("USDC", false, true, false);

    let err = code(fx.client().try_deposit(&(10 * UNIT), &fx.vault));
    assert_eq!(err, SPOKE_ASSET_FROZEN);
    assert_eq!(fx.usdc(&fx.vault), vault_tokens_before);
    assert_eq!(fx.usdc(&fx.strategy), 0);

    fx.assert_full_exit_works();
}

/// D-6 (paused listing): INV-HALT-02 documents that user exits reject
/// `paused`. The vault's funds are trapped only until the flag is relaxed;
/// `balance` keeps reporting and nothing is lost. Pinned here so the
/// documented trap is explicit, not a finding.
#[test]
fn d6_paused_listing_traps_exit_until_relaxed_as_documented() {
    let mut fx = Fx::new();
    fx.client().deposit(&(1_000 * UNIT), &fx.vault);
    fx.t.advance_time(30 * DAY);
    let account_id = fx.stored_id();

    fx.t.set_spoke_asset_flags("USDC", true, false, false);

    let balance = fx.client().balance(&fx.vault);
    assert!(balance > 1_000 * UNIT, "balance still reports under paused");
    let sink = Address::generate(&fx.t.env);
    let err = code(fx.client().try_withdraw(&balance, &fx.vault, &sink));
    assert_eq!(err, SPOKE_ASSET_PAUSED, "documented: exits reject paused");
    let err = code(fx.client().try_withdraw(&UNIT, &fx.vault, &sink));
    assert_eq!(err, SPOKE_ASSET_PAUSED);
    let err = code(fx.client().try_deposit(&UNIT, &fx.vault));
    assert_eq!(err, SPOKE_ASSET_PAUSED);
    assert_eq!(
        fx.stored_id(),
        account_id,
        "mapping survives the failed calls"
    );
    assert_eq!(fx.usdc(&sink), 0);

    // Owner relaxes: everything is recoverable.
    fx.t.set_spoke_asset_paused("USDC", false);
    fx.assert_full_exit_works();
}

/// D-6 (deprecated spoke): `remove_spoke` only deprecates. New exposure is
/// rejected, but the vault can still partially and fully exit, and the
/// account closes with its NFT burned.
#[test]
fn d6_deprecated_spoke_blocks_deposit_but_full_exit_works() {
    let mut fx = Fx::new();
    fx.client().deposit(&(1_000 * UNIT), &fx.vault);
    fx.t.advance_time(30 * DAY);
    let vault_tokens_before = fx.usdc(&fx.vault);

    fx.t.remove_spoke_category(HARNESS_SPOKE);

    let err = code(fx.client().try_deposit(&(10 * UNIT), &fx.vault));
    assert_eq!(err, SPOKE_DEPRECATED);
    assert_eq!(fx.usdc(&fx.vault), vault_tokens_before);
    assert_eq!(fx.usdc(&fx.strategy), 0);

    let sink = Address::generate(&fx.t.env);
    let balance = fx.client().balance(&fx.vault);
    let remaining = fx.client().withdraw(&(250 * UNIT), &fx.vault, &sink);
    assert_eq!(remaining, balance - 250 * UNIT);
    assert_eq!(fx.usdc(&sink), 250 * UNIT);

    fx.assert_full_exit_works();
}

/// D-6 (supply cap): a deposit over the spoke supply cap reverts atomically
/// (vault keeps tokens, adapter holds nothing, mapping unchanged) and a
/// smaller deposit still credits the SAME account, never a different one.
#[test]
fn d6_supply_cap_reached_deposit_reverts_atomically_and_never_mis_credits() {
    let fx = Fx::new();
    fx.client().deposit(&(1_000 * UNIT), &fx.vault);
    let account_id = fx.stored_id();
    let before = fx.client().balance(&fx.vault);
    assert_eq!(before, 1_000 * UNIT);

    // Usage at index RAY: ALICE 10_000 + vault 1_000. Leave 5 USDC headroom.
    let cfg = fx.t.get_asset_config("USDC");
    fx.t.edit_asset_in_spoke_caps(
        "USDC",
        HARNESS_SPOKE,
        cfg.is_collateralizable,
        cfg.is_borrowable,
        cfg.loan_to_value,
        cfg.liquidation_threshold,
        cfg.liquidation_bonus,
        11_005 * UNIT,
        unconstrained_test_cap(7),
    );

    let vault_tokens_before = fx.usdc(&fx.vault);
    let err = code(fx.client().try_deposit(&(10 * UNIT), &fx.vault));
    assert_eq!(err, SPOKE_SUPPLY_CAP_REACHED);
    assert_eq!(
        fx.usdc(&fx.vault),
        vault_tokens_before,
        "failed deposit is atomic"
    );
    assert_eq!(fx.usdc(&fx.strategy), 0, "nothing strands on the adapter");
    assert_eq!(
        fx.stored_id(),
        account_id,
        "mapping unchanged by a failed deposit"
    );
    assert_eq!(fx.client().balance(&fx.vault), before);

    // Under the cap: credited to the same account, exact amount.
    let after = fx.client().deposit(&(4 * UNIT), &fx.vault);
    assert_eq!(
        fx.stored_id(),
        account_id,
        "deposit must reuse the mapped account"
    );
    assert_eq!(after, 1_004 * UNIT);
    assert_eq!(fx.usdc(&fx.strategy), 0);

    // A second vault cannot be credited into the first vault's account.
    let vault_b = Address::generate(&fx.t.env);
    fx.t.resolve_market("USDC")
        .token_admin
        .mint(&vault_b, &UNIT);
    let b_balance = fx.client().deposit(&UNIT, &vault_b);
    assert_eq!(b_balance, UNIT);
    let id_b: u64 = fx.t.env.as_contract(&fx.strategy, || {
        fx.t.env
            .storage()
            .persistent()
            .get(&DataKey::VaultAccount(vault_b.clone()))
            .unwrap_or(0)
    });
    assert!(
        id_b != 0 && id_b != account_id,
        "vault B must own a distinct account"
    );
    assert_eq!(
        fx.client().balance(&fx.vault),
        1_004 * UNIT,
        "vault A unchanged by B"
    );
}

/// D-7 (fee-on-transfer asset, 1% shortfall): the adapter measures once
/// (vault → adapter) and the controller measures again (adapter → pool), so
/// the vault is credited `amount * 0.99^2`. Both haircuts are burned by the
/// token; nothing strands on the adapter and the adapter never over-credits.
#[test]
fn d7_fee_on_transfer_asset_is_haircut_twice_and_nothing_strands_on_adapter() {
    let t = LendingTest::new()
        .with_fee_on_transfer_market(usdc_preset(), 100)
        .with_market(eth_preset())
        .build();
    let fx = Fx::build(t, HARNESS_SPOKE);

    let vault_before = fx.usdc(&fx.vault);
    let credited = fx.client().deposit(&(1_000 * UNIT), &fx.vault);

    // 1000 → 990 at the adapter → 980.1 at the pool (index is RAY, no accrual yet).
    assert_eq!(
        credited, 9_801_000_000,
        "credit must equal the pool-measured amount"
    );
    assert_eq!(fx.client().balance(&fx.vault), 9_801_000_000);
    assert_eq!(
        fx.usdc(&fx.vault),
        vault_before - 1_000 * UNIT,
        "vault debited the full amount"
    );
    assert_eq!(
        fx.usdc(&fx.strategy),
        0,
        "the second haircut is burned by the token, not parked"
    );

    // Full exit: pool pays 980.1 gross, sink receives 980.1 * 0.99 (token fee again).
    let sink = Address::generate(&fx.t.env);
    let left = fx.client().withdraw(&credited, &fx.vault, &sink);
    assert_eq!(left, 0);
    assert_eq!(fx.usdc(&sink), 9_702_990_000);
    assert_eq!(fx.usdc(&fx.strategy), 0);
    assert_eq!(fx.stored_id(), 0);
}

/// D-8 (wrong spoke id): the constructor does not validate `spoke_id`, but the
/// first deposit fails closed before any fund moves (atomic revert), leaving
/// the vault whole, the adapter empty and no mapping.
#[test]
fn d8_wrong_spoke_id_passes_constructor_and_first_deposit_fails_without_moving_funds() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    let fx = Fx::build(t, 99);

    let vault_before = fx.usdc(&fx.vault);
    let err = code(fx.client().try_deposit(&(1_000 * UNIT), &fx.vault));
    assert_eq!(err, SPOKE_NOT_FOUND);
    assert_eq!(
        fx.usdc(&fx.vault),
        vault_before,
        "no funds may move on a misconfigured adapter"
    );
    assert_eq!(fx.usdc(&fx.strategy), 0);
    assert_eq!(fx.stored_id(), 0);
    assert_eq!(fx.client().balance(&fx.vault), 0);
    let sink = Address::generate(&fx.t.env);
    let err = code(fx.client().try_withdraw(&UNIT, &fx.vault, &sink));
    assert_eq!(err, DeFindexStrategyError::InsufficientBalance as u32);
}

/// D-8 (wrong hub id): the constructor's `get_market_index` probe fails with
/// the pool's `PoolNotInitialized` (30) before any config is stored.
#[test]
#[should_panic(expected = "Error(Contract, #30)")]
fn d8_wrong_hub_id_is_rejected_by_the_constructor() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    let _ = Fx::build_with_hub(t, HARNESS_HUB + 7, HARNESS_SPOKE);
}

/// Donation vector: a third party can only top up the asset the vault account
/// already holds. A foreign-asset donation (ETH into a USDC-only account) is
/// refused, so the donation cannot widen the account's position set.
#[test]
fn donation_cannot_add_a_foreign_asset_to_the_vault_account() {
    let fx = Fx::new();
    fx.client().deposit(&(1_000 * UNIT), &fx.vault);
    let account_id = fx.stored_id();

    let attacker = Address::generate(&fx.t.env);
    let eth = fx.t.resolve_asset("ETH");
    fx.t.resolve_market("ETH")
        .token_admin
        .mint(&attacker, &UNIT);
    let res = fx.t.ctrl_client().try_supply(
        &attacker,
        &account_id,
        &HARNESS_SPOKE,
        &vec![&fx.t.env, (hub_asset(eth), UNIT)],
    );
    let err = match res {
        Err(Ok(e)) => e.get_code(),
        other => panic!("expected NotAuthorized, got {other:?}"),
    };
    assert_eq!(err, NOT_AUTHORIZED);
    let (supply, _) = fx.t.ctrl_client().get_account_positions(&account_id);
    assert_eq!(supply.len(), 1, "vault account must stay single-asset");
}
