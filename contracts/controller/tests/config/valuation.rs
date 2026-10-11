//! Per-account valuation exclusion (fix family `lp_shield`): preconditions,
//! the outage allow-list, the restore's feed and stamp checks, and the storage
//! lifecycle of the exclusion map.

use super::*;
use common::types::{Account, AccountPositionRaw, ControllerKey, PositionMode, PriceKey};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{contract, contractimpl, symbol_short, Address, Map, Vec};

use crate::risk::{holds_valuation_excluded_leg, valued_supply_positions};
use crate::Controller;

/// Aggregator stand-in whose `quotes` report every key with one status.
#[contract]
struct QuoteOracle;

#[contractimpl]
impl QuoteOracle {
    pub fn set_status(env: Env, valid: bool, code: Option<u32>) {
        env.storage()
            .instance()
            .set(&symbol_short!("valid"), &valid);
        env.storage().instance().set(&symbol_short!("code"), &code);
    }

    pub fn quotes(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceStatus> {
        let valid: bool = env
            .storage()
            .instance()
            .get(&symbol_short!("valid"))
            .unwrap_or(false);
        let code: Option<u32> = env
            .storage()
            .instance()
            .get(&symbol_short!("code"))
            .unwrap_or(None);
        let mut out = Map::new(&env);
        for key in keys.iter() {
            out.set(
                key,
                PriceStatus {
                    final_wad: i128::from(valid),
                    valid,
                    error_code: code,
                    ..PriceStatus::unusable()
                },
            );
        }
        out
    }
}

const STALE: Option<u32> = Some(OracleError::PriceFeedStale as u32);

struct Fixture {
    env: Env,
    controller: Address,
    oracle: Address,
    asset: Address,
    other: Address,
}

impl Fixture {
    /// The controller wired to a quote oracle reporting `(valid, code)`, and
    /// accounts 1 and 2 both holding supply legs of `asset` and `other`.
    fn new(valid: bool, code: Option<u32>) -> Self {
        let env = Env::default();
        let controller = env.register(Controller, (Address::generate(&env),));
        let oracle = env.register(QuoteOracle, ());
        let asset = Address::generate(&env);
        let other = Address::generate(&env);
        let fixture = Self {
            env,
            controller,
            oracle,
            asset,
            other,
        };
        fixture.set_status(valid, code);
        fixture.env.ledger().set_sequence_number(100);
        fixture.within(|f| {
            storage::set_price_aggregator(&f.env, &f.oracle);
            for account_id in [1u64, 2] {
                storage::set_supply_positions(&f.env, account_id, &f.book());
            }
        });
        fixture
    }

    fn set_status(&self, valid: bool, code: Option<u32>) {
        QuoteOracleClient::new(&self.env, &self.oracle).set_status(&valid, &code);
    }

    fn within<R>(&self, f: impl FnOnce(&Self) -> R) -> R {
        self.env.as_contract(&self.controller, || f(self))
    }

    fn key(&self) -> HubAssetKey {
        HubAssetKey {
            hub_id: 0,
            asset: self.asset.clone(),
        }
    }

    fn other_key(&self) -> HubAssetKey {
        HubAssetKey {
            hub_id: 0,
            asset: self.other.clone(),
        }
    }

    fn book(&self) -> Map<HubAssetKey, AccountPositionRaw> {
        let mut book = Map::new(&self.env);
        book.set(self.key(), leg());
        book.set(self.other_key(), leg());
        book
    }

    fn exclude(&self, account_id: u64) {
        self.within(|f| exclude_account_asset_valuation(&f.env, account_id, f.key()));
    }

    fn restore(&self, account_id: u64, excluded_at: u32) {
        self.within(|f| restore_account_asset_valuation(&f.env, account_id, f.key(), excluded_at));
    }

    fn exclusions(&self, account_id: u64) -> Map<HubAssetKey, u32> {
        self.within(|f| storage::get_account_valuation_exclusions(&f.env, account_id))
    }
}

fn leg() -> AccountPositionRaw {
    AccountPositionRaw {
        scaled_amount: 1,
        liquidation_threshold: 8_500,
        liquidation_bonus: 500,
        loan_to_value: 8_000,
        liquidation_fees: 0,
    }
}

#[test]
#[should_panic(expected = "Error(Contract, #320)")]
fn exclusion_is_refused_while_the_strict_read_succeeds() {
    Fixture::new(true, None).exclude(1);
}

/// A sanity-band violation is the one strict-read failure an immediate
/// governance role (ORACLE_ROLE's `set_sanity_band`) can cause on a priced
/// feed, so it does not admit the exclusion.
#[test]
#[should_panic(expected = "Error(Contract, #323)")]
fn exclusion_is_refused_for_a_sanity_band_failure() {
    Fixture::new(false, Some(OracleError::SanityBoundViolated as u32)).exclude(1);
}

/// Configuration failures come from delayed governance, not from an outage.
#[test]
#[should_panic(expected = "Error(Contract, #323)")]
fn exclusion_is_refused_for_a_configuration_failure() {
    Fixture::new(false, Some(OracleError::OracleNotConfigured as u32)).exclude(1);
}

/// An unusable status without a failure code is not an outage either.
#[test]
#[should_panic(expected = "Error(Contract, #323)")]
fn exclusion_is_refused_without_a_failure_code() {
    Fixture::new(false, None).exclude(1);
}

/// Every allow-listed outage admits the exclusion.
#[test]
fn every_outage_failure_on_the_allow_list_admits_the_exclusion() {
    for failure in OUTAGE_FAILURES {
        let f = Fixture::new(false, Some(failure as u32));
        f.exclude(1);
        assert!(f.exclusions(1).contains_key(f.key()), "{failure:?}");
    }
}

#[test]
#[should_panic(expected = "Error(Contract, #121)")]
fn exclusion_requires_the_account_to_hold_the_supply_leg() {
    let f = Fixture::new(false, STALE);
    f.within(|f| {
        let third = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&f.env),
        };
        exclude_account_asset_valuation(&f.env, 1, third);
    });
}

