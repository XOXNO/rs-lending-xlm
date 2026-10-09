//! Rounding audit, lens R-8: bulk actions, duplicate-leg merges and order
//! dependence.
//!
//! Hypotheses settled here, each against the documented bound of one raw
//! share (1e-27 token-RAY) per boundary (`docs/reference/formulas.md`,
//! "Shares and token amounts"):
//!
//! A. `aggregate_payments` merges `[(A, x), (A, y)]` into one leg before the
//!    pool rounds once. Against an exact integer model at accrued indexes,
//!    the merged leg equals a single `x + y` leg bit for bit, and differs from
//!    two separate transactions by at most one raw share, in the direction
//!    that favours the user (floor-mint gains, ceil-burn and ceil-mint save,
//!    floor-burn gains). Under `MeansAll`, `[(A, x), (A, 0)]` and
//!    `[(A, 0), (A, x)]` both close the position and pay the floored claim.
//!    Spoke usage equals the summed positions after every batch.
//! B. The pool itself rounds per entry: two entries for one market in one
//!    `supply` batch each load the market afresh, so the second result is
//!    built on the caller's stale position, not on the first result. The
//!    controller's merge is what keeps INV-ACCT-10 (never reachable from an
//!    entrypoint; recorded as the reason the merge is load-bearing).
//! C. `update_account_threshold` over N accounts shares one price/index cache.
//!    Stamped tuples and health factors equal those of N single calls and of
//!    a permuted batch; a gated leg keeps its stale tuple in every ordering.
//!    An account under 1.05 reverts the whole batch and reverts alone too.
//! D. `get_bulk_indexes` over several markets equals the per-market call and
//!    the state `update_indexes` commits, across a gap longer than one
//!    compounding chunk, so the plan-time index never drifts from execution.

use common::math::fp_core::{mul_div_ceil, mul_div_floor};
use common::types::{HubAssetKey, PoolAction, PoolSupplyEntry, ScaledPositionRaw};
use controller::constants::RAY;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Vec as SVec};
use test_harness::{
    assert_contract_error, days, errors, eth_preset, hub_asset, usd, usdc_preset,
    usdt_stable_preset, LendingTest, ALICE, BOB, CAROL, DAVE, EVE, HARNESS_SPOKE,
};

const GATE: i128 = 1_050_000_000_000_000_000;

fn key(t: &LendingTest, name: &str) -> HubAssetKey {
    hub_asset(t.resolve_asset(name))
}

fn ray_of(t: &LendingTest, name: &str, amount: i128) -> i128 {
    amount * 10i128.pow(27 - t.resolve_market(name).decimals)
}

/// `(supply index, borrow index)` as stored by the pool.
fn indexes(t: &LendingTest, name: &str) -> (i128, i128) {
    let s = t.pool_client(name).get_sync_data(&key(t, name)).state;
    (s.supply_index, s.borrow_index)
}

fn supply_shares(t: &LendingTest, id: u64, name: &str) -> i128 {
    let (supplies, _) = t.ctrl_client().get_account_positions(&id);
    supplies.get(key(t, name)).map_or(0, |p| p.scaled_amount)
}

fn debt_shares(t: &LendingTest, id: u64, name: &str) -> i128 {
    let (_, borrows) = t.ctrl_client().get_account_positions(&id);
    borrows.get(key(t, name)).map_or(0, |p| p.scaled_amount)
}

fn legs(t: &LendingTest, pays: &[(&str, i128)]) -> SVec<(HubAssetKey, i128)> {
    let mut v = SVec::new(&t.env);
    for (name, amount) in pays {
        v.push_back((key(t, name), *amount));
    }
    v
}

fn mint(t: &LendingTest, who: &Address, name: &str, amount: i128) {
    t.resolve_market(name).token_admin.mint(who, &amount);
}

