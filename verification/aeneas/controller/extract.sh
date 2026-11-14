#!/usr/bin/env bash
# Re-extracts the controller liquidation and risk code to Lean (lean/ControllerAeneas).
# Usage: AENEAS_HOME=/path/to/aeneas-release ./extract.sh
# Charon is started only from the functions below (and translates what they
# call); see ../docs/extraction.md for the stand-ins and the opaque items.
set -euo pipefail
cd "$(dirname "$0")"
: "${AENEAS_HOME:?set AENEAS_HOME to the unpacked Aeneas release}"
# risk/totals.rs is generated from production (one fn-pointer substitution).
./gen_totals.py
M=crate::positions::liquidation::math
A=crate::positions::liquidation::apply
START=(
  crate::rates::value::position_value
  crate::rates::value::position_value_floor
  crate::rates::value::position_value_ceil
  crate::math::fp::Bps::apply_to_wad_floor
  crate::positions::liquidation::curve::is_socializable_bad_debt
  crate::positions::liquidation::curve::calculate_linear_bonus_with_target
  crate::positions::liquidation::curve::max_bonus_for_threshold
  crate::positions::liquidation::curve::max_hf_preserving_bonus_bps
  crate::positions::liquidation::curve::estimate_liquidation_amount
  $M::split_seized_shares
  $M::sum_repaid_usd
  $M::scale_seizures_to_received
  $M::calculate_seized_collateral
  $M::calculate_repayment_amounts
  $M::get_account_bonus_params
  $M::normalize_repayment_plan
  $M::release_unbacked_repayment
  $M::calculate_seizure_proportions
  $M::process_excess_payment
  $A::apply_liquidation_repayments
  $A::apply_liquidation_seizures
  $A::apply_liquidation_share_credit
  $A::credited_shares
  $A::check_bad_debt_after_liquidation
  crate::risk::totals::calculate_account_risk_totals
  crate::risk::totals::calculate_ltv_collateral_wad
  crate::risk::totals::sum_debt_usd
)
# Kept opaque: host/storage/cross-contract effects, modelled in FunsExternal.lean.
OPAQUE=(
  'crate::context::_::load_markets'
  'crate::context::_::cached_pool_address'
  'crate::context::_::require_spoke_asset'
  'crate::context::_::apply_spoke_exit'
  'crate::context::_::record_supply_position_update'
  crate::external::fetch_market_index
  crate::external::pool::pool_seize_positions_call
  crate::payment_legs::transfer_amount_measured
  $M::repays_every_debt_leg
  crate::positions::enforce_spoke_asset_flags
  crate::positions::apply_repay_batch
  crate::positions::apply_withdraw_batch
  crate::positions::liquidation::bad_debt::execute_bad_debt_cleanup
  crate::account_store::cleanup_account_if_empty
  crate::account_store::update_or_remove_supply_position
)
ARGS=()
for s in "${START[@]}"; do ARGS+=(--start-from "$s"); done
for s in "${OPAQUE[@]}"; do ARGS+=(--opaque "$s"); done
mkdir -p target/llbc
# --inline-anon-consts works around an Aeneas internal error (InterpAbs.ml)
# on `while x > Wad::ZERO && ...` loops (process_excess_payment); it leaves
# the translation of every other function unchanged. See ../docs/extraction.md.
"$AENEAS_HOME/charon" cargo --preset=aeneas --inline-anon-consts --dest-file target/llbc/controller.llbc "${ARGS[@]}"
"$AENEAS_HOME/aeneas" -backend lean -dest "${AENEAS_LEAN_DEST:-../lean}" -subdir ControllerAeneas -split-files target/llbc/controller.llbc
# The hand-written external files live in FunsExternal.lean / TypesExternal.lean.
[ -n "${KEEP_TEMPLATES:-}" ] || rm -f "${AENEAS_LEAN_DEST:-../lean}"/ControllerAeneas/*_Template.lean
