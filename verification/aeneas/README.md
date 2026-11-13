# Aeneas + Lean proofs of the lending arithmetic

Machine-checked proofs about the production Rust in `common/`,
`contracts/pool/` and `contracts/controller/`. [Charon](https://github.com/AeneasVerif/charon)
and [Aeneas](https://github.com/AeneasVerif/aeneas) translate the Rust into
Lean 4, and the theorems are stated about that translation and checked by Lean.

Unlike the Kani harnesses (`proof/kani-math`) and most Certora rules, these
proofs need no input bounds: a theorem quantifies over every `i128`/`u128`
value the code can receive, subject only to the stated domain hypotheses.
The full check takes under a minute.

* **How to write a proof here, conventions, solved pitfalls:** [WRITING-PROOFS.md](WRITING-PROOFS.md)
* **Every theorem, with its Rust function, hypotheses and invariant IDs:** the area documents below

## What is proved

1004 theorems: four area libraries plus `FixedPoint`, the rounding
specifications they share (headline theorems plus lemmas; some math proofs
exist once per library, since each library carries its own extraction of the
shared `common` code, and `sync_copies.py` writes those copies).

**Production revision covered:** `main` at `3d96ffc3` (#227) plus the fix for
audit findings S-03 and S-04, which is the last change to the proved code
(before it: `c5e6def0`, "fix(rates): round borrow-curve slopes once", #225,
on top of #224). See
[Changes for audit findings S-03 and S-04](#changes-for-audit-findings-s-03-and-s-04),
[Changes after #224](#changes-after-224-typed-fixed-point-boundary) and the
first finding below. Every theorem depends only on Lean's standard axioms (`propext`,
`Classical.choice`, `Quot.sound`); there is no `sorry`, `admit` or added
axiom. The build enforces this: each library's root file ends with
`#assert_standard_axioms` (`lean/Audit.lean`), which fails on any other
axiom in the generated code, the external models or the proofs.

| Area | Lean library | Document | Highlights |
|---|---|---|---|
| Fixed-point core, floor/ceil, share conversions | `RatesAeneas` | [docs/rates-floor-ceil.md](docs/rates-floor-ceil.md) | `mul_div_floor`/`mul_div_ceil` exact for every `i128` input (native and `I256` paths); saturating floor never aborts; `rescale_*` exact; every `scaling.rs` conversion has the ADR-0003 direction; deposit→claim never pays more, borrow→repay never owes less; `resolve_withdrawal`/`resolve_repay`/`resolve_net_settle` never over-burn or over-pay; every typed `Ray`/`Wad`/`Bps` wrapper (#224) specified |
| Half-up core, rate curve, index accrual | `RatesAeneas` | [docs/rates-half-up-curve-index.md](docs/rates-half-up-curve-index.md) | `mul_div_half_up` and `div_by_int_half_up` exact; flash-loan fee in `[1, amount]`; **borrow rate monotone across all segments** (#225) and exact at every kink, in `[base, max]` under exactly `InterestRateModel::verify`; **`utilization` in `[0, RAY]` for all non-negative totals** (S-03); deposit rate ≤ borrow rate; borrow index monotone and capped (INV-IDX-01); rewards + fee = accrued interest (INV-IDX-05) |
| Pool accounting and operation legs | `PoolAeneas` | [docs/pool.md](docs/pool.md) | `revenue ≤ supplied` preserved everywhere (INV-ACCT-01); guards' exact pass conditions (INV-ACCT-04/07/08/09/11), including the division-free utilization gate at every ceiling (S-03) and the entry ceiling; `compound_interest` bounded; **`accrue_step` and `global_sync` never abort, for any elapsed time and any size of the market values** (interest is held at the value ceiling, S-04, INV-IDX-06), and keep indexes monotone and capped (INV-IDX-01/02/04/05); supply/borrow/withdraw/repay/net-settle/seize/recapitalize/strategy/revenue legs: exact accounting, `PoolInv` inductive; **11 of 13 Certora pool summaries discharged** |
| Controller liquidation and risk | `ControllerAeneas` | [docs/controller.md](docs/controller.md), [docs/controller-estimate.md](docs/controller-estimate.md), [docs/controller-seizure.md](docs/controller-seizure.md), [docs/controller-repayment.md](docs/controller-repayment.md) | valuation floor ≤ half-up ≤ ceil (INV-RISK-02); account risk totals conservative (collateral floored, debt ceiled); bonus curve bounded and antitone for every validated curve; `estimate_liquidation_amount` never aborts in the production domain, repays ≤ debt, leaves no dust, hits the target HF within 5.5·10⁻¹⁸ USD; repayment capped per leg with excess refunded exactly; `normalize_repayment_plan` / `process_excess_payment` conserve tokens (kept + refunded = offered) and never release more than the unbacked part; `calculate_seized_collateral` never aborts in its domain, seizes ≤ held, bonus ≤ seized, fee ≤ amount, within the budget up to stated rounding; **health factor after the executed plan ≥ before** (HF-preserving cap) and **≥ the curve target** (target branch), each up to an explicit rounding allowance; under-delivery scales seizures down end to end (INV-LIQ-03); bad-debt socialisation condition exact (INV-LIQ-04) |
| LP oracle pricing | `OracleAeneas` | [docs/oracle.md](docs/oracle.md) | `isqrt_of_product` exact for every `u128` pair (Certora cannot prove it); LP price fails closed, symmetric, exact formula; **StableSwap solver never traps**, `D` within `D*−1 < D ≤ D*+5` of the exact root, stable price formula exact |

## Findings in the production code

Each reproduced independently of the agent that found it.

1. **Fixed by #225: the annual borrow rate was not monotone across curve
   segment boundaries.** Each segment's contribution was rounded twice,
   scaling the first rounding error by `RAY/range`: with base 0, all slopes
   `4·10²⁵+2`, mid `3·10²⁶`, optimal `8·10²⁶`, max `2·RAY` (passes `verify`),
   `rate(mid−1) = 4·10²⁵+3 > rate(mid) = 4·10²⁵+2`; on a one-unit-wide
   segment a 60% slope gave 100% APR. #225 rounds once, and
   `calculate_annual_borrow_rate_monotone` proves the rate never decreases.
2. **`withdraw_summary` (Certora) is false for zero-amount withdrawals of
   dust.** It assumes `amount == 0` leaves the position unchanged, but the
   code closes any position whose half-up value is 0: a 0-decimal asset at
   supply index `RAY`, position 1 scaled unit, `amount = 0` burns the share
   (`withdraw_zero_amount_closes_dust`). Every controller rule using the
   summary assumes behaviour production does not have; economically
   negligible (10⁻²⁷ of a unit). Fix the summary: allow `new_scaled = 0` when
   `amount == 0`.
3. **Two Certora estimate rules never exercise the curve.**
   `ideal_repayment_targets_curve_hf` and
   `estimate_leaves_no_sub_threshold_dust` set
   `total_collateral = total_debt` and derive `proportion_seized ≥ hf`, so the
   HF-preserving bonus cap is ≤ 0 and the estimate always returns
   `(debt, 0)`; `liquidation_at_target` and the dust logic are never reached
   (`certora_estimate_rules`). Draw `total_collateral ≥ total_debt`
   independently.
4. **The liquidation seizure budget rounds half up**, not in the protocol's
   favour: repay 10 WAD-units at a 500 bps bonus gives 11, exactly 10.5. So
   "seized value ≤ repaid × (1 + bonus)" holds only up to sub-10⁻¹⁸ USD per
   rounding step; seizures stay capped by the held balance.
5. **The post-liquidation health factor can drop by rounding at the exact
   bonus cap**, so "post-HF ≥ pre-HF" holds only up to the rounding
   allowance `health_after_hf_preserving` states. USD 1100 of collateral at a
   50% threshold against USD 1000 of debt (all 18 decimals, price 1, index
   `RAY`) puts the cap at exactly 1000 BPS; repaying 1001 raw units seizes
   1101 (half-up budget) and the production HF goes from `0.55` to
   `0.55 − 10⁻¹⁸` (`ha_counterexample`). One 10⁻¹⁸ unit; not exploitable.
6. **Closed by the S-03 fix: `calculate_deposit_rate` could exceed the borrow
   rate when utilization was above `RAY`** (after bad debt); only the pool
   rate view read it. `utilization` now returns a value in `[0, RAY]`
   (`utilization_spec`).
7. **`update_supply_index` early exits skip the documented clamp**;
   unreachable while stored indexes stay ≤ `MAX`, which is proved.
8. Provably unreachable checks (harmless): the `burn_claimable_revenue`
   assert, the `remaining_excess_usd.checked_sub(removed_usd)` branch of
   `process_excess_payment` in keep-within-quote mode, re-checks in `split_seized_shares`, `share_supply_wad <= 0` in both
   LP pricers, and one branch of `liquidation_at_target`.
9. A solvent partial plan whose excess is below one native unit's value
   appends a refund of 0 (`process_excess_payment`); cosmetic, partial-plan
   refunds are only reported.
10. Edge behaviour outside production use: `rescale` upscaling by ≥ 39
   decimals aborts even for 0; `rescale_floor` truncates toward zero for
   negative inputs.

## Changes after #224 (typed fixed-point boundary)

#224 made `fp_core` private and routed every caller through new typed
wrappers in `common/src/math/fp.rs` (`Ray::div_floor_saturating`,
`Ray::try_div`, `Ray::mul_wad_ratio_floor`, `Bps::apply_to_ceil`,
`Bps::apply_to_ray_ceil`, `Bps::from_fraction_ceil`, `Wad::try_value_of_token`,
`Wad::try_from_token_upscale`, ...). `fp_core.rs` itself did not change.

All four libraries were re-extracted from the new code and re-checked:

* **Every theorem proved before #224 still holds, with its statement
  unchanged.** Each new wrapper makes the same `fp_core` call, with the same
  arguments in the same order, as the code it replaced, so no exact value or
  rounding direction changed. Only proofs that stepped through the old raw
  calls were adjusted to go through the wrapper.
* **The new wrappers have their own specs** (exact rounding, return/abort
  conditions): `RatesAeneas/Proofs/TypedWrappers.lean` covers every wrapper in
  the rates extraction; `PoolAeneas`, `ControllerAeneas` and `OracleAeneas`
  each have a `Proofs/Wrappers.lean` (`Fp.lean` in the oracle) for the
  wrappers their code reaches.
* One trusted-base addition: `i128::from(u64)` (pool, reached only through
  `compound_interest`), modelled as the same integer and proved never to fail.
* The oracle crate now includes `fp.rs`; `try_amount_to_wad` lost its `env`
  parameter in production, and the theorems followed.
* The new `scripts/check_fp_boundary.py` gate scans `common/`, `contracts/`
  and `interfaces/` only, so the proof crates (which include `fp_core`
  verbatim on purpose) are outside it. It passes on this revision.

Each area document has a section with the details.

## Changes for audit findings S-03 and S-04

The Runtime Verification audit's findings S-03 (utilization guard skipped at
a 100% ceiling) and S-04 (accrual overflows once a market value passes
`i128`, blocking every exit) were fixed in the production code, and the four
libraries were re-extracted and re-checked.

* **S-03.** `utilization` compares before it divides and returns a value in
  `[0, RAY]` (`utilization_spec`, no overflow case left).
  `require_utilization_below_max` compares
  `⌈debt value⌉ ≤ ⌊⌊supply value⌋·max_utilization/RAY⌋`, divides nothing, and
  applies at every ceiling on debt mints
  (`require_utilization_below_max_spec`); withdrawals and revenue claims use
  an exit form that skips a 100% ceiling
  (`require_exit_utilization_below_max_spec`).
* **S-04.** `accrue_step` holds interest at `MAX_MARKET_VALUE_RAY`.
  `accrue_step_spec`, `accrue_chunk_spec` and `synced_market_spec` no longer
  assume that the market values fit, and `global_sync_spec` holds for any
  elapsed time (before: at most one year, and only under two fit
  hypotheses). `accrue_step_within_ceiling`: accrual never takes a total
  across the ceiling. The entry ceiling is specified
  (`require_within_entry_ceiling_spec`) and the supply, borrow and strategy
  theorems conclude that the committed cache passes it.
* **Trusted base:** one addition, the event emitter
  `events.emit_market_value_ceiling`, modelled as returning `()`.
* Controller and oracle libraries: regenerated code only (a new error
  variant and one new `fp.rs` wrapper); no proof changed.

Details: [docs/pool.md](docs/pool.md) and
[docs/rates-half-up-curve-index.md](docs/rates-half-up-curve-index.md).

## Trusted base

1. **Charon and Aeneas translate the Rust faithfully.** Pinned: Aeneas
   `nightly-2026.11.12-0d21a9e`, which bundles the matching Charon (commit
   `454586e`), and its Lean library at the same commit (`lean/lakefile.lean`).
2. **The hand-written `TypesExternal.lean`/`FunsExternal.lean` of each
   library**, documented in its area doc:
   * the Soroban host `I256` (and in the oracle `U256`) as mathematical
     integers with the host's traps;
   * a few Rust core functions the Aeneas library does not model yet
     (`checked_pow`, `abs_diff`, `Range::contains`, `Option` combinators,
     `isqrt`, ...), each following the Rust documentation;
   * Soroban `Vec` as a Lean list and `Address` as an opaque value
     (controller, pool);
   * one event emitter returning `()` (pool: the value-ceiling event that
     `global_sync` publishes).
3. **`panic_with_error!` and `assert_with_error!` abort.** A theorem
   `f ⦃ r => P r ⦄` proves `f` returns (does not abort) with `P`. Error codes
   of aborts are not modelled (the Kani harnesses check them); returned `Err`
   values are.

4. **Integer overflow aborts.** Aeneas translates `+`, `-`, `*` on `i128`,
   `u32`, `u64` as aborting on overflow. Production behaves that way because
   the root `Cargo.toml` sets `overflow-checks = true` under
   `[profile.release]`; `run.sh` fails if it no longer does.

Production files are compiled verbatim through `#[path]` includes; each area
crate documents which modules are included verbatim and which are stand-ins.
Where a crate carries a copy of production code instead (the controller's
`types.rs` conversions, `cached_price`, the payment aggregation and a few
helpers), `check_standins.py` compares the copy with production, and pins
the hash of each production item whose stand-in is an adaptation; `run.sh`
runs it first.

## Running

```sh
export AENEAS_HOME=/path/to/unpacked/aeneas-release
./run.sh                 # all four areas
./run.sh pool oracle     # selected areas
```

`run.sh` fails if

* a hand-copied stand-in differs from production (`check_standins.py`), a
  copied proof file is out of date (`sync_copies.py --check`), or the release
  profile no longer traps on overflow;
* a regenerated `Funs.lean`/`Types.lean` differs from the committed one (the
  production code changed; `ALLOW_DRIFT=1` re-checks anyway);
* a proof breaks, or any declaration depends on `sorry` or a non-standard
  axiom.

It downloads the prebuilt Mathlib before building.

### Setup

* Aeneas: download `aeneas-linux-x86_64.tar.gz` from the
  [`nightly-2026.11.12-0d21a9e`](https://github.com/AeneasVerif/aeneas/releases/tag/nightly-2026.11.12-0d21a9e)
  release and unpack it; that directory is `AENEAS_HOME`.
* Charon's Rust toolchain: `rustup toolchain install nightly-2026-09-17 --profile minimal -c rustc-dev -c llvm-tools -c rust-src`.
* Lean: [elan](https://github.com/leanprover/elan). `lean/lean-toolchain`
  pins `v4.36.0-rc2`.

First run: about 2 GB of downloads (Lean and the Mathlib cache) and about
3 minutes to build the Aeneas Lean library; afterwards a full check takes
under a minute.

## Not covered yet

Each area doc lists its gaps with the blocker. The main ones:

* That every pool leg keeps the market value ceiling as a stored-state
  invariant. Accrual keeps it (`accrue_step_within_ceiling`) and the entry
  legs pass a ceiling eight times lower, but that the exit legs never raise a
  market value is argued in `docs/reference/invariants.md`, not proved; the
  exit legs' own conversions still assume their values fit `i128`.
* Liquidation execution side: `apply_liquidation_seizures`, the share credit
  and the pool repay are modelled (`SupplyAfter`/`DebtAfter`), not proved from
  the calls; transfer-mode seizure can burn one more share than modelled. The
  seizure domain (`SzLegOK`, budget near the collateral) and the post-account
  totals domain are hypotheses of the health theorems; non-abort is proved on
  sufficient, not exact, domains.
* `get_sync_data_summary` and the market-creation/rate-model clauses; token
  transfers, events and batch drivers (outside the model).
* That the StableSwap loop always stops before its 255-round cap (if it did
  not, it returns `InvalidPrice`, which is covered).
* Error codes of aborts; CI integration.
