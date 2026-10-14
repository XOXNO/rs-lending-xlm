//! RV rounding audit, lenses R-10 (other fees) and R-11 (silent clamps), round 1.
//!
//! Integer mirrors, checked against exact rationals, of
//! * `common/src/math/fp.rs` `Bps::flash_loan_fee_on` (lines 296-303): the flash-loan
//!   fee and, through `contracts/pool/src/ops/strategy.rs` `compute_fee` (94-101),
//!   the strategy origination fee;
//! * `contracts/pool/src/ops/strategy.rs` `accounting` (58-88), driven through the
//!   deployed pool WASM at odd indexes: ceil debt mint, floor revenue mint, cash out
//!   `amount - fee`;
//! * `contracts/swap-aggregator/src/fees.rs` `fee_amount` (120-122) and
//!   `execute/mod.rs` `Mode::Ppm` (227-231): per-component and per-split floors, and
//!   `constants.rs` `residual_allowance` (142-144);
//! * `contracts/swap-aggregator/src/venues/soroswap.rs` `soroswap_fee` /
//!   `soroswap_amount_out` (22-43) against the UniswapV2 `getAmountOut` reference
//!   and the pair's `k` check;
//! * `common/src/rates/index.rs` `protocol_fee_shares` (94-99): the saturating
//!   division and the share-headroom cap, across the admitted supply-index domain.
//!
//! Every assertion is the documented bound (`docs/reference/formulas.md`, "Caps, fees,
//! and numeric limits" and "Compounding and interest allocation"; ADR-0011 for the
//! router residual policy):
//! * a flash or strategy fee is half-up BPS of principal with a minimum of one unit,
//!   so the protocol is never short by more than half a unit, the counterparty never
//!   pays more than one unit above the rate, and only the one-unit bump exceeds half;
//! * the pool books exactly that fee, mints ceil debt shares on the gross principal
//!   and floor revenue shares on the fee, and pays `amount - fee`;
//! * router fee components floor independently (total within two units of the exact
//!   combined rate, never above it), ppm splits leave under one unit per hop, and the
//!   residual allowance is `max(credited / 1e6, 1000)` units;
//! * the Soroswap adapter never requests more than the pair pays and under-requests
//!   by at most the value of one input unit plus one output unit;
//! * `protocol_fee_shares` never mints shares worth more than the fee, never
//!   saturates for a fee bounded by the market's supply value, and its headroom cap
//!   binds only at the share-domain edge.

use common::constants::{
    BPS, MAX_FLASHLOAN_FEE_BPS, MAX_SUPPLY_INDEX_RAY, RAY, SUPPLY_INDEX_FLOOR_RAW, WAD,
};
use common::math::fp::{Bps, Ray, Wad};
use common::rates::protocol_fee_shares;
use common::types::{HubAssetKey, PoolAction, PoolKey, PoolStateRaw, ScaledPositionRaw};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, ToPrimitive, Zero};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};
use test_harness::{hub_asset, LendingTest};

