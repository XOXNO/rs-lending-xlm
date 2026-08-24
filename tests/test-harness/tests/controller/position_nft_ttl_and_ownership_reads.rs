//! A11-ttl audit: the position-NFT `Owner(token_id)` TTL window against the
//! controller's own account window, and what that means for liquidation.
//!
//! `docs/reference/invariants.md` declares INV-STOR-02c and INV-STOR-02d NOT
//! ENFORCED with a VERIFICATION GAP. These tests close the measurable half of
//! that gap: they pin the real windows, prove the two explicit renewal paths
//! behave as INV-STOR-02b claims, and record the precise reason the archival
//! half cannot be exercised in-process.
//!
//! Windows measured here, not assumed:
//! * `Owner(token_id)` — stamped to the protocol `TTL_BUMP_USER` window at
//!   `mint` since F-7, and topped back up to OpenZeppelin's shorter
//!   `OWNER_EXTEND_AMOUNT` (30 days) by `owner_of` once it decays below OZ's
//!   29-day threshold. `stellar-tokens` git rev `fbfde38`,
//!   `packages/tokens/src/non_fungible/mod.rs:395`,
//!   `packages/tokens/src/non_fungible/storage.rs:69`.
//! * `AccountMeta(account_id)` — protocol `TTL_BUMP_USER`, 120 days.
//!   `common/src/constants/shared.rs:81`.
//!
//! Harness caveat that every assertion below depends on:
//! `tests/test-harness/src/time.rs` pins `min_persistent_entry_ttl: 10`, far
//! below any real network's `CONFIG_SETTING_STATE_ARCHIVAL` minimum. Newly
//! created entries therefore start near-dead here and on a real network do
//! not. Assertions are written against the structural relationship rather
//! than that number wherever possible.

use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::{contracttype, Address};
use test_harness::{assert_contract_error, errors, usd_cents, LendingTest, ALICE, LIQUIDATOR};

const DAY_LEDGERS: u32 = 17_280;
const DAY_SECS: u64 = 86_400;

/// OZ `OWNER_EXTEND_AMOUNT` — 30 days.
const OZ_OWNER_WINDOW: u32 = 30 * DAY_LEDGERS;
/// Protocol `TTL_BUMP_USER` — 120 days.
const PROTOCOL_USER_WINDOW: u32 = 120 * DAY_LEDGERS;
/// OZ `OWNER_TTL_THRESHOLD` — 29 days. `owner_of` only extends below this.
const OZ_OWNER_THRESHOLD: u32 = 29 * DAY_LEDGERS;

/// Mirror of `stellar_tokens::non_fungible::NFTStorageKey` (git rev `fbfde38`,
/// `packages/tokens/src/non_fungible/storage.rs:27`). The harness does not
/// depend on `stellar-tokens`, and a `#[contracttype]` enum keys on variant
/// index and payload, so this reproduces the on-ledger key exactly. Variant
/// names and order must track upstream.
#[contracttype]
pub enum NftKey {
    Owner(u32),
    Balance(Address),
    Approval(u32),
    ApprovalForAll(Address, Address),
    Metadata,
}

/// Collapses a `try_` client result into the shape `assert_contract_error`
/// takes. Mirrors `test_harness::ops::internal::map_try_ok_value`, which is
/// crate-private.
fn flatten<T, E>(
    result: Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<T, soroban_sdk::Error>
where
    E: Into<soroban_sdk::Error>,
{
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected a contract error, got an InvokeError")),
    }
}

/// Remaining ledgers of life on the NFT's `Owner(token_id)` entry.
fn owner_ttl(t: &LendingTest, account_id: u64) -> u32 {
    let env = t.env.clone();
    let nft = t.position_nft.clone();
    let key = NftKey::Owner(u32::try_from(account_id).expect("test ids fit u32"));
    env.as_contract(&nft, || env.storage().persistent().get_ttl(&key))
}

/// Remaining ledgers of life on the controller's `AccountMeta(account_id)` entry.
fn meta_ttl(t: &LendingTest, account_id: u64) -> u32 {
    let env = t.env.clone();
    let ctrl = t.controller.clone();
    let key = common::types::ControllerKey::AccountMeta(account_id);
    env.as_contract(&ctrl, || env.storage().persistent().get_ttl(&key))
}

