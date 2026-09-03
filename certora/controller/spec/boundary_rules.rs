use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::Env;

use crate::constants::{BAD_DEBT_USD_THRESHOLD, WAD};
use crate::positions::liquidation::curve::is_socializable_bad_debt;
use common::math::fp::Wad;

#[rule]
fn bad_debt_socialization_threshold_boundary(e: Env, debt_wad: i128, collateral_wad: i128) {
    let _ = e;
    cvlr_assume!(debt_wad > 0 && debt_wad <= 1_000_000 * WAD);
    cvlr_assume!(collateral_wad >= 0 && collateral_wad <= 1_000_000 * WAD);

    let socializable = is_socializable_bad_debt(Wad::from(debt_wad), Wad::from(collateral_wad));

    if collateral_wad > BAD_DEBT_USD_THRESHOLD {
        cvlr_assert!(!socializable);
    }
    if debt_wad <= collateral_wad {
        cvlr_assert!(!socializable);
    }
    if debt_wad > collateral_wad && collateral_wad <= BAD_DEBT_USD_THRESHOLD {
        cvlr_assert!(socializable);
    }
}

/// Predicate-level mirror of the `BadDebtGate::Insolvent` arm reached only through the
/// owner-gated `force_socialize_bad_debt` entrypoint
/// ([`lib.rs`](contracts/controller/src/lib.rs) `force_socialize_bad_debt` →
/// [`liquidation/mod.rs`](contracts/controller/src/positions/liquidation/mod.rs)
/// `BadDebtGate::admits`, `Self::Insolvent => totals.total_debt > totals.total_collateral`).
///
/// `BadDebtGate` is private to `positions::liquidation`, so the spec module cannot name it;
/// the predicate is restated here and anchored to production code by
/// `bad_debt_straddle_admitted_by_force_gate`, which asserts that the dust gate
/// (`curve::is_socializable_bad_debt`) is exactly this predicate conjoined with the dust cap.
/// If the two insolvency comparisons ever diverge, that assertion fails.
fn insolvent_gate_admits(total_debt: Wad, total_collateral: Wad) -> bool {
    total_debt > total_collateral
}

/// Aave-comparison V-7, first half. An attacker who tops `total_collateral` to strictly above
/// `BAD_DEBT_USD_THRESHOLD` — `BAD_DEBT_USD_THRESHOLD + 1` is the cheapest such state — blocks
/// the permissionless dust-gated socialization path while staying insolvent. Unlike Aave's
/// count-based `activeCollateralCount` gate (ToB-AAVE-1, Blackthorn L-3), this is not reachable
/// with 1 wei of a second collateral: the gate is value-based, so the attacker must post real
/// value above the threshold and keep it there.
#[rule]
fn bad_debt_straddle_blocks_dust_gate(e: Env, debt_wad: i128, collateral_wad: i128) {
    let _ = e;
    cvlr_assume!(collateral_wad >= BAD_DEBT_USD_THRESHOLD + 1);
    cvlr_assume!(collateral_wad <= 1_000_000 * WAD);
    cvlr_assume!(debt_wad > collateral_wad && debt_wad <= 2_000_000 * WAD);

    // Symbolic straddle: anywhere strictly above the cap, the dust gate is shut.
    cvlr_assert!(!is_socializable_bad_debt(
        Wad::from(debt_wad),
        Wad::from(collateral_wad)
    ));

    // Explicit +1 witness: one wad-wei above the cap is already enough to block it.
    cvlr_assert!(!is_socializable_bad_debt(
        Wad::from(BAD_DEBT_USD_THRESHOLD + 2),
        Wad::from(BAD_DEBT_USD_THRESHOLD + 1)
    ));
}

/// Aave-comparison V-7, second half. The owner-gated force path
/// (`force_socialize_bad_debt`, `BadDebtGate::Insolvent`) still admits exactly the straddling
/// state that `bad_debt_straddle_blocks_dust_gate` shows the dust gate rejects. This pins the
/// escape hatch as load-bearing: the dust gate does not cover the whole insolvent domain, so
/// removing the force path would leave straddled bad debt permanently unsocializable.
#[rule]
fn bad_debt_straddle_admitted_by_force_gate(e: Env, debt_wad: i128, collateral_wad: i128) {
    let _ = e;
    cvlr_assume!(debt_wad >= 0 && debt_wad <= 2_000_000 * WAD);
    cvlr_assume!(collateral_wad >= 0 && collateral_wad <= 2_000_000 * WAD);

    let debt = Wad::from(debt_wad);
    let collateral = Wad::from(collateral_wad);
    let dust_gate = is_socializable_bad_debt(debt, collateral);
    let force_gate = insolvent_gate_admits(debt, collateral);

    // Containment: the force gate is strictly looser, so it never rejects what the
    // permissionless path accepts.
    cvlr_assert!(!dust_gate || force_gate);

    // Anchor: the dust gate is exactly the force gate conjoined with the dust cap. Keeps the
    // mirrored `Insolvent` predicate from drifting away from production silently.
    cvlr_assert!(dust_gate == (force_gate && collateral_wad <= BAD_DEBT_USD_THRESHOLD));

    // The straddle separates the two gates: dust blocked, force admits.
    if collateral_wad > BAD_DEBT_USD_THRESHOLD && debt_wad > collateral_wad {
        cvlr_assert!(!dust_gate);
        cvlr_assert!(force_gate);
    }

    // Explicit +1 witness, on the same concrete point the dust-gate rule rejects.
    let witness_collateral = Wad::from(BAD_DEBT_USD_THRESHOLD + 1);
    let witness_debt = Wad::from(BAD_DEBT_USD_THRESHOLD + 2);
    cvlr_assert!(!is_socializable_bad_debt(witness_debt, witness_collateral));
    cvlr_assert!(insolvent_gate_admits(witness_debt, witness_collateral));
}

/// Reachability witness for the V-7 straddle at exactly
/// `collateral == BAD_DEBT_USD_THRESHOLD + 1 && debt == collateral + 1`: the two gates really do
/// split there, so neither straddle rule is vacuous.
#[rule]
fn bad_debt_straddle_gate_split_sanity(e: Env, debt_wad: i128, collateral_wad: i128) {
    let _ = e;
    cvlr_assume!(collateral_wad == BAD_DEBT_USD_THRESHOLD + 1);
    cvlr_assume!(debt_wad == collateral_wad + 1);

    let debt = Wad::from(debt_wad);
    let collateral = Wad::from(collateral_wad);

    cvlr_satisfy!(
        !is_socializable_bad_debt(debt, collateral) && insolvent_gate_admits(debt, collateral)
    );
}
