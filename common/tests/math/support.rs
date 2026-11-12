//! Exact reference arithmetic for the fixed-point tests.
//!
//! [`Wide`] is a 256-bit two's-complement integer: wide enough for the product
//! of any two `i128` values plus a few `i128` terms. The predicates state each
//! rounding rule through its defining inequalities and never divide, so they
//! share no arithmetic with the code under test.

extern crate std;

use core::ops::{Add, Neg, Sub};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::string::String;

/// A 256-bit two's-complement integer. The derived ordering compares the
/// signed high half first and the unsigned low half second, which is numeric
/// order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Wide {
    hi: i128,
    lo: u128,
}

impl Wide {
    /// The exact product `x * y`.
    pub fn product(x: i128, y: i128) -> Wide {
        const LOW: u128 = u64::MAX as u128;
        let (a, b) = (x.unsigned_abs() >> 64, x.unsigned_abs() & LOW);
        let (c, d) = (y.unsigned_abs() >> 64, y.unsigned_abs() & LOW);
        let (bd, ad, bc, ac) = (b * d, a * d, b * c, a * c);
        let mid = (bd >> 64) + (ad & LOW) + (bc & LOW);
        let magnitude = Wide {
            // Both magnitudes are at most 2^127, so the high half is at most 2^126.
            hi: (ac + (ad >> 64) + (bc >> 64) + (mid >> 64)) as i128,
            lo: (bd & LOW) | (mid << 64),
        };
        if (x < 0) != (y < 0) {
            -magnitude
        } else {
            magnitude
        }
    }

    pub fn is_negative(self) -> bool {
        self.hi < 0
    }

    fn doubled(self) -> Wide {
        self + self
    }
}

impl From<i128> for Wide {
    fn from(v: i128) -> Wide {
        Wide {
            hi: if v < 0 { -1 } else { 0 },
            lo: v as u128,
        }
    }
}

impl Add for Wide {
    type Output = Wide;

    /// Panics if the sum leaves the 256-bit range, so the reference can never
    /// wrap silently.
    fn add(self, rhs: Wide) -> Wide {
        let (lo, carry) = self.lo.overflowing_add(rhs.lo);
        let hi = self
            .hi
            .checked_add(rhs.hi)
            .and_then(|hi| hi.checked_add(i128::from(carry)))
            .expect("Wide addition overflowed 256 bits");
        Wide { hi, lo }
    }
}

impl Neg for Wide {
    type Output = Wide;

    fn neg(self) -> Wide {
        let lo = (!self.lo).wrapping_add(1);
        let hi = (!self.hi)
            .checked_add(i128::from(lo == 0))
            .expect("Wide negation overflowed 256 bits");
        Wide { hi, lo }
    }
}

impl Sub for Wide {
    type Output = Wide;

    fn sub(self, rhs: Wide) -> Wide {
        self + (-rhs)
    }
}

/// `q == floor(n / d)` for a nonzero `d`: `q*d <= n < (q+1)*d`, with the
/// inequalities reversed for a negative `d`.
pub fn is_floor(q: i128, n: Wide, d: i128) -> bool {
    let qd = Wide::product(q, d);
    let next = qd + Wide::from(d);
    if d > 0 {
        qd <= n && n < next
    } else {
        qd >= n && n > next
    }
}

/// `q == ceil(n / d)` for a nonzero `d`: `(q-1)*d < n <= q*d`, with the
/// inequalities reversed for a negative `d`.
pub fn is_ceil(q: i128, n: Wide, d: i128) -> bool {
    let qd = Wide::product(q, d);
    let previous = qd - Wide::from(d);
    if d > 0 {
        previous < n && n <= qd
    } else {
        previous > n && n >= qd
    }
}

/// `qd / d` is `n / d` rounded to nearest with exact halves up, for `n >= 0`
/// and `d > 0`: `2*qd <= 2*n + d < 2*qd + 2*d`.
fn is_nearest(qd: Wide, n: Wide, d: i128) -> bool {
    let biased = n.doubled() + Wide::from(d);
    qd.doubled() <= biased && biased < qd.doubled() + Wide::from(d).doubled()
}