/// Supplies `pays` as one batch into `account` (0 creates one). Returns the id.
fn supply_legs(t: &mut LendingTest, user: &str, account: u64, pays: &[(&str, i128)]) -> u64 {
    let who = t.get_or_create_user(user);
    for (name, amount) in pays {
        mint(t, &who, name, *amount);
    }
    let v = legs(t, pays);
    t.ctrl_client().supply(&who, &account, &HARNESS_SPOKE, &v)
}

fn withdraw_legs(
    t: &mut LendingTest,
    user: &str,
    account: u64,
    pays: &[(&str, i128)],
) -> SVec<(HubAssetKey, i128)> {
    let who = t.get_or_create_user(user);
    let v = legs(t, pays);
    t.ctrl_client().withdraw(&who, &account, &v, &None)
}

fn borrow_legs(t: &mut LendingTest, user: &str, account: u64, pays: &[(&str, i128)]) {
    let who = t.get_or_create_user(user);
    let v = legs(t, pays);
    t.ctrl_client().borrow(&who, &account, &v, &None);
}

fn repay_legs(t: &mut LendingTest, user: &str, account: u64, pays: &[(&str, i128)]) {
    let who = t.get_or_create_user(user);
    for (name, amount) in pays {
        mint(t, &who, name, *amount);
    }
    let v = legs(t, pays);
    t.ctrl_client().repay(&who, &account, &v);
}

/// Three accrued markets: every index is off one RAY, so each boundary rounds.
fn accrued_book() -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .build();
    t.supply(BOB, "USDC", 200_000.0);
    t.supply(BOB, "USDT", 200_000.0);
    t.supply(BOB, "ETH", 200.0);
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "USDC", 20_000.0);
    t.borrow(ALICE, "USDT", 20_000.0);
    t.borrow(ALICE, "ETH", 5.0);
    t.advance_and_sync(days(200));
    for name in ["USDC", "USDT", "ETH"] {
        let (si, bi) = indexes(&t, name);
        assert!(si > RAY && bi > RAY, "{name}: indexes did not accrue");
        assert!(
            si % 1_000 != 0 && bi % 1_000 != 0,
            "{name}: indexes too round to exercise rounding"
        );
        std::println!("RV-R8 {name}: si={si} bi={bi}");
    }
    t
}

/// `(x, y)` raw-unit pairs for the duplicate-leg sweep: odd units, one and
/// many whole units, and a prime split.
const PAIRS: [(i128, i128); 8] = [
    (1, 1),
    (1, 2),
    (3, 7),
    (1, 10_000_000),
    (9_999_999, 1),
    (12_345_671, 7_654_329),
    (333_333_337, 666_666_663),
    (1_000_000_000_000, 1),
];

/// Smallest `x` whose `(x, x + 1)` split crosses a rounding boundary at
/// `index`: for `floor`, `floor(x) + floor(x + 1) + 1 == floor(2x + 1)`; for
/// ceil, `ceil(x) + ceil(x + 1) == ceil(2x + 1) + 1`. Keeps every sweep
/// index-agnostic: the fixed pairs alone may all land on the same side.
fn boundary_pair(t: &LendingTest, name: &str, index: i128, floor: bool) -> (i128, i128) {
    let f = |units: i128| {
        let ray = ray_of(t, name, units);
        if floor {
            mul_div_floor(&t.env, ray, RAY, index)
        } else {
            mul_div_ceil(&t.env, ray, RAY, index)
        }
    };
    (1..=100_000)
        .find(|x| {
            let (a, b, c) = (f(*x), f(x + 1), f(2 * x + 1));
            if floor {
                c - (a + b) == 1
            } else {
                (a + b) - c == 1
            }
        })
        .map(|x| (x, x + 1))
        .expect("an index off one RAY has a rounding boundary below 100k units")
}

fn sweep_pairs(extra: (i128, i128)) -> std::vec::Vec<(i128, i128)> {
    let mut pairs = PAIRS.to_vec();
    pairs.push(extra);
    pairs
}