// ---------------------------------------------------------------------------
// Deterministic PRNG and integer helpers
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[lo, hi]` for non-negative bounds below `i128::MAX`.
    fn range(&mut self, lo: i128, hi: i128) -> i128 {
        assert!(lo <= hi);
        let span = (hi - lo) as u128 + 1;
        let wide = ((self.next() as u128) << 64) | self.next() as u128;
        lo + (wide % span) as i128
    }

    /// Log-uniform in `[1, hi]`: picks a decade first so small and large values both appear.
    fn log_range(&mut self, hi: i128) -> i128 {
        let decades = hi.ilog10() as i128;
        let d = self.range(0, decades);
        let top = 10i128.pow(d as u32).saturating_mul(10).min(hi);
        self.range(10i128.pow(d as u32), top)
    }
}

fn big(v: i128) -> BigInt {
    BigInt::from(v)
}

fn rat(v: i128) -> BigRational {
    BigRational::from_integer(big(v))
}

/// `floor(a / b)` for non-negative operands.
fn floor_div(a: &BigInt, b: &BigInt) -> BigInt {
    a / b
}

/// `ceil(a / b)` for non-negative operands.
fn ceil_div(a: &BigInt, b: &BigInt) -> BigInt {
    let q = a / b;
    if a % b == BigInt::zero() {
        q
    } else {
        q + 1
    }
}

/// `round_half_up(a / b)` for non-negative operands.
fn half_up_div(a: &BigInt, b: &BigInt) -> BigInt {
    (a + b / 2) / b
}

/// `fp.rs:296-303`: half-up BPS of `amount`, bumped to one unit for a positive rate.
fn model_fee(amount: i128, bps: i128) -> i128 {
    let fee = half_up_div(&(big(amount) * big(bps)), &big(BPS))
        .to_i128()
        .unwrap();
    if bps > 0 && fee == 0 {
        1
    } else {
        fee
    }
}

const DECIMALS: u32 = 7;
/// RAY per raw unit at 7 decimals.
const UNIT_RAY: i128 = 100_000_000_000_000_000_000;
const ODD_SUPPLY_INDEX: i128 = 1_234_567_891_234_567_891_234_567_891;
const ODD_BORROW_INDEX: i128 = 1_100_000_000_000_000_000_000_000_007;

// ---------------------------------------------------------------------------
// R-10a. Flash-loan and strategy fee: half-up BPS, minimum one unit, never above
// principal. Dense sweep covers every residue class of `amount * bps mod BPS`.
// ---------------------------------------------------------------------------

#[test]
fn rv_round_r10_flash_and_strategy_fee_within_half_unit_of_rate_and_never_above_principal() {
    let env = Env::default();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let half = BigRational::new(big(1), big(2));
    // Token-to-RAY input maximum at 7 decimals (formulas.md numeric limits).
    let max_amount = i128::MAX / UNIT_RAY;

    let mut checked = 0u64;
    let mut bumped = 0u64;
    let mut max_under = BigRational::zero();
    let mut max_under_at = (0i128, 0i128);
    let mut max_over = BigRational::zero();

    for bps in [1i128, 9, 30, 50, 100, MAX_FLASHLOAN_FEE_BPS] {
        let rate = Bps::from(bps);
        let mut amounts: Vec<i128> = (1..=20_000).collect();
        for _ in 0..20_000 {
            amounts.push(rng.log_range(max_amount));
        }
        for amount in amounts {
            let fee = rate.flash_loan_fee_on(&env, amount);
            assert_eq!(fee, model_fee(amount, bps), "bps {bps} amount {amount}");
            let exact = BigRational::new(big(amount) * big(bps), big(BPS));
            let fee_r = rat(fee);

            // Documented: minimum one unit for a positive rate; never above principal.
            assert!(fee >= 1, "bps {bps} amount {amount}: zero fee");
            assert!(
                fee <= amount,
                "bps {bps} amount {amount}: fee {fee} above principal"
            );

            // Protocol side: half-up never leaves the protocol short by more than half a unit.
            let under = &exact - &fee_r;
            assert!(
                under <= half,
                "bps {bps} amount {amount}: protocol short by {under} units"
            );
            if under > max_under {
                max_under = under.clone();
                max_under_at = (bps, amount);
            }

            // Counterparty side: never more than one unit above the rate, and more than
            // half a unit above it only through the one-unit bump.
            let over = &fee_r - &exact;
            assert!(
                over <= BigRational::one(),
                "bps {bps} amount {amount}: counterparty overpays {over} units"
            );
            if over > half {
                assert_eq!(
                    fee, 1,
                    "bps {bps} amount {amount}: over-half without the bump"
                );
                bumped += 1;
            }
            if over > max_over {
                max_over = over;
            }
            checked += 1;
        }
    }
    assert!(
        max_under < half,
        "a half-unit shortfall is the exact tie; half-up rounds it up"
    );
    println!(
        "R-10a flash/strategy fee: {checked} cases, {bumped} one-unit bumps, max protocol \
         shortfall {max_under} unit at bps {} amount {}, max counterparty overpay {max_over} unit",
        max_under_at.0, max_under_at.1
    );
}

// ---------------------------------------------------------------------------
// R-10b. Strategy origination fee through the pool WASM at odd indexes.
// ---------------------------------------------------------------------------

struct Book {
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    cash: i128,
}

fn read_book(t: &LendingTest, key: &HubAssetKey) -> Book {
    let s = t.pool_client("USDC").get_sync_data(key).state;
    Book {
        supplied: s.supplied,
        borrowed: s.borrowed,
        revenue: s.revenue,
        cash: s.cash,
    }
}

/// Fresh USDC market with `bps` strategy fee, unbounded utilization, and an injected
/// book: `supplied` shares at `supply_index`, no debt, `cash` units backed by tokens.
fn strategy_fixture(
    bps: u32,
    supply_index: i128,
    borrow_index: i128,
    supplied: i128,
    cash: i128,
) -> LendingTest {
    let mut preset = test_harness::usdc_preset();
    preset.initial_liquidity = 0.0;
    let t = LendingTest::new().with_market(preset).build();
    let market = t.resolve_market("USDC");
    let key: HubAssetKey = hub_asset(market.asset.clone());
    let pl = t.pool_client("USDC");
    let mut model = pl.get_sync_data(&key).params.rate_model_view();
    model.max_utilization = RAY;
    model.is_flashloanable = true;
    model.flashloan_fee = bps;
    pl.update_params(&key, &model);

    let now_ms = t.env.ledger().timestamp() * 1_000;
    t.env.as_contract(&market.pool, || {
        t.env.storage().persistent().set(
            &PoolKey::State(key.clone()),
            &PoolStateRaw {
                supplied,
                borrowed: 0,
                revenue: 0,
                borrow_index,
                supply_index,
                last_timestamp: now_ms,
                cash,
            },
        );
    });
    market.token_admin.mint(&market.pool, &cash);
    t
}

#[test]
fn rv_round_r10_pool_strategy_fee_books_half_up_fee_ceil_debt_and_floor_revenue_at_odd_indexes() {
    // (bps, supply index, borrow index, supplied shares): value about 1e12 units each.
    let cases = [
        (9u32, RAY, RAY, 1_000_000_000_000 * UNIT_RAY),
        (
            100u32,
            ODD_SUPPLY_INDEX,
            ODD_BORROW_INDEX,
            1_000_000_000_000 * UNIT_RAY,
        ),
        (
            500u32,
            SUPPLY_INDEX_FLOOR_RAW,
            ODD_BORROW_INDEX,
            1_000_000_000_000 * UNIT_RAY * 1_000,
        ),
    ];
    let amounts = [
        1i128,
        2,
        555,
        1_666,
        9_999,
        10_000,
        12_345_678,
        123_456_789,
        10_000_000_000,
    ];
    let mut largest_unrepresented = BigInt::zero();
    for (bps, si, bi, supplied) in cases {
        let cash = 2_000_000_000_000i128;
        let t = strategy_fixture(bps, si, bi, supplied, cash);
        let key: HubAssetKey = hub_asset(t.resolve_asset("USDC"));
        let pl = t.pool_client("USDC");
        let receiver = Address::generate(&t.env);
        for amount in amounts {
            let before = read_book(&t, &key);
            let m = pl.create_strategy(
                &receiver,
                &PoolAction {
                    position: ScaledPositionRaw { scaled_amount: 0 },
                    amount,
                    hub_asset: key.clone(),
                },
                &true,
            );
            let after = read_book(&t, &key);

            let fee = model_fee(amount, i128::from(bps));
            let amount_ray = big(amount) * big(UNIT_RAY);
            let fee_ray = big(fee) * big(UNIT_RAY);
            // strategy.rs:70 -> borrow.rs:68: ceil debt shares on the gross principal.
            let debt_shares = ceil_div(&(&amount_ray * big(RAY)), &big(bi));
            // strategy.rs:72-73 -> index.rs:94-99: floor revenue shares on the fee.
            let fee_shares = floor_div(&(&fee_ray * big(RAY)), &big(si));

            assert_eq!(m.actual_amount, amount, "bps {bps} si {si} amount {amount}");
            assert_eq!(
                m.amount_received,
                amount - fee,
                "bps {bps} si {si} amount {amount}"
            );
            assert_eq!(
                big(m.position.scaled_amount),
                debt_shares,
                "bps {bps} si {si} amount {amount}"
            );
            assert_eq!(
                big(after.borrowed - before.borrowed),
                debt_shares,
                "debt mint"
            );
            assert_eq!(
                big(after.revenue - before.revenue),
                fee_shares,
                "revenue mint"
            );
            assert_eq!(
                big(after.supplied - before.supplied),
                fee_shares,
                "supplied mint"
            );
            assert_eq!(
                after.cash - before.cash,
                -(amount - fee),
                "cash out is amount - fee"
            );

            // Protocol claim on the fee: within one raw share of the fee, never above it.
            let booked_value = floor_div(&(&fee_shares * big(si)), &big(RAY));
            assert!(
                booked_value <= fee_ray,
                "revenue shares worth more than the fee"
            );
            let unrepresented = &fee_ray - &booked_value;
            assert!(
                unrepresented < big(si),
                "more than one raw share unrepresented"
            );
            if unrepresented > largest_unrepresented {
                largest_unrepresented = unrepresented;
            }
        }
        println!(
            "R-10b pool strategy fee bps {bps} si {si} bi {bi}: {} amounts booked exactly",
            amounts.len()
        );
    }
    println!(
        "R-10b largest fee value unrepresented by revenue shares: {largest_unrepresented} raw RAY \
         (one raw unit is {UNIT_RAY} RAY)"
    );
}

// ---------------------------------------------------------------------------
// R-10c. Router static + referral fee components, ppm splits, residual allowance.
// ---------------------------------------------------------------------------

#[test]
fn rv_round_r10_router_fee_components_floor_within_two_units_and_ppm_splits_within_one_unit_per_hop(
) {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    const FEE_CAP: i128 = 1_000;
    const PPM: i128 = 1_000_000;

    let mut max_short_vs_combined_floor = 0i128;
    let mut max_short_vs_exact = BigRational::zero();
    for _ in 0..200_000 {
        let balance = rng.log_range(100_000_000_000_000_000_000);
        let static_bps = rng.range(0, FEE_CAP);
        let referral_bps = rng.range(0, FEE_CAP - static_bps);
        // fees.rs:120-122 per component; 80-82 sums them.
        let total = balance * static_bps / BPS + balance * referral_bps / BPS;
        let combined_floor = balance * (static_bps + referral_bps) / BPS;
        assert!(
            total <= combined_floor,
            "components above the combined floor"
        );
        assert!(
            combined_floor - total <= 1,
            "components short of the combined floor by over one unit"
        );
        let exact = BigRational::new(big(balance) * big(static_bps + referral_bps), big(BPS));
        let short = exact - rat(total);
        assert!(
            short < BigRational::from_integer(big(2)),
            "short of the exact rate by two units"
        );
        max_short_vs_combined_floor = max_short_vs_combined_floor.max(combined_floor - total);
        if short > max_short_vs_exact {
            max_short_vs_exact = short;
        }
    }

    // execute/mod.rs:227-231: each split is a floor of the available balance.
    let mut max_split_remainder = BigRational::zero();
    for _ in 0..200_000 {
        let available = rng.log_range(100_000_000_000_000_000_000);
        let weight = rng.range(1, PPM);
        let part = available * weight / PPM;
        assert!(part <= available);
        let remainder = BigRational::new(big(available) * big(weight), big(PPM)) - rat(part);
        assert!(
            remainder < BigRational::one(),
            "a split leaves a whole unit"
        );
        if remainder > max_split_remainder {
            max_split_remainder = remainder;
        }
    }

    // constants.rs:142-144: leftover up to this allowance becomes admin revenue; above
    // it the strategy reverts (ADR-0011, "capped admin-revenue policy").
    let allowance = |credited: i128| (credited / PPM).max(1_000);
    assert_eq!(allowance(1), 1_000);
    assert_eq!(allowance(1_000_000_000), 1_000);
    assert_eq!(allowance(10_000_000_000_000), 10_000_000);

    println!(
        "R-10c router fees: max component shortfall vs combined floor {max_short_vs_combined_floor} \
         unit, vs exact {max_short_vs_exact} unit; max ppm split remainder {max_split_remainder} unit; \
         residual allowance 1,000 units below 1e9 credited, 1 ppm above (1 whole 7-decimal token \
         per 1e6 tokens credited)"
    );
}

// ---------------------------------------------------------------------------
// R-10d. Soroswap adapter versus the UniswapV2 reference and the pair's k check.
// ---------------------------------------------------------------------------

/// `soroswap.rs:22-43`.
fn adapter_out(a: &BigInt, r_in: &BigInt, r_out: &BigInt) -> BigInt {
    let zero = BigInt::zero();
    if *a <= zero || *r_in <= zero || *r_out <= zero {
        return zero;
    }
    let fee = ceil_div(&(a * 3), &big(1_000));
    let in_less = a - fee;
    if in_less <= zero {
        return zero;
    }
    floor_div(&(&in_less * r_out), &(r_in + &in_less))
}

/// UniswapV2 `getAmountOut` with a 30 BPS fee: `floor(997 a R_out / (1000 R_in + 997 a))`.
fn reference_out(a: &BigInt, r_in: &BigInt, r_out: &BigInt) -> BigInt {
    floor_div(&(a * 997 * r_out), &(r_in * 1_000 + a * 997))
}

/// UniswapV2 pair invariant after a swap that takes `out` for `a` in.
fn k_holds(a: &BigInt, r_in: &BigInt, r_out: &BigInt, out: &BigInt) -> bool {
    let adjusted_in = (r_in + a) * 1_000 - a * 3;
    let adjusted_out = (r_out - out) * 1_000;
    adjusted_in * adjusted_out >= r_in * r_out * 1_000_000
}

#[test]
fn rv_round_r10_soroswap_adapter_never_over_requests_and_under_requests_at_most_one_unit_each_side()
{
    let mut rng = Rng(0x5851_F42D_4C95_7F2D);
    let mut checked = 0u64;
    let mut zero_output = 0u64;
    let mut max_gap = BigInt::zero();
    let mut max_gap_over_price = BigRational::zero();
    for i in 0..150_000 {
        let (a, r_in, r_out) = if i % 3 == 0 {
            // Skewed reserves: cheap output token, up to 1e12 output units per input unit.
            let r_in = big(rng.log_range(1_000_000_000_000_000_000));
            let r_out = &r_in * big(rng.log_range(1_000_000_000_000));
            (big(rng.log_range(1_000_000_000_000_000_000)), r_in, r_out)
        } else {
            (
                big(rng.log_range(1_000_000_000_000_000_000)),
                big(rng.log_range(1_000_000_000_000_000_000_000_000)),
                big(rng.log_range(1_000_000_000_000_000_000_000_000)),
            )
        };
        let adapter = adapter_out(&a, &r_in, &r_out);
        let reference = reference_out(&a, &r_in, &r_out);
        // Never above what the pair pays, so the pair's k check always admits the request.
        assert!(
            adapter <= reference,
            "a {a} r_in {r_in} r_out {r_out}: over-request"
        );
        if adapter > BigInt::zero() {
            assert!(k_holds(&a, &r_in, &r_out, &adapter), "k check fails");
        } else {
            zero_output += 1;
        }
        // Under-request bounded by one input unit priced in output units, plus one output unit.
        let gap = &reference - &adapter;
        let bound = floor_div(&r_out, &r_in) + 1;
        assert!(
            gap <= bound,
            "a {a} r_in {r_in} r_out {r_out}: gap {gap} above {bound}"
        );
        if gap > max_gap {
            max_gap = gap.clone();
        }
        if r_out > r_in {
            let over_price = BigRational::new(gap, floor_div(&r_out, &r_in));
            if over_price > max_gap_over_price {
                max_gap_over_price = over_price;
            }
        }
        checked += 1;
    }
    println!(
        "R-10d soroswap adapter: {checked} cases, {zero_output} fail closed with zero output, \
         max under-request {max_gap} output units, at most {max_gap_over_price} of one input unit's \
         output price"
    );
}

// ---------------------------------------------------------------------------
// R-11. protocol_fee_shares: saturation and the share-headroom cap.
// ---------------------------------------------------------------------------

#[test]
fn rv_round_r11_protocol_fee_share_clamps_never_bind_for_supply_bounded_fees_and_only_lower_the_mint(
) {
    let env = Env::default();
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let indexes = [
        SUPPLY_INDEX_FLOOR_RAW,
        SUPPLY_INDEX_FLOOR_RAW + 1,
        RAY - 1,
        RAY,
        RAY * 3 / 2,
        ODD_SUPPLY_INDEX,
        MAX_SUPPLY_INDEX_RAY - 1,
        MAX_SUPPLY_INDEX_RAY,
    ];
    let max = big(i128::MAX);
    let mut cap_bound = 0u64;
    let mut checked = 0u64;
    for si in indexes {
        // Supplied shares whose value still fits the RAY domain (formulas.md: market totals
        // must independently fit; `supplied.mul(index)` panics otherwise).
        let max_supplied = (&max * big(RAY) / big(si))
            .min(max.clone())
            .to_i128()
            .unwrap();
        for _ in 0..3_000 {
            let supplied = rng.range(1, max_supplied);
            let value = floor_div(&(big(supplied) * big(si)), &big(RAY))
                .to_i128()
                .unwrap();
            // Every fee the pool books is bounded by cash, a gross withdrawal or a seized
            // position, all at most the market's supply value.
            let fee = rng.range(0, value);
            let shares =
                protocol_fee_shares(&env, Ray::from(fee), Ray::from(si), Ray::from(supplied));
            let exact_floor = floor_div(&(big(fee) * big(RAY)), &big(si));
            // No saturation: fee <= value implies fee * RAY / si <= supplied <= i128::MAX.
            assert!(exact_floor <= big(supplied));
            let headroom = &max - big(supplied);
            let expected = exact_floor.clone().min(headroom.clone());
            assert_eq!(
                big(shares.raw()),
                expected,
                "si {si} supplied {supplied} fee {fee}"
            );
            // Direction: never worth more than the fee.
            let worth = floor_div(&(big(shares.raw()) * big(si)), &big(RAY));
            assert!(worth <= big(fee), "si {si}: shares worth more than the fee");
            if expected != exact_floor {
                cap_bound += 1;
                assert!(
                    big(supplied) + exact_floor > max,
                    "headroom cap bound below the share-domain edge"
                );
            }
            if supplied <= i128::MAX / 2 {
                assert_eq!(
                    big(shares.raw()),
                    floor_div(&(big(fee) * big(RAY)), &big(si))
                );
            }
            checked += 1;
        }
    }

    // The one reachable saturation: an interest fee after a write-down to the floor.
    // Book before the write-down: 1.7e11 whole 7-decimal tokens at index one RAY, the
    // token-to-RAY maximum. 99.9% written off leaves supplied = 1.7e38 shares at the
    // floor and up to 0.001 of the old value, 1.7e8 whole tokens, of remaining debt.
    // One idle year at the 200% cap with a 100% reserve factor accrues
    // (e^2 - 1) * 1.7e8 = 1.09e9 whole tokens of fee.
    let supplied = i128::MAX - 1_000;
    let fee_whole: i128 = 1_086_000_000;
    let fee = fee_whole * 10i128.pow(DECIMALS) * UNIT_RAY;
    let threshold = floor_div(&(&max * big(SUPPLY_INDEX_FLOOR_RAW)), &big(RAY));
    assert!(
        big(fee) > threshold,
        "the interest-fee extreme saturates mul_div_floor_saturating"
    );
    let shares = protocol_fee_shares(
        &env,
        Ray::from(fee),
        Ray::from(SUPPLY_INDEX_FLOOR_RAW),
        Ray::from(supplied),
    );
    assert_eq!(
        shares.raw(),
        1_000,
        "saturated mint capped to the remaining headroom"
    );
    let worth = floor_div(
        &(big(shares.raw()) * big(SUPPLY_INDEX_FLOOR_RAW)),
        &big(RAY),
    );
    assert!(worth <= big(fee), "the protocol never gains from the cap");
    let unrepresented_whole = (big(fee) - &worth) / big(RAY);
    println!(
        "R-11 protocol_fee_shares: {checked} supply-bounded cases, headroom cap bound {cap_bound} \
         times (only when supplied + floor(fee/index) > i128::MAX); interest-fee extreme at the \
         floor index leaves {unrepresented_whole} whole tokens of fee unrepresented, all to the \
         protocol's loss"
    );

    // HF saturation (`fp.rs:207`, `totals.rs:206`): floor(W * WAD / D) saturates only when
    // W / D exceeds i128::MAX / WAD, about 1.7e20. Any debt of at least one cent against
    // collateral up to 1e18 USD stays exact.
    for _ in 0..2_000 {
        let w = Wad::from(rng.log_range(1_000_000_000_000_000_000_000_000_000_000_000_000i128));
        let d = Wad::from(rng.range(
            10_000_000_000_000_000i128,
            1_000_000_000_000_000_000_000_000_000_000_000_000i128,
        ));
        let hf = w.div_floor_saturating(&env, d);
        let exact = floor_div(&(big(w.raw()) * big(WAD)), &big(d.raw()));
        assert_eq!(
            big(hf.raw()),
            exact,
            "HF saturated inside the admitted domain"
        );
    }
    let w = Wad::from(1_000_000_000_000_000_000_000_000_000_000_000_000i128);
    let d = Wad::from(1i128);
    assert_eq!(
        w.div_floor_saturating(&env, d).raw(),
        i128::MAX,
        "a 1e-18 USD debt against 1e18 USD saturates to the debt-free sentinel"
    );
}