/// `q` is `n / d` rounded half up, for `n >= 0` and `d > 0`.
pub fn is_half_up(q: i128, n: Wide, d: i128) -> bool {
    is_nearest(Wide::product(q, d), n, d)
}

/// `q` is `n / d` rounded to nearest with exact halves away from zero, for
/// any `n` and `d > 0`.
pub fn is_half_away(q: i128, n: Wide, d: i128) -> bool {
    let qd = Wide::product(q, d);
    if n.is_negative() {
        is_nearest(-qd, -n, d)
    } else {
        is_nearest(qd, n, d)
    }
}

/// True when `floor(n / d)` is representable in `i128`, for a nonzero `d`.
pub fn floor_fits(n: Wide, d: i128) -> bool {
    let low = Wide::product(i128::MIN, d);
    let high = Wide::product(i128::MAX, d) + Wide::from(d);
    if d > 0 {
        low <= n && n < high
    } else {
        low >= n && n > high
    }
}

/// True when `ceil(n / d)` is representable in `i128`, for a nonzero `d`.
pub fn ceil_fits(n: Wide, d: i128) -> bool {
    let low = Wide::product(i128::MIN, d) - Wide::from(d);
    let high = Wide::product(i128::MAX, d);
    if d > 0 {
        low < n && n <= high
    } else {
        low > n && n >= high
    }
}

/// True when `n / d` rounded half up is representable in `i128`, for
/// `n >= 0` and `d > 0`.
pub fn half_up_fits(n: Wide, d: i128) -> bool {
    let limit = Wide::product(i128::MAX, d).doubled() + Wide::from(d).doubled();
    n.doubled() + Wide::from(d) < limit
}

