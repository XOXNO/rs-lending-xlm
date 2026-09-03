use crate::external::position_nft::GhostNftKey;
use crate::storage;
use common::types::{
    AccountMeta, AccountPositionRaw, HubAssetKey, HubConfig, PositionLimits, PositionMode,
    SpokeAssetConfig, SpokeConfig, SpokeUsageRaw,
};
use cvlr::cvlr_assume;
use cvlr_soroban::nondet_address;
use soroban_sdk::{Address, Env};

pub const ACCOUNT_ID: u64 = 1;
pub const HUB_ID: u32 = 1;
pub const SPOKE_ID: u32 = 1;

pub const UNCONSTRAINED_CAP: i128 = i128::MAX
    / 10i128.pow(common::constants::RAY_DECIMALS)
    / (common::constants::RAY / common::constants::SUPPLY_INDEX_FLOOR_RAW);

/// `POSITION_LIMIT_MAX` as a `usize`, for the fixed-size seed arrays the
/// position-limit fixtures build.
pub const POSITION_LIMIT: usize = common::constants::POSITION_LIMIT_MAX as usize;

/// Compile-time guard on the position cap.
///
/// A rule cannot build its seed array from a runtime value — the assets are
/// rule *arguments* — so every position-limit fixture spells out
/// `POSITION_LIMIT` (or `POSITION_LIMIT - 1`) addresses by hand and lets the
/// array type check the count. Raising or lowering `POSITION_LIMIT_MAX` must
/// break this build, not quietly make those rules vacuous: seeding more
/// positions than the cap makes `cvlr_assume!(seeded == cap)` unsatisfiable,
/// which is exactly how ten-asset fixtures survived the 2026-08-14 change from
/// ten to five without a single failing rule.
///
/// Fixtures to re-count when this fires:
/// `solvency_rules::supply_position_limit_enforced`,
/// `solvency_rules::borrow_position_limit_enforced`,
/// `solvency_rules::supply_position_limit_enforced_fixture_completes`,
/// `solvency_rules::borrow_position_limit_enforced_fixture_completes`,
/// `solvency_rules::supply_topup_survives_lowered_limit`,
/// `spoke_rules::bulk_supply_duplicate_asset_counted_once`,
/// `spoke_rules::bulk_supply_distinct_legs_exceed_limit_reverts`,
/// `spoke_rules::bulk_supply_distinct_legs_exceed_limit_reverts_fixture_completes`,
/// `spoke_rules::bulk_borrow_duplicate_leg_not_double_counted`,
/// `spoke_rules::bulk_borrow_distinct_legs_exceed_limit_reverts`.
const _: () = assert!(common::constants::POSITION_LIMIT_MAX == 5);

pub fn hub_asset(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: HUB_ID,
        asset: asset.clone(),
    }
}

pub fn seed_protocol(env: &Env) {
    crate::storage::set_pool(env, &nondet_address());
    crate::storage::set_swap_aggregator(env, &nondet_address());
    crate::storage::set_price_aggregator(env, &nondet_address());
    crate::storage::set_accumulator(env, &nondet_address());
    crate::storage::set_position_limits(
        env,
        &PositionLimits {
            max_supply_positions: common::constants::POSITION_LIMIT_MAX,
            max_borrow_positions: common::constants::POSITION_LIMIT_MAX,
        },
    );
    crate::storage::set_min_borrow_collateral_usd_wad(env, 0);
    crate::storage::set_hub(env, HUB_ID, &HubConfig { is_active: true });
    crate::storage::set_spoke(
        env,
        SPOKE_ID,
        &SpokeConfig {
            is_deprecated: false,
            liquidation_target_hf_wad: crate::constants::DEFAULT_LIQUIDATION_TARGET_HF_WAD,
            hf_for_max_bonus_wad: crate::constants::DEFAULT_HF_FOR_MAX_BONUS_WAD,
            liquidation_bonus_factor_bps: crate::constants::DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS,
        },
    );
}

/// Assumes `account_id` starts with both position books empty, so a rule can
/// then seed exactly the positions its property needs and nothing else.
///
/// Sunbeam havocs contract storage at rule start, so a book no fixture writes
/// is an *arbitrary* map — any length, any keys, any scaled amounts and risk
/// parameters — not an empty one. A rule that seeds one position on top of
/// that still runs over the arbitrary remainder, which is both the strongest
/// and the most expensive form (every read is unrolled under the conf's
/// `loop_iter`).
///
/// What this excludes: every pre-existing position the account might hold, and
/// therefore every counterexample that needs a second asset in the book. Use
/// it where the property is about the acting account's own positions or
/// totals. Frame rules — "this verb does not touch that other account" — must
/// stay unbounded, and `assume_books_at_most_one` is the middle setting the
/// health family uses.
pub fn seed_empty_books(env: &Env, account_id: u64) {
    cvlr_assume!(storage::get_supply_positions(env, account_id).is_empty());
    cvlr_assume!(storage::get_debt_positions(env, account_id).is_empty());
}