// ---------------------------------------------------------------------------
// A. Controller duplicate legs against the integer model
// ---------------------------------------------------------------------------

/// Supply `[(A, x), (A, y)]`: the merged mint equals `floor((x + y) / si)` and
/// a single `x + y` leg; two transactions mint at most one raw share less.
#[test]
fn rv_round_r8_supply_duplicate_legs_merge_before_the_floor_within_one_raw_share() {
    let mut t = accrued_book();
    let (si, _) = indexes(&t, "USDC");
    let pairs = sweep_pairs(boundary_pair(&t, "USDC", si, true));
    let mut max_gap = 0;
    for (i, (x, y)) in pairs.iter().enumerate() {
        let (x, y) = (*x, *y);
        let dup = supply_legs(&mut t, &format!("dup{i}"), 0, &[("USDC", x), ("USDC", y)]);
        let one = supply_legs(&mut t, &format!("one{i}"), 0, &[("USDC", x + y)]);
        let seq = supply_legs(&mut t, &format!("seq{i}"), 0, &[("USDC", x)]);
        supply_legs(&mut t, &format!("seq{i}"), seq, &[("USDC", y)]);

        let model_one = mul_div_floor(&t.env, ray_of(&t, "USDC", x + y), RAY, si);
        let model_seq = mul_div_floor(&t.env, ray_of(&t, "USDC", x), RAY, si)
            + mul_div_floor(&t.env, ray_of(&t, "USDC", y), RAY, si);
        let s_dup = supply_shares(&t, dup, "USDC");
        let s_one = supply_shares(&t, one, "USDC");
        let s_seq = supply_shares(&t, seq, "USDC");
        assert_eq!(s_dup, model_one, "pair {i}: merged mint != floor((x+y)/si)");
        assert_eq!(s_dup, s_one, "pair {i}: merged mint != single leg");
        assert_eq!(
            s_seq, model_seq,
            "pair {i}: sequential != floor(x)+floor(y)"
        );
        let gap = s_dup - s_seq;
        assert!(
            (0..=1).contains(&gap),
            "pair {i}: merge gains {gap} raw shares"
        );
        max_gap = max_gap.max(gap);
        std::println!("RV-R8 supply ({x},{y}): dup={s_dup} seq={s_seq} gap={gap}");
    }
    assert_eq!(max_gap, 1, "the sweep never hit a rounding boundary");
    t.assert_spoke_usage_matches_positions();
}