/// INV-STOR-02c, restated after F-7.
///
/// Account creation writes `AccountMeta` through `set_user`, which stamps the
/// protocol's 120-day window. Before F-7 the paired `Owner` entry got nothing —
/// `mint` reaches the key through `Base::update`, which does not extend — so
/// the ownership leg started on the *network's* `minPersistentEntryTTL`.
/// `mint` now lifts it to the same protocol window via
/// `extend_user_persistent_ttl` (`contracts/position-nft/src/contract.rs`), so
/// the two legs start together.
///
/// On mainnet that extend is a strict no-op: `min_persistent_ttl` is 2,073,600,
/// exactly `TTL_BUMP_USER`, so a fresh entry already holds the full window and
/// the 30-day threshold never trips. It bites on testnet
/// (`min_persistent_ttl` = 120,960) and in this harness, which pins
/// `min_persistent_entry_ttl: 10`. See F-7 in
/// `docs/audits/2026-08-22-final-protocol-audit.md`.
#[test]
fn mint_lifts_owner_entry_to_the_protocol_window() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    let id = t.account_id(ALICE);

    let owner = owner_ttl(&t, id);
    let meta = meta_ttl(&t, id);

    assert_eq!(
        meta, PROTOCOL_USER_WINDOW,
        "the controller stamps its own account entry with TTL_BUMP_USER"
    );
    assert_eq!(
        owner, PROTOCOL_USER_WINDOW,
        "F-7: mint lifts the ownership leg to the protocol window"
    );
    assert_eq!(
        owner, meta,
        "the two legs start together -- the mint-time asymmetry F-7 described \
         is closed; owner={owner} meta={meta}"
    );
}

/// Ages the ledger until the ownership leg has decayed below OZ's threshold,
/// then runs one controller op so OZ's passive extend fires.
///
/// Needed since F-7: `mint` now starts the `Owner` entry on the full 120-day
/// protocol window, so `owner_of` is a no-op until the entry falls under
/// `OZ_OWNER_THRESHOLD`. Decaying from `PROTOCOL_USER_WINDOW` to below 29 days
/// takes just over 91 days; 92 clears it with margin. The supply is chosen as
/// the op because adding collateral cannot fail a health check.
fn decay_owner_leg_then_touch(t: &mut LendingTest) {
    let id = t.account_id(ALICE);
    t.advance_time(92 * DAY_SECS);
    let decayed = owner_ttl(t, id);
    assert!(
        decayed < OZ_OWNER_THRESHOLD,
        "the warm-up must actually take the leg below OZ's threshold or the \
         top-up below is vacuous; decayed={decayed} threshold={OZ_OWNER_THRESHOLD}"
    );
    t.supply(ALICE, "USDC", 1.0);
}

/// INV-STOR-02c, second half: OZ renewal tops up to 30 days and no further, and
/// does not stack. Controller traffic on the same account cannot lift it past
/// OZ's ceiling, even though F-7 started the entry above it.
#[test]
fn passive_owner_of_lifts_only_to_the_oz_window() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    let id = t.account_id(ALICE);

    // F-7 starts the leg on the protocol window, above OZ's threshold, so a
    // passive read does nothing until it has decayed.
    assert_eq!(
        owner_ttl(&t, id),
        PROTOCOL_USER_WINDOW,
        "F-7: the leg starts on the protocol window"
    );
    t.borrow(ALICE, "ETH", 1.0);
    assert_eq!(
        owner_ttl(&t, id),
        PROTOCOL_USER_WINDOW,
        "a passive read above OZ's threshold must not shorten the entry"
    );

    // Once decayed below OZ's threshold, a resolved owner_of tops it back up.
    decay_owner_leg_then_touch(&mut t);
    assert_eq!(
        owner_ttl(&t, id),
        OZ_OWNER_WINDOW,
        "a resolved owner_of restores the entry to exactly OWNER_EXTEND_AMOUNT"
    );

    // Repeating it does not stack: still 30 days, not 60.
    t.supply(ALICE, "USDC", 1.0);
    assert_eq!(
        owner_ttl(&t, id),
        OZ_OWNER_WINDOW,
        "OZ renewal is a top-up to a fixed ceiling, never an addition"
    );
    assert_eq!(
        meta_ttl(&t, id),
        PROTOCOL_USER_WINDOW,
        "the same operations keep the controller leg on the 120-day window"
    );
    assert_eq!(
        PROTOCOL_USER_WINDOW / OZ_OWNER_WINDOW,
        4,
        "the documented asymmetry is exactly 4x; a change here changes the \
         dormancy budget a liquidation bot must plan for"
    );
}

