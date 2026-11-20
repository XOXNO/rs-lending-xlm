# Aeneas + Lean proofs of the lending arithmetic

Machine-checked proofs about the production Rust in `common/`,
`contracts/pool/` and `contracts/controller/`: fixed-point math, rate curve
and indexes, pool accounting, liquidation math and LP oracle pricing.

[Charon](https://github.com/AeneasVerif/charon) and
[Aeneas](https://github.com/AeneasVerif/aeneas) translate the Rust into Lean 4;
each theorem is about that translation, checked by Lean, and holds for every
input allowed by the hypotheses in its statement.

* **Size:** five Lean libraries. To count the theorems, run from this
  directory:

  ```sh
  grep -rhE '^\s*(@\[[^]]*\]\s*)?(private |protected )?theorem ' --include='*.lean' \
    lean/FixedPoint lean/RatesAeneas lean/PoolAeneas lean/ControllerAeneas lean/OracleAeneas | wc -l
  ```

* **Hidden assumptions:** none. No `sorry`, `admit` or added axiom, and the
  build enforces it.
* **Run it:** `./run.sh` (see [Running](#running)). The `Aeneas Proofs` CI
  workflow (`.github/workflows/aeneas.yml`) runs it on every pull request and
  every push to `main` that touches the covered code.

The area documents below list every theorem with its Rust function,
hypotheses and invariant IDs. To add or repair a proof, read
[WRITING-PROOFS.md](WRITING-PROOFS.md). The controller and pool extraction
crates, models and Aeneas workarounds are in
[docs/extraction.md](docs/extraction.md).

## What is proved

A theorem `f args ⦃ r => P r ⦄` says that, under its hypotheses, the call
returns (does not panic) and its result satisfies `P`. "Never aborts" means
"returns for every input in the stated domain".

**Fixed-point math and share conversions** (`RatesAeneas`,
[details](docs/rates-floor-ceil.md))

* `mul_div_floor`, `mul_div_ceil` and `mul_div_half_up` return the exactly
  rounded result whenever the divisor is non-zero and the result fits
  `i128`. `mul_div_half_up` also requires non-negative operands and aborts
  otherwise.
* The conversions in `scaling.rs` that mint, burn or pay
  (`calculate_scaled_supply`, `calculate_scaled_supply_ceil`,
  `calculate_scaled_borrow`, `calculate_scaled_borrow_floor`,
  `unscale_supply_floor`, `unscale_borrow_ceil`) round in the direction
  [ADR-0003](../../docs/explanation/decisions.md#adr-0003) requires. At one
  index, a deposit is never claimed back for more than was paid in, and a
  debt is never repaid for less than is owed. The half-up valuations
  `scaled_to_original`, `unscale_supply` and `unscale_borrow` have no
  theorem of their own.
* `resolve_withdrawal`, `resolve_repay` and `resolve_net_settle` never burn
  or pay more than the position holds.

**Rate curve and indexes** (`RatesAeneas`,
[details](docs/rates-half-up-curve-index.md))

* The borrow rate, annual and per millisecond, never decreases as
  utilization rises, and the annual rate stays between the base rate and the
  maximum rate for every model that `InterestRateModel::verify` accepts.
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
* The Certora pool summaries are checked against the code: the seven with
  non-trivial postconditions are discharged; `withdraw` and `get_sync_data`
  only partly; `flash_loan` only for its fee arithmetic; two are empty
  ([table](docs/pool.md)).
* The withdraw and repay theorems hold without the fit hypotheses that
  stood for INV-ACCT-10 (`NoFit.lean`).

**Controller: liquidation and risk** (`ControllerAeneas`, details:
[overview](docs/controller.md), [estimate](docs/controller-estimate.md),
[seizure](docs/controller-seizure.md),
[repayment](docs/controller-repayment.md))

* Valuations and account risk totals are conservative. Risk-weighted
  collateral is rounded down, total collateral half up, and debt up
  (INV-RISK-02).
* `estimate_liquidation_amount` never aborts in its stated domain, which
  includes a total debt of at most 10¹⁸ USD that no code enforces. Its quote
  never exceeds the debt and leaves no dust on solvent accounts. On the
  curve-target branch it reaches the target health factor of its own linear
  model within 5.5·10⁻¹⁸ USD.
* A repayment plan conserves tokens: kept plus refunded equals offered, over
  the merged payment list (`normalize_repayment_plan`).
* A seizure takes at most what is held. Every plan that
  `build_liquidation_plan` returns seizes, per supply position, a positive
  share amount at most the shares held (`build_liquidation_plan_seized_le_held`).
  This holds for every input, with no domain hypothesis.
* In a model of the executed plan (credit-mode seizure, full delivery, the
  pool effects and the bonus cap taken as hypotheses), the health factor
  drops by at most a stated rounding allowance, and reaches the curve target
  up to that allowance when the liquidator repays at least the quote. The
  allowance is needed (`ha_counterexample`).

**LP oracle pricing** (`OracleAeneas`, [details](docs/oracle.md))

* `isqrt_of_product` is exact for every pair of `u128` values.
* The LP price fails closed and equals the documented formula.
* The StableSwap solver never traps. Its result `D` satisfies
  `D*−1 < D ≤ D*+5`, where `D*` is the exact root (`stableG_strict`,
  `post_brackets`).

The fifth library, `FixedPoint`, defines the shared rounding specifications.
Each library has its own translation of `common`, so some math proofs exist
once per library; `sync_copies.py` generates those copies.

## Trusted base

The proofs are as strong as five assumptions.

1. **Charon and Aeneas translate faithfully.** Pinned: Aeneas
   `nightly-2026.11.12-0d21a9e`, with its bundled Charon (commit `454586e`)
   and its Lean library at the same commit (`lean/lakefile.lean`).
2. **The hand-written models are right.** Each library's
   `TypesExternal.lean` and `FunsExternal.lean` model what the tools cannot
   translate: the host's `I256` and `U256` as integers that trap as the host
   does, a few Rust core functions, `Vec` as a list, and stored state, ledger
   time, prices and external calls as opaque values. Each area document lists
   its models; the controller and pool models are in
   [docs/extraction.md](docs/extraction.md).
3. **A panic is an abort.** The proofs do not check which error code an
   abort carries.
4. **Integer overflow aborts.** This holds while the root `Cargo.toml` keeps
   `overflow-checks = true` under `[profile.release]`.
5. **The host budget suffices.** CPU and memory metering are not modelled, so
   "never aborts" means "never aborts within the transaction budget".

The build checks two more things instead of trusting them.
`#assert_standard_axioms` (`lean/Audit.lean`), at the end of each library,
fails unless every declaration depends only on `propext`,
`Classical.choice` and `Quot.sound`, so on any `sorry`, `admit` or added
axiom. `check_standins.py` fails if a hand-copied piece of
production code differs from the original, or if the production body of an
opaque stand-in modelled in `FunsExternal.lean` changes. All other
production code is compiled verbatim through `#[path]` includes.

## Not covered

* Token transfers, events and batch drivers.
* Which error code an abort carries.
* The execution side of a liquidation: the pool calls that apply seizures and
  repayments are modelled, not proved.
* "A position is at most its market total" (INV-ACCT-10). It is proved
  nowhere. The withdraw and repay theorems have versions that do not need it
  (`NoFit.lean`); the net-settlement theorems still assume it.
* The repayment-plan and health-after theorems assume that the opaque
  `repays_every_debt_leg` returns for every list and context. The real
  function can abort on a market the context has not cached.
* The LTV refresh bound (`ltv_under_stored_threshold`, audit S-02) and the
  oracle TWAP, sanity-band and aggregation arithmetic.
* The plan builder (`build_liquidation_plan`) beyond its seizure bound: that
  it returns on the production domain, its health-factor gate, its spoke-flag
  checks and its repayment side.
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
| The seizure budget rounds half up, not in the protocol's favour | Known; a liquidation has `1 + 3n` half-up steps for `n` collateral legs, and the share and price steps scale with the budget and the price: about 5·10⁻¹³ USD for a $1M liquidation against BTC |
| At the exact bonus cap, the health factor can drop by 10⁻¹⁸ after a liquidation | Known (`ha_counterexample`) |
| Unreachable checks and early exits, a refund entry of 0, edge behaviour of `rescale` outside production use | Harmless |

## Running

```sh
export AENEAS_HOME=/path/to/unpacked/aeneas-release
./run.sh                 # all four areas
./run.sh pool oracle     # selected areas: rates, pool, controller, oracle
```

`run.sh` stops at the first failure:

1. A hand-copied stand-in differs from production, a generated proof copy is
   out of date, or the release profile no longer traps on overflow.
2. The re-translation differs from the committed `Funs.lean` or
   `Types.lean`: production code changed (moved lines count, since the
   generated Lean records source positions). `run.sh` leaves the new
   translation in place; review it, repair any proof and commit.
   `ALLOW_DRIFT=1` continues anyway.
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

The first run downloads about 2 GB (Lean, prebuilt Mathlib) and takes about
10 minutes; later runs take about a minute.