/// Withdraw `[(A, x), (A, 0)]` and `[(A, 0), (A, x)]` close the position and
/// pay the floored claim like `[(A, 0)]`; `[(A, x), (A, y)]` burns
/// `ceil((x + y) / si)`, at most one raw share less than two transactions.
#[test]
fn rv_round_r8_withdraw_zero_leg_means_all_and_duplicates_merge_before_the_ceil() {
    let mut t = accrued_book();
    let (si, _) = indexes(&t, "USDC");
    let dec = t.resolve_market("USDC").decimals;
    let seed = 123_456_789_i128;
    let one_unit_ray = 10i128.pow(27 - dec);

    // Full-close variants on identical positions.
    let mut closers = std::vec::Vec::new();
    for (i, pays) in [
        &[("USDC", 0)][..],
        &[("USDC", 5), ("USDC", 0)][..],
        &[("USDC", 0), ("USDC", 5)][..],
        &[("USDC", 0), ("USDC", 0)][..],
    ]
    .iter()
    .enumerate()
    {
        let name = format!("close{i}");
        let id = supply_legs(&mut t, &name, 0, &[("USDC", seed)]);
        let shares = supply_shares(&t, id, "USDC");
        let floor_claim = mul_div_floor(&t.env, shares, si, RAY) / one_unit_ray;
        let paid = withdraw_legs(&mut t, &name, id, pays);
        assert_eq!(paid.len(), 1, "variant {i}: one merged leg");
        let (_, amount) = paid.get(0).unwrap();
        assert_eq!(amount, floor_claim, "variant {i}: not the floored claim");
        assert_eq!(supply_shares(&t, id, "USDC"), 0, "variant {i}: shares left");
        assert!(t.ctrl_client().get_account_positions(&id).0.is_empty());
        closers.push(amount);
    }
    assert!(closers.windows(2).all(|w| w[0] == w[1]), "{closers:?}");
    assert!(closers[0] <= seed, "closing pays more than deposited");

    // Partial duplicates against the ceil model.
    let pairs = sweep_pairs(boundary_pair(&t, "USDC", si, false));
    let mut max_gap = 0;
    for (i, (x, y)) in pairs.iter().enumerate() {
        let (x, y) = (*x, *y);
        if x + y >= seed {
            continue;
        }
        let dup = supply_legs(&mut t, &format!("wdup{i}"), 0, &[("USDC", seed)]);
        let seq = supply_legs(&mut t, &format!("wseq{i}"), 0, &[("USDC", seed)]);
        let p = supply_shares(&t, dup, "USDC");
        assert_eq!(p, supply_shares(&t, seq, "USDC"));

        let paid = withdraw_legs(
            &mut t,
            &format!("wdup{i}"),
            dup,
            &[("USDC", x), ("USDC", y)],
        );
        assert_eq!(paid.get(0).unwrap().1, x + y, "pair {i}: merged payout");
        withdraw_legs(&mut t, &format!("wseq{i}"), seq, &[("USDC", x)]);
        withdraw_legs(&mut t, &format!("wseq{i}"), seq, &[("USDC", y)]);

        let burned_dup = p - supply_shares(&t, dup, "USDC");
        let burned_seq = p - supply_shares(&t, seq, "USDC");
        let model_dup = mul_div_ceil(&t.env, ray_of(&t, "USDC", x + y), RAY, si);
        let model_seq = mul_div_ceil(&t.env, ray_of(&t, "USDC", x), RAY, si)
            + mul_div_ceil(&t.env, ray_of(&t, "USDC", y), RAY, si);
        assert_eq!(
            burned_dup, model_dup,
            "pair {i}: merged burn != ceil((x+y)/si)"
        );
        assert_eq!(
            burned_seq, model_seq,
            "pair {i}: sequential != ceil(x)+ceil(y)"
        );
        let gap = burned_seq - burned_dup;
        assert!(
            (0..=1).contains(&gap),
            "pair {i}: merge saves {gap} raw shares"
        );
        max_gap = max_gap.max(gap);
        std::println!("RV-R8 withdraw ({x},{y}): dup={burned_dup} seq={burned_seq} gap={gap}");
    }
    assert_eq!(max_gap, 1, "the sweep never hit a rounding boundary");
    t.assert_spoke_usage_matches_positions();
}