/// The smallest `q` for which `holds(q)` is true. `holds` must be true at
/// `i128::MAX` and, once true, stay true for every larger `q`.
fn first_true(holds: impl Fn(i128) -> bool) -> i128 {
    let (mut low, mut high) = (i128::MIN, i128::MAX);
    while low < high {
        // The lower midpoint, without overflowing `high - low`.
        let mid = (low & high) + ((low ^ high) >> 1);
        if holds(mid) {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    low
}

/// `floor(x * y / d)` for a positive `d`, or `None` when it does not fit
/// `i128`. Bisects the inequality [`is_floor`] states, so it never divides.
pub fn floor_of(x: i128, y: i128, d: i128) -> Option<i128> {
    let n = Wide::product(x, y);
    floor_fits(n, d).then(|| first_true(|q| n < Wide::product(q, d) + Wide::from(d)))
}

/// `ceil(x * y / d)` for a positive `d`, or `None` when it does not fit
/// `i128`. Bisects the inequality [`is_ceil`] states.
pub fn ceil_of(x: i128, y: i128, d: i128) -> Option<i128> {
    let n = Wide::product(x, y);
    ceil_fits(n, d).then(|| first_true(|q| n <= Wide::product(q, d)))
}

/// `x * y / d` rounded half up, for `x * y >= 0` and a positive `d`, or `None`
/// when it does not fit `i128`. Bisects the inequality [`is_half_up`] states.
pub fn half_up_of(x: i128, y: i128, d: i128) -> Option<i128> {
    let n = Wide::product(x, y);
    let biased = n.doubled() + Wide::from(d);
    half_up_fits(n, d)
        .then(|| first_true(|q| biased < Wide::product(q, d).doubled() + Wide::from(d).doubled()))
}

/// True when the exact quotient `n / d` is strictly negative, for a nonzero `d`.
pub fn quotient_negative(n: Wide, d: i128) -> bool {
    n != Wide::from(0) && n.is_negative() != (d < 0)
}

/// How a decimal downscale rounds.
#[derive(Clone, Copy, Debug)]
pub enum Rounding {
    /// To nearest, exact halves away from zero.
    HalfAway,
    /// Toward zero.
    TowardZero,
    /// Toward positive infinity.
    Ceil,
}

/// The largest decimal gap whose power of ten fits `i128`.
const MAX_EXACT_GAP: u32 = 38;

/// True when rescaling `a` from `from` to `to` decimals is representable:
/// every downscale is, and an upscale is when both the power of ten and the
/// scaled value fit `i128`.
pub fn rescale_fits(a: i128, from: u32, to: u32) -> bool {
    if to <= from {
        return true;
    }
    let gap = to - from;
    if gap > MAX_EXACT_GAP {
        return false;
    }
    let scaled = Wide::product(a, 10i128.pow(gap));
    Wide::from(i128::MIN) <= scaled && scaled <= Wide::from(i128::MAX)
}

/// True when `r` is `a` rescaled from `from` to `to` decimals: exact when
/// upscaling, rounded by `rounding` when downscaling. Requires
/// `rescale_fits(a, from, to)`.
pub fn is_rescaled(r: i128, a: i128, from: u32, to: u32, rounding: Rounding) -> bool {
    if from == to {
        return r == a;
    }
    if to > from {
        return Wide::product(a, 10i128.pow(to - from)) == Wide::from(r);
    }
    let gap = from - to;
    if gap > MAX_EXACT_GAP {
        // 10^39 exceeds twice the largest `i128` magnitude, so the quotient
        // lies strictly between -1/2 and 1/2.
        return match rounding {
            Rounding::HalfAway | Rounding::TowardZero => r == 0,
            Rounding::Ceil => r == i128::from(a > 0),
        };
    }
    let (n, factor) = (Wide::from(a), 10i128.pow(gap));
    match rounding {
        Rounding::HalfAway => is_half_away(r, n, factor),
        Rounding::TowardZero if a >= 0 => is_floor(r, n, factor),
        Rounding::TowardZero | Rounding::Ceil => is_ceil(r, n, factor),
    }
}

/// Magnitudes at the boundaries the arithmetic branches on: the smallest
/// values, the protocol scales, the 64-bit limb boundary, the largest value
/// whose square fits `i128` and its successor, and the top of the range.
const EDGE_MAGNITUDES: [i128; 12] = [
    1,
    2,
    3,
    1_000_000_000_000_000_000,
    1_000_000_000_000_000_000_000_000_000,
    1 << 63,
    1 << 64,
    13_043_817_825_332_782_212,
    13_043_817_825_332_782_213,
    i128::MAX / 2 + 1,
    i128::MAX - 1,
    i128::MAX,
];

/// Zero, both signs of every edge magnitude, and `i128::MIN`.
pub fn edges() -> impl Iterator<Item = i128> {
    core::iter::once(0)
        .chain(EDGE_MAGNITUDES.into_iter().flat_map(|m| [m, -m]))
        .chain(core::iter::once(i128::MIN))
}

/// A deterministic xorshift64* generator, so every run checks the same inputs.
pub struct Sampler(u64);

impl Sampler {
    pub fn new(seed: u64) -> Sampler {
        Sampler(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A uniform value below `bound`.
    pub fn below(&mut self, bound: u32) -> u32 {
        (self.next() % u64::from(bound)) as u32
    }

    /// A non-negative value whose bit length is uniform over `0..=127`, so
    /// every magnitude class is as likely as any other.
    pub fn non_negative(&mut self) -> i128 {
        let bits = self.below(128);
        let raw = (u128::from(self.next()) << 64) | u128::from(self.next());
        if bits == 0 {
            0
        } else {
            (raw >> (128 - bits)) as i128
        }
    }

    /// A non-negative value no larger than `max`. Draws that already fit keep
    /// their uniform bit length; the rest wrap into the range.
    pub fn at_most(&mut self, max: i128) -> i128 {
        let value = self.non_negative();
        if value > max {
            value % (max + 1)
        } else {
            value
        }
    }

    /// A value of either sign whose magnitude has a uniform bit length.
    pub fn signed(&mut self) -> i128 {
        let magnitude = self.non_negative();
        if self.below(2) == 0 {
            magnitude
        } else {
            -magnitude
        }
    }
}

/// Runs `f` and returns the contract error code it panics with, or `None` if
/// it returns normally or panics without a contract error.
pub fn contract_error_code(f: impl FnOnce()) -> Option<u32> {
    let payload = catch_unwind(AssertUnwindSafe(f)).err()?;
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())?;
    let digits = message.split("Error(Contract, #").nth(1)?;
    digits
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}