/// INV-STOR-02b: both explicit paths lift the ownership leg to the protocol
/// window. `renew_account` is owner-gated; `position-nft::renew` carries no
/// `require_auth` (`contracts/position-nft/src/contract.rs:92`), so a keeper or
/// liquidation bot can pre-warm any position it watches.
#[test]
fn renew_account_and_permissionless_renew_close_the_ttl_gap() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    let id = t.account_id(ALICE);
    let alice = t.get_or_create_user(ALICE);
    let token_id = u32::try_from(id).expect("test ids fit u32");

    t.ctrl_client().renew_account(&alice, &id);
    assert_eq!(
        owner_ttl(&t, id),
        PROTOCOL_USER_WINDOW,
        "renew_account must lift the ownership leg to the controller's window"
    );

    // Decay past TTL_THRESHOLD_USER (30 days remaining), the point below which
    // Soroban's `extend_ttl` stops being a no-op, then have an unrelated third
    // party top it back up.
    t.advance_time_no_refresh(91 * DAY_SECS);
    let decayed = owner_ttl(&t, id);
    assert!(
        decayed < 30 * DAY_LEDGERS,
        "must be under the renewal threshold for the extend to bite, got {decayed}"
    );

    position_nft::PositionNftClient::new(&t.env, &t.position_nft).renew(&token_id);
    assert_eq!(
        owner_ttl(&t, id),
        PROTOCOL_USER_WINDOW,
        "position-nft::renew is permissionless rent charity and reaches the same window"
    );
}

/// INV-STOR-02d — the archival half. Verdict: **not a liquidation DoS.**
///
/// `liquidate` does resolve NFT ownership on its hot path. The first statement
/// of `process_liquidation` after the auth check is `storage::get_account`
/// (`contracts/controller/src/positions/liquidation/mod.rs:56`), which reaches
/// `try_account_owner` (`contracts/controller/src/storage/account.rs:30`) and
/// cross-calls `owner_of`. An unreadable `Owner` entry would block it. The
/// same is true of `clean_bad_debt` and `force_socialize_bad_debt`, both of
/// which enter through `socialize_bad_debt`
/// (`.../liquidation/mod.rs:249`) and then burn through `Base::owner_of` again.
///
/// It is not blocked, and this test records why. The soroban-sdk test `Env`
/// runs storage in **recording** footprint mode, where
/// `soroban-env-host-27.0.1/src/storage.rs:723` (`handle_maybe_expired_entry`)
/// silently auto-restores an expired *persistent* entry instead of failing —
/// only *temporary* entries are dropped. That is not a test-only shortcut:
/// recording mode is what `simulateTransaction` runs, and from protocol 23 the
/// restored ids ride back as
/// `SorobanTransactionData.ext.v1.archivedSorobanEntries`
/// (`stellar-xdr-23.0.0`), so the submitted `InvokeHostFunctionOp` restores the
/// entry in line. The harness pins `protocol_version: 27`
/// (`tests/test-harness/src/presets.rs:20`).
///
/// Consequence: a liquidator that preflights then submits is never blocked by
/// an archived `Owner` entry; it pays restore rent and proceeds. Only a
/// liquidator that hand-builds a footprint without simulating needs an explicit
/// restore. This test pins the auto-restore so that an SDK or protocol change
/// reinstating a hard failure breaks here loudly rather than silently turning
/// dormant positions into bad debt.
#[test]
fn liquidation_resolves_nft_ownership_and_succeeds_after_auto_restore() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.assert_healthy(ALICE);
    let id = t.account_id(ALICE);

    // Since F-7 the ownership leg starts on the protocol window, so the
    // asymmetry this test depends on does not exist at mint. Let it decay past
    // OZ's threshold and take one passive top-up: that puts the ownership leg
    // on OZ's 30-day ceiling and re-stamps the account leg to 120 days, which
    // is the state the hazard below is about.
    decay_owner_leg_then_touch(&mut t);

    // Ownership leg on OZ's ceiling, account leg on the protocol's; note the
    // exact ledger the ownership leg dies at.
    let owner_dies_at = t.env.ledger().sequence() + owner_ttl(&t, id);
    let meta_dies_at = t.env.ledger().sequence() + meta_ttl(&t, id);
    assert_eq!(owner_ttl(&t, id), OZ_OWNER_WINDOW);
    assert!(
        meta_dies_at > owner_dies_at + 89 * DAY_LEDGERS,
        "the account outlives its ownership leg by ~90 days -- that window is \
         the whole hazard: owner_dies_at={owner_dies_at} meta_dies_at={meta_dies_at}"
    );

    // Dormant past the ownership leg's death, while the account leg is alive.
    t.advance_time(31 * DAY_SECS);
    let seq = t.env.ledger().sequence();
    assert!(
        seq > owner_dies_at,
        "ledger {seq} must be past the Owner entry's live-until {owner_dies_at}"
    );
    assert!(
        seq < meta_dies_at,
        "the controller's account entry must still be alive -- that is the asymmetry"
    );

    // The account goes underwater while dormant.
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    // Liquidation succeeds: the host auto-restored the lapsed Owner entry.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    assert!(
        t.try_nft_owner_of(id),
        "ownership resolves after the auto-restore"
    );
    assert!(
        owner_ttl(&t, id) > 0,
        "the restored entry carries a fresh live-until"
    );
}

