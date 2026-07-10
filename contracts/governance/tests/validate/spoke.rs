use super::*;

const DEFAULT_TARGET_HF_WAD: i128 = 1_020_000_000_000_000_000;
const DEFAULT_HF_FOR_MAX_BONUS_WAD: i128 = DEFAULT_TARGET_HF_WAD / 2;

#[test]
fn validate_liquidation_curve_accepts_protocol_defaults() {
    let env = Env::default();
    validate_liquidation_curve(
        &env,
        DEFAULT_TARGET_HF_WAD,
        DEFAULT_HF_FOR_MAX_BONUS_WAD,
        10_000,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #134)")]
fn validate_liquidation_curve_rejects_target_hf_at_one() {
    let env = Env::default();
    validate_liquidation_curve(
        &env,
        1_000_000_000_000_000_000,
        500_000_000_000_000_000,
        10_000,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #134)")]
fn validate_liquidation_curve_rejects_hf_for_max_bonus_above_target() {
    let env = Env::default();
    validate_liquidation_curve(
        &env,
        DEFAULT_TARGET_HF_WAD,
        DEFAULT_TARGET_HF_WAD + 1,
        10_000,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #134)")]
fn validate_liquidation_curve_rejects_bonus_factor_above_bps() {
    let env = Env::default();
    validate_liquidation_curve(
        &env,
        DEFAULT_TARGET_HF_WAD,
        DEFAULT_HF_FOR_MAX_BONUS_WAD,
        10_001,
    );
}