/// Borrow `[(D, x), (D, y)]` mints `ceil((x + y) / bi)`, at most one raw share
/// less than two transactions; repay `[(D, x), (D, y)]` burns
/// `floor((x + y) / bi)`, at most one raw share more than two transactions.
#[test]
fn rv_round_r8_borrow_and_repay_duplicate_legs_merge_within_one_raw_share() {
    let mut t = accrued_book();
    let (_, bi) = indexes(&t, "USDT");
    let collateral = 10_000_i128 * 10_000_000;
    let base = 1_000_i128 * 10_000_000;
    let mut pairs = sweep_pairs(boundary_pair(&t, "USDT", bi, false));
    pairs.push(boundary_pair(&t, "USDT", bi, true));
    let mut max_borrow_gap = 0;
    let mut max_repay_gap = 0;
    for (i, (x, y)) in pairs.iter().enumerate() {
        let (x, y) = (*x, *y);
        if x + y >= base {
            continue;
        }
        let dup = supply_legs(&mut t, &format!("bdup{i}"), 0, &[("USDC", collateral)]);
        let seq = supply_legs(&mut t, &format!("bseq{i}"), 0, &[("USDC", collateral)]);

        borrow_legs(
            &mut t,
            &format!("bdup{i}"),
            dup,
            &[("USDT", x), ("USDT", y)],
        );
        borrow_legs(&mut t, &format!("bseq{i}"), seq, &[("USDT", x)]);
        borrow_legs(&mut t, &format!("bseq{i}"), seq, &[("USDT", y)]);
        let d_dup = debt_shares(&t, dup, "USDT");
        let d_seq = debt_shares(&t, seq, "USDT");
        let model_dup = mul_div_ceil(&t.env, ray_of(&t, "USDT", x + y), RAY, bi);
        let model_seq = mul_div_ceil(&t.env, ray_of(&t, "USDT", x), RAY, bi)
            + mul_div_ceil(&t.env, ray_of(&t, "USDT", y), RAY, bi);
        assert_eq!(d_dup, model_dup, "pair {i}: merged mint != ceil((x+y)/bi)");
        assert_eq!(d_seq, model_seq, "pair {i}: sequential != ceil(x)+ceil(y)");
        let gap = d_seq - d_dup;
        assert!(
            (0..=1).contains(&gap),
            "pair {i}: merge saves {gap} debt shares"
        );
        max_borrow_gap = max_borrow_gap.max(gap);

        // Top both up to the same debt so the repay compares like with like,
        // then repay the pair partially: never reaching the ceiled balance.
        borrow_legs(&mut t, &format!("bdup{i}"), dup, &[("USDT", base)]);
        borrow_legs(&mut t, &format!("bseq{i}"), seq, &[("USDT", base)]);
        let p_dup = debt_shares(&t, dup, "USDT");
        let p_seq = debt_shares(&t, seq, "USDT");
        repay_legs(
            &mut t,
            &format!("bdup{i}"),
            dup,
            &[("USDT", x), ("USDT", y)],
        );
        repay_legs(&mut t, &format!("bseq{i}"), seq, &[("USDT", x)]);
        repay_legs(&mut t, &format!("bseq{i}"), seq, &[("USDT", y)]);
        let burned_dup = p_dup - debt_shares(&t, dup, "USDT");
        let burned_seq = p_seq - debt_shares(&t, seq, "USDT");
        let model_dup = mul_div_floor(&t.env, ray_of(&t, "USDT", x + y), RAY, bi);
        let model_seq = mul_div_floor(&t.env, ray_of(&t, "USDT", x), RAY, bi)
            + mul_div_floor(&t.env, ray_of(&t, "USDT", y), RAY, bi);
        assert_eq!(
            burned_dup, model_dup,
            "pair {i}: merged burn != floor((x+y)/bi)"
        );
        assert_eq!(
            burned_seq, model_seq,
            "pair {i}: sequential != floor(x)+floor(y)"
        );
        let gap = burned_dup - burned_seq;
        assert!(
            (0..=1).contains(&gap),
            "pair {i}: merge clears {gap} extra shares"
        );
        max_repay_gap = max_repay_gap.max(gap);
        std::println!(
            "RV-R8 borrow/repay ({x},{y}): mint dup={d_dup} seq={d_seq}; burn dup={burned_dup} seq={burned_seq}"
        );
    }
    assert_eq!(max_borrow_gap, 1, "the borrow sweep never hit a boundary");
    assert_eq!(max_repay_gap, 1, "the repay sweep never hit a boundary");
    t.assert_spoke_usage_matches_positions();
}

// ---------------------------------------------------------------------------
// B. The pool rounds per entry and trusts the caller's position per entry
// ---------------------------------------------------------------------------