/// Assumes every entry of `account_id`'s arbitrary books is well formed:
/// non-negative scaled amounts, a liquidation threshold within `BPS`, and a
/// loan-to-value no greater than that threshold.
///
/// This is the premise the risk-totals summary encodes implicitly (it draws
/// non-negative totals with `weighted <= total` and `ltv <= total`) and the
/// premise production maintains: `validate_asset_params` refuses a listing
/// outside these bounds and every write restamps from a listing. Havoced
/// storage does not know that, so a rule that keeps its books unbounded — the
/// frame rules — must say it, or a counterexample can be a book no listing
/// could produce.
///
/// It does *not* pin the risk tuple to any particular listing: a well-formed
/// pre-book entry can still be restamped to different values by a verb, so
/// rules that compare a valuation across a call need explicit seeds instead.
pub fn assume_wellformed_book(env: &Env, account_id: u64) {
    for (_, position) in storage::get_supply_positions(env, account_id).iter() {
        cvlr_assume!(position.scaled_amount >= 0);
        cvlr_assume!(i128::from(position.liquidation_threshold) <= common::constants::BPS);
        cvlr_assume!(position.loan_to_value <= position.liquidation_threshold);
    }
    for (_, position) in storage::get_debt_positions(env, account_id).iter() {
        cvlr_assume!(position.scaled_amount >= 0);
    }
}

/// Assumes both of `account_id`'s books hold at most one entry: one unknown
/// neighbour position survives, an arbitrary book does not. The shape every
/// `post_gate_*` verb rule and the frozen-valuation rules start from.
pub fn assume_books_at_most_one(env: &Env, account_id: u64) {
    let account = storage::get_account(env, account_id);
    cvlr_assume!(account.supply_positions.len() <= 1);
    cvlr_assume!(account.borrow_positions.len() <= 1);
}

/// Assumes `assets` pairwise distinct and all different from `extra`.
///
/// The position maps are keyed by hub asset, so only distinct keys reach the
/// seeded count the position-limit rules assume, and only a key outside
/// `assets` opens a new slot. This is what the production de-duplicating
/// counter relies on.
pub fn assume_pairwise_distinct(assets: &[Address], extra: &Address) {
    for i in 0..assets.len() {
        cvlr_assume!(assets[i] != *extra);
        for j in (i + 1)..assets.len() {
            cvlr_assume!(assets[i] != assets[j]);
        }
    }
}

/// Pins the id the next `create_account` mints to `last_id + 1`, so a rule
/// that drives a verb which opens a fresh account knows that account's id
/// before the call and can bound its books with `seed_empty_books`.
///
/// Without this the ghost NFT counter is havoced, the new id is arbitrary, and
/// the "fresh" account the verb creates may read back an arbitrary book.
pub fn seed_next_account_id(env: &Env, last_id: u64) -> u64 {
    env.storage()
        .persistent()
        .set(&GhostNftKey::NextId, &last_id);
    last_id + 1
}

pub fn seed_account(env: &Env, account_id: u64, owner: &Address) {
    env.storage()
        .persistent()
        .set(&GhostNftKey::Owner(account_id), owner);
    crate::storage::set_account_meta(
        env,
        account_id,
        &AccountMeta {
            spoke_id: SPOKE_ID,
            mode: PositionMode::Normal,
        },
    );
}

pub fn seed_market(env: &Env, asset: &Address) {
    seed_protocol(env);
    crate::storage::set_spoke_asset(
        env,
        SPOKE_ID,
        &hub_asset(asset),
        &SpokeAssetConfig {
            is_collateralizable: true,
            is_borrowable: true,
            paused: false,
            frozen: false,
            no_seize: false,
            loan_to_value: 7_500,
            liquidation_threshold: 8_000,
            liquidation_bonus: 500,
            liquidation_fees: 100,
            supply_cap: UNCONSTRAINED_CAP,
            borrow_cap: UNCONSTRAINED_CAP,
        },
    );
}

