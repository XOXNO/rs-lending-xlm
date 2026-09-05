#![allow(dead_code)]
#[path = "/Users/mihaieremia/GitHub/rs-lending-xlm/common/src/errors.rs"]
mod errors;
#[path = "/Users/mihaieremia/GitHub/rs-lending-xlm/common/src/constants/mod.rs"]
mod constants;
mod math {
    #[path = "/Users/mihaieremia/GitHub/rs-lending-xlm/common/src/math/fp_core.rs"]
    pub mod fp_core;
    #[path = "/Users/mihaieremia/GitHub/rs-lending-xlm/common/src/math/fp.rs"]
    pub mod fp;
}
use math::fp_core::*;
use math::fp::{Ray, Wad, Bps};
use soroban_sdk::Env;
use std::panic::{catch_unwind, AssertUnwindSafe};

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    let mut checked = 0usize;
    for (row, line) in include_str!("mul-div.tsv").lines().enumerate() {
        let fields: Vec<_> = line.split('\t').collect();
        let x: i128 = fields[0].parse().unwrap();
        let y: i128 = fields[1].parse().unwrap();
        let d: i128 = fields[2].parse().unwrap();
        if fields[3] != "_" {
            assert_eq!(mul_div_floor(&env, x, y, d), fields[3].parse::<i128>().unwrap(), "floor row {row}");
            checked += 1;
        }
        if fields[4] != "_" {
            assert_eq!(mul_div_ceil(&env, x, y, d), fields[4].parse::<i128>().unwrap(), "ceil row {row}");
            checked += 1;
        }
        assert_eq!(mul_div_floor_saturating(&env, x, y, d), fields[5].parse::<i128>().unwrap(), "saturating row {row}");
        assert_eq!(try_mul_div_half_up(&env, x, y, d), fields[6].parse::<i128>().ok(), "half_up row {row}");
        checked += 2;
    }
    println!("20,840 Python big-integer cases; {checked} mul/div assertions passed");

    let mut conversions = 0;
    for decimals in 0..=18 {
        let ceiling = i128::MAX / 10i128.pow(27 - decimals);
        for amount in [0, 1, 7, ceiling / 3, ceiling - 1, ceiling] {
            let ray = Ray::from_asset(&env, amount, decimals);
            assert_eq!(ray.to_asset(&env, decimals), amount);
            assert_eq!(ray.to_asset_floor(&env, decimals), amount);
            assert_eq!(ray.to_asset_ceil(&env, decimals), amount);
            let wad = Wad::from_token(&env, amount, decimals);
            assert_eq!(wad.to_token(&env, decimals), amount);
            assert_eq!(wad.to_token_floor(&env, decimals), amount);
            assert_eq!(wad.to_ray(&env).raw(), ray.raw());
            conversions += 6;
        }
    }
    for basis_points in 0..=10_000 {
        assert_eq!(Bps::from(basis_points).to_wad(&env).raw(), basis_points * 100_000_000_000_000i128);
    }
    println!("{conversions} conversion assertions and 10,001 exact Bps-to-Wad ratios passed");

    for diff in 1..=38 {
        let factor = 10i128.pow(diff);
        for a in [0, 1, 5, i128::MAX / 2, i128::MAX - 1, i128::MAX] {
            let half_up = a / factor + i128::from(a % factor >= factor / 2);
            assert_eq!(rescale_half_up(&env, a, diff, 0), half_up);
            assert_eq!(rescale_floor(&env, a, diff, 0), a / factor);
            assert_eq!(rescale_ceil(&env, a, diff, 0), a / factor + i128::from(a % factor != 0));
        }
    }
    for diff in [39, 100, u32::MAX] {
        for a in [i128::MIN, -1, 0, 1, i128::MAX] {
            assert_eq!(rescale_half_up(&env, a, diff, 0), 0);
            assert_eq!(rescale_floor(&env, a, diff, 0), 0);
            assert_eq!(rescale_ceil(&env, a, diff, 0), i128::from(a > 0));
        }
    }
    for (name, function) in [
        ("rescale_half_up(MIN, 1, 0)", rescale_min as fn(&Env) -> i128),
        ("div_by_int_half_up(MIN, 2)", div_min),
        ("div_by_int_half_up(MAX, 2)", div_max),
    ] {
        let panic = catch_unwind(AssertUnwindSafe(|| function(&env))).unwrap_err();
        let message = panic.downcast_ref::<String>().map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied()).unwrap_or("non-string panic");
        println!("Reproduced {name}: {message}");
    }
    println!("729 nonnegative/extreme-factor rescaling checks passed; 3 documented/latent edge panics reproduced");
}

fn rescale_min(env: &Env) -> i128 { rescale_half_up(env, i128::MIN, 1, 0) }
fn div_min(env: &Env) -> i128 { div_by_int_half_up(env, i128::MIN, 2) }
fn div_max(env: &Env) -> i128 { div_by_int_half_up(env, i128::MAX, 2) }
