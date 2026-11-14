# Aeneas + Lean proofs of the lending arithmetic

Machine-checked proofs about the production Rust in `common/`,
`contracts/pool/` and `contracts/controller/`. They cover the fixed-point
math, the rate curve and indexes, the pool's accounting, the liquidation math
and the LP oracle pricing.

[Charon](https://github.com/AeneasVerif/charon) and
[Aeneas](https://github.com/AeneasVerif/aeneas) translate the Rust into Lean 4.
Each theorem is stated about that translation and checked by Lean. It holds
for every input the code can receive. The only limits on the values are the
hypotheses written in its statement.

* **Size:** 1031 theorems in five Lean libraries.
* **Hidden assumptions:** none. No `sorry`, `admit` or added axiom, and the
  build enforces it.
* **Run it:** `./run.sh` (see [Running](#running)). No CI workflow runs it.
  Run it after any change to the covered code.

The area documents below list every theorem with its Rust function,
hypotheses and invariant IDs. To add or repair a proof, read
[WRITING-PROOFS.md](WRITING-PROOFS.md).

## What is proved

A theorem `f args ⦃ r => P r ⦄` has two parts. The call returns (it does not
panic), and its result satisfies `P`. Both hold under the hypotheses of the
statement. "Never aborts" means "returns for every input in the stated
domain".

**Fixed-point math and share conversions** (`RatesAeneas`,
[details](docs/rates-floor-ceil.md))

* `mul_div_floor`, `mul_div_ceil` and `mul_div_half_up` return the exactly
  rounded result for every `i128` input.
* Every conversion in `scaling.rs` rounds in the direction
  [ADR-0003](../../docs/explanation/decisions.md#adr-0003) requires. A deposit
  is never claimed back for more than was paid in, and a debt is never repaid
  for less than is owed.
* `resolve_withdrawal`, `resolve_repay` and `resolve_net_settle` never burn
  or pay more than the position holds.

**Rate curve and indexes** (`RatesAeneas`,
[details](docs/rates-half-up-curve-index.md))

* The borrow rate never decreases as utilization rises, and stays between the
  base rate and the maximum rate for every model that
  `InterestRateModel::verify` accepts.
* `utilization` is in `[0, RAY]`, and the deposit rate is at most the borrow
  rate.
* The borrow index never decreases and is capped (INV-IDX-01). Supplier
  rewards plus the protocol fee equal the accrued interest (INV-IDX-05).

**Pool accounting** (`PoolAeneas`, [details](docs/pool.md))

* `revenue ≤ supplied` holds after every operation (INV-ACCT-01).
* Accrual never aborts, for any elapsed time and any market size. Interest
  stops at the value ceiling instead of overflowing (INV-IDX-06).
* Each guard passes exactly when its documented condition holds, and each
  operation books exactly the documented amounts.
* 11 of the 13 Certora pool summaries are proved from the code instead of
  assumed.

**Controller: liquidation and risk** (`ControllerAeneas`, details:
[overview](docs/controller.md), [estimate](docs/controller-estimate.md),
[seizure](docs/controller-seizure.md),
[repayment](docs/controller-repayment.md))

* Valuations and account risk totals are conservative. Collateral is rounded
  down and debt is rounded up (INV-RISK-02).
* `estimate_liquidation_amount` never aborts in the production domain. It
  never repays more than the debt and leaves no dust. It reaches the target
  health factor within 5.5·10⁻¹⁸ USD.
* A repayment plan conserves tokens: kept plus refunded equals offered.
* A seizure takes at most what is held. After the executed plan, the health
  factor is at least what it was before. It is also at least the curve
  target, up to a stated rounding allowance.

**LP oracle pricing** (`OracleAeneas`, [details](docs/oracle.md))

* `isqrt_of_product` is exact for every pair of `u128` values.
* The LP price fails closed and equals the documented formula.
* The StableSwap solver never traps. Its result `D` satisfies
  `D*−1 < D ≤ D*+5`, where `D*` is the exact root.

The fifth library, `FixedPoint`, defines the rounding specifications the
others share. Some math proofs exist once per library, because each library
has its own translation of `common`; `sync_copies.py` generates those copies.

## Trusted base

The proofs are as strong as four assumptions.

1. **Charon and Aeneas translate faithfully.** Pinned: Aeneas
   `nightly-2026.11.12-0d21a9e`, with its bundled Charon (commit `454586e`)
   and its Lean library at the same commit (`lean/lakefile.lean`).
2. **The hand-written models are right.** Each library's
   `TypesExternal.lean` and `FunsExternal.lean` describe what the tools
   cannot translate. They model the Soroban host's `I256` and `U256` as
   integers that trap as the host does. They also model a few Rust core
   functions, and `Vec` as a list.
   They model stored state, ledger time, prices and external calls as opaque
   values.
   Each area document lists its models.
3. **A panic is an abort.** The proofs do not check which error code an
   abort carries.
4. **Integer overflow aborts.** This holds while the root `Cargo.toml` keeps
   `overflow-checks = true` under `[profile.release]`.

The build checks two more things instead of trusting them.
`#assert_standard_axioms` (`lean/Audit.lean`) fails on any `sorry` or
non-standard axiom. `check_standins.py` fails if a hand-copied piece of
production code differs from the original. All other production code is
compiled verbatim through `#[path]` includes.

## Not covered

* Token transfers, events and batch drivers.
* Which error code an abort carries.
* The execution side of a liquidation: the pool calls that apply seizures and
  repayments are modelled, not proved.
* "A position is at most its market total" (INV-ACCT-10). The withdraw,
  repay and net-settlement theorems assume it.
* The flash loan before `finalize`.
* That the StableSwap loop stops before its 255-round cap. If it did not, it
  would return `InvalidPrice`, which is covered.

Each area document lists its own gaps.

## What the proofs found

None is exploitable. The area documents have the details and the examples.

| Finding | Status |
|---|---|
| The borrow rate could fall across a breakpoint of the curve | Fixed. Monotonicity is proved (`calculate_annual_borrow_rate_monotone`) |
| The deposit rate could exceed the borrow rate after bad debt | Fixed with audit finding S-03. Utilization is proved to stay in `[0, RAY]` (`utilization_spec`) |
| The Certora `withdraw_summary` is false for a zero-amount withdrawal of dust | Open, in the spec (`withdraw_zero_amount_closes_dust`) |
| Two Certora estimate rules never reach the liquidation curve | Open, in the spec (`certora_estimate_rules`) |
| The seizure budget rounds half up, not in the protocol's favour | Known; under 10⁻¹⁸ USD per rounding step |
| At the exact bonus cap, the health factor can drop by 10⁻¹⁸ after a liquidation | Known (`ha_counterexample`) |
| Unreachable checks and early exits, a refund entry of 0, edge behaviour of `rescale` outside production use | Harmless |

## Running

```sh
export AENEAS_HOME=/path/to/unpacked/aeneas-release
./run.sh                 # all four areas
./run.sh pool oracle     # selected areas: rates, pool, controller, oracle
```

`run.sh` stops at the first of these failures:

1. A hand-copied stand-in differs from production, a generated proof copy is
   out of date, or the release profile no longer traps on overflow.
2. The crates are translated again and the result differs from the committed
   `Funs.lean` or `Types.lean`. This means the production code changed. A
   change that only moves lines counts too, because the generated Lean records
   source positions. `run.sh` leaves the new translation in place. Review it,
   repair any proof and commit. `ALLOW_DRIFT=1` continues anyway.
3. A proof breaks, or a declaration depends on `sorry` or a non-standard
   axiom.

`Funs.lean` and `Types.lean` are generated: never edit them. The proof crates
include `fp_core` verbatim on purpose, and `scripts/check_fp_boundary.py`
does not scan them.

### Setup

* Aeneas: download `aeneas-linux-x86_64.tar.gz` from the
  [`nightly-2026.11.12-0d21a9e`](https://github.com/AeneasVerif/aeneas/releases/tag/nightly-2026.11.12-0d21a9e)
  release and unpack it. That directory is `AENEAS_HOME`.
* Charon's Rust toolchain: `rustup toolchain install nightly-2026-09-17 --profile minimal -c rustc-dev -c llvm-tools -c rust-src`.
* Lean: [elan](https://github.com/leanprover/elan). `lean/lean-toolchain`
  pins `v4.36.0-rc2`.

The first run downloads about 2 GB (Lean and the prebuilt Mathlib) and takes
about 10 minutes. Later runs take about a minute.