pub fn seed_live_account(env: &Env, account_id: u64, owner: &Address, asset: &Address) {
    seed_market(env, asset);
    seed_account(env, account_id, owner);
}

/// Writes a concrete supply position for `asset`. The caller must have already
/// seeded the account (e.g. `seed_live_account`), so rules can control the
/// owner/caller relationship; this helper only writes the position map.
pub fn seed_supply_position(env: &Env, account_id: u64, asset: &Address, scaled_amount: i128) {
    let mut map = crate::storage::get_supply_positions(env, account_id);
    map.set(
        hub_asset(asset),
        AccountPositionRaw {
            scaled_amount,
            liquidation_threshold: 8_000,
            liquidation_bonus: 500,
            loan_to_value: 7_500,
            liquidation_fees: 100,
        },
    );
    crate::storage::set_supply_positions(env, account_id, &map);
}

/// Writes a concrete debt position for `asset`. The caller must have already
/// seeded the account (e.g. `seed_live_account`).
pub fn seed_debt_position(env: &Env, account_id: u64, asset: &Address, scaled_amount: i128) {
    let mut map = crate::storage::get_debt_positions(env, account_id);
    map.set(
        hub_asset(asset),
        common::types::DebtPositionRaw { scaled_amount },
    );
    crate::storage::set_debt_positions(env, account_id, &map);
}

/// Writes one concrete supply position per entry of `assets`, returning the
/// resulting map length.
///
/// The length is `assets.len()` only when the assets are pairwise distinct
/// *and* the book started empty: the map read here is the account's havoced
/// book, so distinctness alone gives `len >= assets.len()`. Call
/// `seed_empty_books` first. `cvlr_assume!(seeded == N)` is not a substitute —
/// it constrains the pre-map instead, and it silently becomes unsatisfiable
/// (vacuous rule) when `N` exceeds the position cap.
pub fn seed_supply_positions(env: &Env, account_id: u64, assets: &[Address]) -> u32 {
    let mut map = crate::storage::get_supply_positions(env, account_id);
    for asset in assets {
        map.set(
            hub_asset(asset),
            AccountPositionRaw {
                scaled_amount: 1,
                liquidation_threshold: 8_000,
                liquidation_bonus: 500,
                loan_to_value: 7_500,
                liquidation_fees: 100,
            },
        );
    }
    let count = map.len();
    crate::storage::set_supply_positions(env, account_id, &map);
    count
}

/// Reads spoke `spoke_id`'s stored `SpokeUsage` row for `hub_asset`, falling
/// back to the zero row when storage has none.
///
/// `SpokeUsageContext::apply_exit` treats a missing row as "nothing to
/// decrement" rather than as a zero row it may take negative, so the absent
/// and zero cases are *not* interchangeable for exits. Rules that exercise an
/// exit leg must therefore seed a row with `seed_spoke_usage` first; see
/// `usage_exit_without_usage_row_is_a_noop` in `spoke_rules.rs`, which pins
/// that carve-out instead of hiding it.
pub fn spoke_usage(env: &Env, spoke_id: u32, hub_asset: &HubAssetKey) -> SpokeUsageRaw {
    crate::storage::get_spoke_usage(env, spoke_id, hub_asset).unwrap_or_default()
}

/// Writes a concrete `SpokeUsage` row for `asset` on `SPOKE_ID`. Callers pass
/// values at or above the account-level scaled amounts they seeded, since the
/// stored row is the sum over every account bound to the spoke.
pub fn seed_spoke_usage(
    env: &Env,
    asset: &Address,
    supplied_scaled_ray: i128,
    borrowed_scaled_ray: i128,
) {
    crate::storage::set_spoke_usage(
        env,
        SPOKE_ID,
        &hub_asset(asset),
        &SpokeUsageRaw {
            supplied_scaled_ray,
            borrowed_scaled_ray,
        },
    );
}

/// Writes one concrete debt position per entry of `assets`, returning the
/// resulting map length. Same caveat as `seed_supply_positions`: call
/// `seed_empty_books` first, or the length includes whatever the havoced book
/// already held.
pub fn seed_debt_positions(env: &Env, account_id: u64, assets: &[Address]) -> u32 {
    let mut map = crate::storage::get_debt_positions(env, account_id);
    for asset in assets {
        map.set(
            hub_asset(asset),
            common::types::DebtPositionRaw { scaled_amount: 1 },
        );
    }
    let count = map.len();
    crate::storage::set_debt_positions(env, account_id, &map);
    count
}