/// Settles whether an **ordinary partial liquidation** resolves NFT ownership,
/// against the claim that the liquidation module's only NFT touch is the burn
/// in `remove_account_and_burn_nft` (`.../liquidation/bad_debt.rs:61`).
///
/// A symbol grep of `positions/liquidation/**` does miss it, because the read
/// is indirect: `process_liquidation` (`.../liquidation/mod.rs:56`) calls
/// `storage::get_account`, and the NFT hop is three frames down in
/// `storage/account.rs`. This test removes the grep from the argument.
///
/// Method: the test `Env` cannot archive an entry, but it can make the same
/// entry unreadable in the one way it does model — burn it — while leaving
/// every controller-side account entry intact. If liquidation resolves the
/// owner, it must now fail `AccountNotFound` (`common/src/errors.rs:39`)
/// *before* touching any balance. If it does not resolve the owner, a partial
/// liquidation must still succeed.
#[test]
fn partial_liquidation_resolves_nft_ownership() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    let id = t.account_id(ALICE);
    let token_id = u32::try_from(id).expect("test ids fit u32");

    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    // Detach the ownership leg, leaving controller state untouched.
    position_nft::PositionNftClient::new(&t.env, &t.position_nft).burn(&token_id);
    assert!(!t.try_nft_owner_of(id), "ownership leg is now unreadable");
    assert!(
        t.account_exists(id),
        "controller-side account state must still be live -- this isolates the \
         NFT read as the only thing that changed"
    );

    // A plain partial liquidation, well under the close factor.
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::ACCOUNT_NOT_FOUND);
}

/// Same isolation applied to the wind-down path. `clean_bad_debt` and
/// `force_socialize_bad_debt` both enter `socialize_bad_debt`
/// (`.../liquidation/mod.rs:247`), whose line 249 is the same
/// `storage::get_account`, so they fail on the owner read long before reaching
/// the burn at `bad_debt.rs:61`.
#[test]
fn bad_debt_winddown_resolves_nft_ownership() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    let id = t.account_id(ALICE);
    let token_id = u32::try_from(id).expect("test ids fit u32");
    let keeper = t.get_or_create_user(LIQUIDATOR);

    t.set_price("USDC", usd_cents(1));
    position_nft::PositionNftClient::new(&t.env, &t.position_nft).burn(&token_id);
    assert!(t.account_exists(id), "controller state still live");

    assert_contract_error(
        flatten(t.ctrl_client().try_clean_bad_debt(&keeper, &id)),
        errors::ACCOUNT_NOT_FOUND,
    );
    assert_contract_error(
        flatten(t.ctrl_client().try_force_socialize_bad_debt(&id)),
        errors::ACCOUNT_NOT_FOUND,
    );
}