/// The exclusion is per account: account 1 drops the leg, account 2 holding
/// the same asset still values it, and the listing's flags epoch is untouched.
#[test]
fn exclusion_drops_the_leg_of_the_named_account_only() {
    let f = Fixture::new(false, STALE);
    f.exclude(1);

    assert_eq!(
        f.exclusions(1).get(f.key()),
        Some(100),
        "stamped at the ledger"
    );
    assert!(f.exclusions(2).is_empty());
    f.within(|f| {
        let valued = valued_supply_positions(&f.env, 1, &f.book());
        assert_eq!(valued.keys(), vec![&f.env, f.other_key()]);
        assert_eq!(valued_supply_positions(&f.env, 2, &f.book()).len(), 2);
        assert_eq!(storage::get_spoke_flags_epoch(&f.env, 1, &f.key()), 0);
        assert_eq!(storage::get_spoke_flags_epoch(&f.env, 0, &f.key()), 0);
    });
}

#[test]
fn holds_valuation_excluded_leg_follows_the_named_account() {
    let f = Fixture::new(false, STALE);
    f.exclude(1);
    f.within(|f| {
        let owner = Address::generate(&f.env);
        let account = |positions| Account {
            owner: owner.clone(),
            spoke_id: 1,
            mode: PositionMode::Normal,
            supply_positions: positions,
            borrow_positions: Map::new(&f.env),
        };
        assert!(holds_valuation_excluded_leg(&f.env, 1, &account(f.book())));
        assert!(!holds_valuation_excluded_leg(&f.env, 2, &account(f.book())));
        let mut without = f.book();
        without.remove(f.key());
        assert!(!holds_valuation_excluded_leg(&f.env, 1, &account(without)));
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #321)")]
fn exclusion_rejects_an_excluded_leg() {
    let f = Fixture::new(false, STALE);
    f.exclude(1);
    f.exclude(1);
}

/// A restore that names the live stamp clears the exclusion once the feed is
/// usable and deletes the emptied map.
#[test]
fn restore_with_the_live_stamp_clears_the_exclusion_once_the_feed_is_usable() {
    let f = Fixture::new(false, STALE);
    f.exclude(1);
    f.set_status(true, None);
    f.restore(1, 100);
    assert!(f.exclusions(1).is_empty());
    f.within(|f| {
        assert!(!f
            .env
            .storage()
            .persistent()
            .has(&ControllerKey::AccountValuationExclusions(1)));
    });
}

/// R1: a dead feed keeps its exclusion, so a restore during the outage cannot
/// re-arm the shield.
#[test]
#[should_panic(expected = "Error(Contract, #322)")]
fn restore_is_refused_while_the_feed_is_unusable() {
    let f = Fixture::new(false, STALE);
    f.exclude(1);
    f.restore(1, 100);
}

/// A restore prepared for an earlier exclusion cannot clear a later one.
#[test]
#[should_panic(expected = "Error(Contract, #321)")]
fn restore_naming_an_earlier_exclusion_reverts() {
    let f = Fixture::new(false, STALE);
    f.exclude(1);
    f.set_status(true, None);
    f.restore(1, 100);
    f.set_status(false, STALE);
    f.env.ledger().set_sequence_number(150);
    f.exclude(1);
    f.set_status(true, None);
    f.restore(1, 100);
}

#[test]
#[should_panic(expected = "Error(Contract, #321)")]
fn restore_of_a_leg_that_is_not_excluded_reverts() {
    let f = Fixture::new(true, None);
    f.restore(1, 100);
}

/// Account removal deletes the exclusion map, and the TTL renewal of a live
/// account covers it.
#[test]
fn account_removal_and_renewal_cover_the_exclusion_map() {
    use common::constants::TTL_BUMP_USER;
    use soroban_sdk::testutils::storage::Persistent as _;

    let f = Fixture::new(false, STALE);
    f.exclude(1);
    let key = ControllerKey::AccountValuationExclusions(1);
    f.env
        .ledger()
        .with_mut(|l| l.sequence_number += TTL_BUMP_USER - 1_000);
    let aged = f.within(|f| f.env.storage().persistent().get_ttl(&key));
    f.within(|f| storage::renew_user_account(&f.env, 1));
    assert!(f.within(|f| f.env.storage().persistent().get_ttl(&key)) > aged);

    f.within(|f| storage::remove_account_entry(&f.env, 1));
    f.within(|f| assert!(!f.env.storage().persistent().has(&key)));
}