/// Two entries for one market in one pool `supply` batch: each is floored on
/// its own and the second result is built on the input position, not on the
/// first result. Market supply grows by `floor(x) + floor(y)`.
#[test]
fn rv_round_r8_pool_batch_rounds_each_duplicate_entry_on_the_stale_position() {
    let t = accrued_book();
    let k = key(&t, "USDC");
    let (si, _) = indexes(&t, "USDC");
    let pool = t.pool_client("USDC");
    let market = t.resolve_market("USDC");
    let payer = Address::generate(&t.env);
    let (x, y, p) = (3_i128, 7_i128, 1_000_000_000_000_000_000_000_i128);
    market.token_admin.mint(&payer, &(x + y));
    token::Client::new(&t.env, &market.asset).transfer(&payer, &market.pool, &(x + y));
    let supplied_before = pool.get_sync_data(&k).state.supplied;

    let entry = |amount: i128| PoolSupplyEntry {
        action: PoolAction {
            position: ScaledPositionRaw { scaled_amount: p },
            amount,
            hub_asset: k.clone(),
        },
    };
    let results = pool.supply(&vec![&t.env, entry(x), entry(y)]);
    let fx = mul_div_floor(&t.env, ray_of(&t, "USDC", x), RAY, si);
    let fy = mul_div_floor(&t.env, ray_of(&t, "USDC", y), RAY, si);
    let fxy = mul_div_floor(&t.env, ray_of(&t, "USDC", x + y), RAY, si);
    assert_eq!(results.get(0).unwrap().position.scaled_amount, p + fx);
    assert_eq!(results.get(1).unwrap().position.scaled_amount, p + fy);
    assert_eq!(
        pool.get_sync_data(&k).state.supplied - supplied_before,
        fx + fy,
        "the market books both floors"
    );
    assert!(fxy - (fx + fy) <= 1 && fxy >= fx + fy);
    std::println!("RV-R8 pool dup: floor(x)={fx} floor(y)={fy} floor(x+y)={fxy}");
}

// ---------------------------------------------------------------------------
// C. update_account_threshold batches share one cache
// ---------------------------------------------------------------------------

type Stamp = (u32, u32, u32, u32);

fn stamp(t: &LendingTest, id: u64, name: &str) -> Stamp {
    let (supplies, _) = t.ctrl_client().get_account_positions(&id);
    let leg = supplies.get(key(t, name)).unwrap();
    (
        leg.loan_to_value,
        leg.liquidation_threshold,
        leg.liquidation_bonus,
        leg.liquidation_fees,
    )
}

/// Four USDC-collateral accounts owing 2.0, 2.5, 3.0 and 3.4 ETH; ETH then
/// moves to 2060 so the last sits under the 1.05 gate at the new threshold
/// (0.7 * 10_000 / 7_004 = 0.9994) but not at the stale one (1.142). A fifth
/// account owing 3.7 ETH sits under the gate at its stale threshold
/// (0.8 * 10_000 / 7_622 = 1.0496). USDC is relisted at 6_900 / 7_000 / 600.
fn threshold_book() -> (LendingTest, [u64; 5]) {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(DAVE, "ETH", 100.0);
    let users = [ALICE, BOB, CAROL, EVE, "frank"];
    let eth = [2.0, 2.5, 3.0, 3.4, 3.7];
    let mut ids = [0u64; 5];
    for (i, user) in users.iter().enumerate() {
        t.supply(user, "USDC", 10_000.0);
        t.borrow(user, "ETH", eth[i]);
        ids[i] = t.account_id(user);
    }
    t.set_price("ETH", usd(2_060));
    assert!(t.health_factor_for_raw("frank", ids[4]) < GATE);
    assert!(t.health_factor_for_raw(EVE, ids[3]) >= GATE);
    t.edit_asset_in_spoke("USDC", HARNESS_SPOKE, true, true, 6_900, 7_000, 600);
    (t, ids)
}

fn snapshot(t: &LendingTest, ids: &[u64]) -> std::vec::Vec<(Stamp, i128)> {
    ids.iter()
        .map(|id| (stamp(t, *id, "USDC"), t.health_factor_for_raw("", *id)))
        .collect()
}

#[test]
fn rv_round_r8_threshold_batch_equals_single_calls_in_every_order() {
    let (t_batch, ids) = threshold_book();
    let stale = stamp(&t_batch, ids[0], "USDC");
    t_batch.update_account_threshold(true, &ids[..4]);
    let batch = snapshot(&t_batch, &ids[..4]);

    let (t_single, ids_s) = threshold_book();
    for id in &ids_s[..4] {
        t_single.update_account_threshold(true, &[*id]);
    }
    let single = snapshot(&t_single, &ids_s[..4]);

    let (t_rev, ids_r) = threshold_book();
    let reversed: std::vec::Vec<u64> = ids_r[..4].iter().rev().copied().collect();
    t_rev.update_account_threshold(true, &reversed);
    let rev = snapshot(&t_rev, &ids_r[..4]);

    assert_eq!(batch, single, "batch != single calls");
    assert_eq!(batch, rev, "batch != reversed batch");
    for (i, (s, hf)) in batch.iter().enumerate() {
        std::println!("RV-R8 threshold acct{i}: stamp={s:?} hf={hf}");
        if i < 3 {
            assert_eq!(*s, (6_900, 7_000, 600, 0), "acct{i}: listing not applied");
        } else {
            // Gated: LTV refreshes, the liquidation tuple stays stale.
            assert_eq!(*s, (6_900, stale.1, stale.2, stale.3), "acct{i}: gate");
        }
        assert!(*hf >= GATE, "acct{i}: final gate");
    }

    // An account under 1.05 at its own stamps fails alone and fails the batch.
    assert_contract_error(
        t_batch.try_update_account_threshold(true, &[ids[4]]),
        errors::HEALTH_FACTOR_TOO_LOW,
    );
    assert_contract_error(
        t_batch.try_update_account_threshold(true, &ids),
        errors::HEALTH_FACTOR_TOO_LOW,
    );
    assert_eq!(
        snapshot(&t_batch, &ids[..4]),
        batch,
        "a reverted batch wrote"
    );
    t_batch.assert_spoke_usage_matches_positions();
}

// ---------------------------------------------------------------------------
// D. Bulk index simulation equals per-market and committed accrual
// ---------------------------------------------------------------------------

/// After a 400-day gap (two compounding chunks), the bulk view, the single
/// view, the controller view and the committed state agree bit for bit.
#[test]
fn rv_round_r8_bulk_indexes_match_per_market_and_committed_accrual() {
    let mut t = accrued_book();
    t.advance_time(days(400));
    let names = ["USDC", "USDT", "ETH"];
    let keys = legs(&t, &[("USDC", 0), ("USDT", 0), ("ETH", 0)]);
    let mut hub_assets = SVec::new(&t.env);
    for (k, _) in keys.iter() {
        hub_assets.push_back(k);
    }
    let pool = t.pool_client("USDC");
    let bulk = pool.get_bulk_indexes(&hub_assets);
    for (i, name) in names.iter().enumerate() {
        let k = key(&t, name);
        let single = pool
            .get_bulk_indexes(&vec![&t.env, k.clone()])
            .get(0)
            .unwrap();
        let view = t.ctrl_client().get_market_index(&k);
        let b = bulk.get(i as u32).unwrap();
        assert_eq!(
            (b.supply_index, b.borrow_index),
            (single.supply_index, single.borrow_index)
        );
        assert_eq!(
            (b.supply_index, b.borrow_index),
            (view.supply_index, view.borrow_index)
        );
        let stored = indexes(&t, name);
        assert!(
            b.supply_index > stored.0 && b.borrow_index > stored.1,
            "{name}: no gap"
        );
    }
    t.update_indexes_for(&names);
    for (i, name) in names.iter().enumerate() {
        let b = bulk.get(i as u32).unwrap();
        assert_eq!(
            indexes(&t, name),
            (b.supply_index, b.borrow_index),
            "{name}: drift"
        );
        std::println!(
            "RV-R8 bulk {name}: si={} bi={}",
            b.supply_index,
            b.borrow_index
        );
    }
}
