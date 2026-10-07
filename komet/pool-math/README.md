# Pool mathematics: Komet verification scope

The production pool is **not fully formally verified**. This ledger covers all
3 files in `common/src/math/`, 24 files in `contracts/pool/src/`, and 7 files in
`common/src/rates/`. Its obligations are work to prove, not established results.

The harness imports the real `common` crate without the `certora` feature.
Specifications use independent integer formulas and explicit domains. Production
contract source is unchanged. See [toolchain setup](../README.md).

The current `157b6f3` fork includes reviewed reverse-CLZ, masked-XOR, complement,
bounded two-word packing and signed-word normalization. There are 133 distinct
passing backend cases across runs: 22 focused, 110 prior, and one prior startup
failure passing on an unchanged-input retry. The failure remains recorded.
The integrated runtime isolates Booster crash output and starts SMT queries at
5000 ms with unchanged retries and strict Unknown handling; 153 unit checks pass.
Broad arithmetic runs remain incomplete. These tooling checks establish no
financial invariant; source and validation evidence are under `signed-in-range/`,
`smt-budget/` and the earlier model archives.

The fixed-factor [borrow-index doubling proof](proofs/borrow-double/README.md)
is complete: native APR `PASSED`, both cap branches covered, no open or failing
leaves, and a genuine false cap control rejected. Its exact Wasm, graphs, extra
module, Lean helper proofs, runtime hashes and independent reviews are archived
with an executable artifact check. General borrow growth and pool endpoints
remain separate obligations.

## Current executable claims

Build one feature with `--no-default-features --features FEATURE`. The default
build contains `half-up`, `utilization`, `borrow-index`, and `controls`; use
`--all-features` to compile every claim. Isolated artifacts reduce prover setup
work without reducing the selected claim's input domain.

| Feature | Property | Domain / assertion |
|---|---|---|
| `half-up` | `test_half_up_exact` | All signed i128 triples; exact quotient inequalities on success; `None` exactly for invalid inputs or an unrepresentable quotient. |
| `half-two` | `test_half_two`, `test_half_two_wrong` | Every signed i128, with no premise: division by two rounds ties away from zero. Separate deliberately false positive-tie control. Supplements the general multiply-divide claim. |
| `signed-floor` | `test_floor_exact` | All signed inputs with nonzero divisor and representable floor. |
| `signed-ceil` | `test_ceil_exact` | All signed inputs with nonzero divisor and representable ceil. |
| `signed-saturation` | `test_saturating_floor` | All signed inputs with nonzero divisor, including either overflow direction. |
| `scaled-supply` | `test_scaled_supply` | Nonnegative amount, positive index, decimals 0–27; rescale intermediate and floor must fit i128. |
| `scaled-borrow` | `test_scaled_borrow` | Same domain; exact ceiling. |
| `utilization` | `test_utilization_bounds` | `0 <= borrowed <= supplied`, `supplied > 0`; result in `[0,RAY]`. |
| `borrow-index` | `test_borrow_index_bounds` | `RAY <= old <= 10^36`, `RAY <= factor <= 8*RAY`; result in `[old,10^36]`. |
| `borrow-double` | `test_borrow_double`, `test_borrow_double_wrong` | Every valid old borrow index, factor fixed at `2*RAY`; result exactly `min(2*old,10^36)`. Separate false uncapped control at `old=10^36/2+1`. |
| `index-exact` | `test_borrow_index_exact` | Valid old index, factor at least RAY, representable pre-cap result; exact half-up growth and cap. |
| `supply-index` | `test_supply_index_exact` | Valid supply index, nonnegative shares/rewards; exact early returns or representable old value plus rewards; capped floor growth. |
| `reward-conservation` | `test_supply_rewards` | Same index domain, representable old value plus rewards; distributed value plus actual shortfall equals rewards. |
| `interest-split` | `test_interest_split` | Valid monotone borrow indexes, representable debt values, reserve BPS 0–9999; exact fee and supplier remainder. |
| `fee-shares` | `test_fee_shares` | Entire nonnegative i128 fee/supply domain and valid supply index; floor fee shares capped by supply headroom. |
| `flash-fee` | `test_flash_fee` | Nonnegative amount, every u32 rate, representable pre-minimum fee; exact half-up fee with a minimum of one when the rate is positive. Arithmetic only. |
| `repay` | `test_repay` | Nonnegative request/position, positive index, decimals 0–27, representable production stages; exact partial floor burn or full close, exact refund and burn bound. |
| `withdrawal` | `test_withdrawal` | Same domain; sequential half-up full-close threshold, floor full payout, partial ceiling burn and payout/burn bounds. |
| `net-settle` | `test_net_settle` | Nonnegative request/positions, positive indexes, decimals 0–27, representable production stages; exact overlap, full/partial side decisions and both burn bounds. |
| `controls` | Five earlier witnesses / false assertions | Zero supply, concrete full utilization and its false inverse, a false general utilization bound, concrete widened ceiling. |

Only half-up currently specifies its entire rejection domain. Half-two has no
excluded inputs. Other claims skip
explicitly excluded inputs; they do not prove zero-divisor or rejected-overflow
behavior. Skipped inputs return true as an implication premise. Inspect proof
paths and use feasible witnesses before accepting a nonvacuous result.

Run embedded native boundary checks with:

```sh
RUSTC_WRAPPER='' cargo test --locked --manifest-path komet/pool-math/Cargo.toml \
  --all-features --lib
```

The eight-export half-up harness completed 500 concrete Komet examples in 35
seconds against the repaired integer ABI. This is concrete validation, not a
symbolic theorem. The earlier 19-claim harness compiled successfully. On the clean concrete
models, 1,502 executions across its 17 positive claims passed; the deliberately
false concrete utilization assertion failed as expected. These generated inputs
can include premise-skipping cases; native witnesses exercise valid arithmetic
and both cap branches. The fixed doubling theorem is now proved; the other
broad-domain arithmetic claims below remain open. The current harness has 26
exports: 22 positive claims and four deliberately false controls. All nine native boundary checks pass. The five
new positive claims (signed-half, flash fee and the three settlements) passed
500 additional concrete K executions; the false tie assertion failed.
Settlement specifications retain both valuation rounding stages and
admit full-close requests whose unused asset-to-Ray conversion would overflow.
Decimals 19–27 cover helper arithmetic beyond the market validation limit of 18.

A fresh symbolic run on the committed `fb74768` word-wrap semantics proved
`test_util_zero_supply`: for every signed i128 borrowed amount, zero supplied
liquidity returns zero utilization. CLI exit was zero after 185.87 seconds;
the graph has 617 rewrites and no pending, failing, stuck, vacuous, bounded or
admitted nodes. This covers the zero-denominator branch only. Its exact Wasm,
input hashes, source, log and graph are retained under
`word-wrap-revalidation/zero-supply/` in the artifact directory linked by the
toolchain guide.

The zero-mask symbolic runs of general half-up and doubled borrow index were
interrupted after unresolved masked-word bounds produced spurious trap paths.
Their checkpoints are not accepted proofs or production counterexamples.
Three independently reviewed rules now establish masked-word bounds and unwrap
in-range memory words. Seven focused backend regressions pass, and the exact
old half-up frontier now simplifies under its original constraints. The fresh
signed-half, doubled-borrow-index and general half-up runs were subsequently
interrupted after finding further impossible paths caused by opaque symbolic
signed casts, Boolean conversion and full-word masks. Their checkpoints are
preserved; none is accepted as completed. The subsequent `e7b5188` repair
normalizes those operations and prunes only explicitly UNSAT constraints.
Backend tests reproduce the exact impossible branches and retain feasible
predecessors, without narrowing the claims. Those signed-half and doubled-index
runs were interrupted after about 25 minutes to use the subsequent guarded
word-conversion repair (`2ba460c`). Their complete checkpoints and actual
interruption exits are retained under
`arithmetic-normalization/interrupted-broad-runs/`.

The word-conversion revision passed 31 focused backend checks, six existing
symbolic regressions and 49 concrete checks. Its false half-two control completed
with the expected assertion failure after 78.50 seconds. Signed-half and general
half-up runs were interrupted after about 31 minutes when remaining opaque XOR
and bit-length branches were diagnosed. Their checkpoints and actual interruption
exits are preserved under `word-concat/interrupted-broad-runs/`. Neither is a
completed proof. The subsequent XOR repair passed 41 backend checks; guarded
bit-length and masked-OR normalization then passed 53 compiled regressions
and independent review (`d23f449`). Fresh whole-program proofs are required.
The broad input domains and assertions are unchanged.

## Prover repairs and evidence

The local fork repairs I256/U256 object tags, constructors, limb argument
validation, arithmetic errors, signed-minimum remainder rejection and byte
length checks. Validation: 95 K checks and 89 exact native SDK 28 fixture calls.
These checks cover the repaired cases, not complete SDK 28 parity.

A macOS Booster crash was traced to a 512 KiB native worker stack. The launcher
raises only default worker stacks to 64 MiB. A concrete utilization proof then
closed in 8m09s with 1,882 rewrites; its false inverse failed at the returned
boolean. Resource tuning trials closed the same positive claim in 6m45s and
5m33s. These are diagnostic timings, not broad-domain arithmetic proofs.

Subsequent review flagged nine inherited `KWASM-LEMMAS` rules. Six concern
negative shifts outside K's declared defined domain; they are not counted as
established proof-soundness defects. The actual backend simplified `(X << 3) mod 3` to zero, although
`X=1` gives 2. The optional module is now excluded; base arithmetic and memory
semantics remain. Two disjoint-memory read rules were separately reviewed and
retained with explicit nonnegative address/width guards. Without them, the
solver explored a read-equals-one branch in memory known to contain zero. **Earlier arithmetic-bearing symbolic results are provisional
and must be rerun on rebuilt semantics.** Removing the module does not certify
every remaining host or Wasm rule.

The runner now rejects cached proofs whose claim, additional module, compiled
K definitions or native library changed. It checkpoints interrupted runs and
rejects pending/empty proofs instead of reporting success. Retain `inputs.json`
and full proof graphs with every result. Never resume an old proof directory
after a source, Wasm, model or compiled-library change.

```sh
RUSTC_WRAPPER='' stellar contract build --locked \
  --manifest-path komet/pool-math/Cargo.toml \
  --no-default-features --features half-up --out-dir /tmp/pool-half-up-wasm
komet prove run --always-allocate \
  --wasm /tmp/pool-half-up-wasm/pool_math_proof.wasm \
  --id test_half_up_exact --proof-dir /tmp/pool-half-up-proof
```

Stateful obligations below require the actual pool Wasm deployed as a child
contract, with an explicit external environment. Arithmetic helper proofs alone
do not establish pool transitions, token transfers, rollback or liquidation.

## Arithmetic specification

Use exact mathematical integers for specifications. Do not compute an expected
value through the same unproved helper that the implementation uses. Let
`R = 10^27`, `K = 10^(27 - decimals)`, and `I` be a positive index.

For nonnegative numerator `N`, positive denominator `D`, and output `q`:

```text
floor:    q*D <= N < (q+1)*D
ceil:     (q-1)*D < N <= q*D
half-up:  q = floor((N + floor(D/2)) / D)

supply mint / partial debt burn: q = floor(amount*K*R / I)
debt mint / partial supply burn: q = ceil(amount*K*R / I)
supply token claim:             floor(shares*I / (R*K))
full debt token obligation:      ceil(shares*I / (R*K))
```

Prove native `i128` and widened host `I256` execution against these definitions,
including the branch where the product fits but the rounding bias overflows.
Include zero, exact divisibility, half-way cases, signed extrema where
supported, unrepresentable results, and every specified error. A success-only
proof does not establish that a valid operation can complete.

| File | Required obligations |
|---|---|
| [math/fp_core.rs](../../common/src/math/fp_core.rs) | Exact half-up, signed floor/ceil and saturating multiply-divide; native/widened agreement; quotient sign and remainder correction; `I256` conversion; divisor-zero and overflow errors; `try_*` returns `None` exactly on its invalid/unrepresentable domain; all decimal-rescaling branches, including power overflow; signed division rounding. `rescale_floor` truncates signed values toward zero, unlike signed `mul_div_floor`. |
| [math/fp.rs](../../common/src/math/fp.rs) | Every Ray/Wad/Bps wrapper uses its declared scale and direction; checked addition and nonnegative subtraction; identity and directed conversion bounds; composed BPS-to-WAD rounding; `try_mul` error propagation; flash fee minimum of one for a positive fee rate. Constructors do not validate nonnegativity: callers must establish it. |
| [math/mod.rs](../../common/src/math/mod.rs) | Wiring only: normal production arithmetic modules are exported, with no substituted proof implementation. |
| [rates/curve.rs](../../common/src/rates/curve.rs) | Exact half-up utilization and zero denominator; conditional `[0,RAY]` bound when debt value does not exceed supply; three rounded curve branches, endpoints, clamping, maximum APR and annual/per-ms conversion; deposit-rate split and invalid-factor behavior. Global curve monotonicity needs validation against rounding before being adopted. |
| [rates/compound.rs](../../common/src/rates/compound.rs) | Exact eighth-order rounded polynomial; zero-time and zero-rate identities; nonnegative-rate growth; factor bounds on the validated rate/one-year domain; intermediate representability and error behavior. Do not substitute exact `exp` or assume cadence independence. |
| [rates/index.rs](../../common/src/rates/index.rs) | Borrow identity/growth/cap; supply reward growth/cap and every early return; exact borrower-interest split; distributed reward plus shortfall equals supplier reward; fee-share floor and remaining-supply-headroom cap. Supply rewards can overflow before clamping: prove the valid domain or the error. |
| [rates/scaling.rs](../../common/src/rates/scaling.rs) | All asset/share conversions and cap saturation; full/partial withdrawal and repayment, including refund bounds; net settlement overlap, full-close conditions and bounded directed burns; split/combined rounding inequalities with precise branch conditions. Withdrawal full-close threshold uses half-up display, while payout uses floor. |
| [rates/simulate.rs](../../common/src/rates/simulate.rs) | Compose `accrue_step` from proved primitives; both index changes, shortfall routing and revenue headroom; add revenue to supply before subsequent chunks; zero/backward time; chunk-loop invariant, progress and termination; agreement with actual mutating accrual. |
| [rates/value.rs](../../common/src/rates/value.rs) | Sequential half-up/floor/ceil valuation and representability; floor value <= exact rational value <= ceil value. Dependency for controller liquidation/risk, beyond cash/share pool accounting. |
| [rates/mod.rs](../../common/src/rates/mod.rs) | Wiring only: all public rate helpers use production implementations; no Certora summary substituted for index simulation. |

## Pool state and operation obligations

Start from market creation, then prove every transition preserves a **closed**
state domain. Arbitrary fixture caps such as `shares <= 100*RAY` are insufficient
for induction when the postcondition allows larger shares.

The structural state invariant is:

```text
supplied >= 0; borrowed >= 0; cash >= 0
0 <= revenue <= supplied
10^24 <= supply_index <= 10^36
RAY <= borrow_index <= 10^36
```

For every mutation, establish exact deltas, returned positions/indexes/amounts,
unchanged fields, untouched market keys and rollback on failure. Arithmetic
proofs must distinguish representable success from correctly rejected overflow.
For all reachable-state claims, validated parameters and controller-provided
positions must be proved or explicitly assumed.

Paths below are relative to `contracts/pool/src/`.

| File | Required obligations |
|---|---|
| [lib.rs](../../contracts/pool/src/lib.rs) | Entrypoint-to-operation wiring; owner gates; returned batch ordering; constructors and upgrade boundary; projected bulk-index agreement. Auth correctness needs an effective host authorization model. |
| [cache/mod.rs](../../contracts/pool/src/cache/mod.rs) | Load/commit round trip for every field and market identity; no lost updates; elapsed-time saturation; mark timestamp only after completed accrual; valid initialized state. Test-only setters are excluded from production claims. |
| [cache/cash.rs](../../contracts/pool/src/cache/cash.rs) | Exact cash credit/debit; negative/overflow/insufficient-reserve rejection; zero transfer; correct token, amount and recipient; token movement remains distinct from accounting cash. |
| [cache/shares.rs](../../contracts/pool/src/cache/shares.rs) | Exact supply/debt mint/burn; revenue subset preservation; revenue mint/burn changes supply equally; seized-supply reclassification; cash-limited revenue payout and proportional ceiling burn; zero payout and no overburn. |
| [cache/scale.rs](../../contracts/pool/src/cache/scale.rs) | Every wrapper forwards the correct market decimals and borrow/supply index; utilization uses both indexed totals; full/partial settlement outputs agree with separately proved scaling functions. |
| [cache/report.rs](../../contracts/pool/src/cache/report.rs) | Returned DTOs and snapshots reflect the exact post-operation state, correct market/time/decimals, gross amounts and net strategy receipts; setters receive validated index results. |
| [guards.rs](../../contracts/pool/src/guards.rs) | Conservative utilization inequality and all skip branches; cash buffer after gross debt draw; saturating shortfall calculation; no debt without supply on guarded exits; exact errors. |
| [interest.rs](../../contracts/pool/src/interest.rs) | Mutating multi-chunk accrual matches projection; revenue shares enter subsequent supply; timestamp progress; protocol-fee minting; bad-debt cap, two-floor write-down and nonzero floor; quantify remaining loss instead of assuming full backing. |
| [time.rs](../../contracts/pool/src/time.rs) | Seconds-to-ms conversion exact when representable, `MathOverflow` otherwise; no wrapped time. |
| [storage.rs](../../contracts/pool/src/storage.rs) | Distinct hub/asset keys remain isolated; read/write serialization preserves fields; missing-market errors; model replacement preserves token identity/decimals; TTL renewal targets correct keys. Ledger expiry requires its own host model. |
| [views.rs](../../contracts/pool/src/views.rs) | Correct stored-index amounts, utilization, annual rates, floored revenue and time delta; views do not accrue or mutate financial state; TTL effects explicitly allowed. |
| [events.rs](../../contracts/pool/src/events.rs) | Payload field order/content matches committed snapshots; batch ordering, empty-batch and zero-fee behavior. Payload construction can be proved independently; emitted-log correctness needs an event model. |
| [ops/mod.rs](../../contracts/pool/src/ops/mod.rs) | Accrue before mutation; nonnegative action inputs; per-leg fresh cache; batch order and duplicate-market behavior; all-or-nothing failure across the batch. Controller position consistency is an explicit premise. |
| [ops/market.rs](../../contracts/pool/src/ops/market.rs) | Initialization establishes the invariant; duplicate-market rejection; validated parameters; accrue with old model before replacement; invalid replacement rolls back; same-ledger commits preserve the write footprint without changing values. |
| [ops/supply.rs](../../contracts/pool/src/ops/supply.rs) | Floor shares and exact user/market increments; cash credit equals provided receipt; reject preexisting shortfall and positive zero-share mint; unchanged debt/revenue after the separately accounted accrual. Measured receipt is established by the controller, not this pool helper. |
| [ops/borrow.rs](../../contracts/pool/src/ops/borrow.rs) | Ceil debt shares and exact user/market increments; exact cash debit and token payout; utilization, reserves and liquidation buffer; positive zero-share rejection. |
| [ops/repay.rs](../../contracts/pool/src/ops/repay.rs) | Partial floor burn or full debt close; burn cannot exceed position; net repayment plus refund equals receipt; exact cash/debt changes; positive repayment burns shares; refund transfer failure rolls everything back. |
| [ops/withdraw.rs](../../contracts/pool/src/ops/withdraw.rs) | Partial ceil burn versus full close at the actual threshold; floor full-close payout; bounded fees; net payout = gross - fee; revenue minted using post-burn supply headroom; exact cash/share deltas; ordinary/liquidation/empty-close guard differences. |
| [ops/net_settle.rs](../../contracts/pool/src/ops/net_settle.rs) | Overlap = min(request, floor supply, ceil debt); exact paired position/market burns; cash unchanged; positive overlap burns both; no orphan debt; directed rounding and full-close branches against an independent oracle. |
| [ops/seize.rs](../../contracts/pool/src/ops/seize.rs) | Debt seizure burns exact debt and socializes the correctly valued loss; collateral seizure raises revenue without minting total supply; bounds, unchanged cash and market isolation. Controller liquidation eligibility/quote is a separate prerequisite. |
| [ops/recapitalize.rs](../../contracts/pool/src/ops/recapitalize.rs) | Applied cash = min(receipt, shortfall); refund is exact excess; no shares minted and no index restoration; successful transfer/rollback and no-shortfall cases. |
| [ops/revenue.rs](../../contracts/pool/src/ops/revenue.rs) | Payout bounded by cash and floor revenue claim; equal supply/revenue burns; guarded final state; exact cash debit; payment to owner; zero-payout and transfer-failure cases. |
| [ops/strategy.rs](../../contracts/pool/src/ops/strategy.rs) | Gross debt mint; optional bounded fee; net payout = principal - fee; cash buffer checks gross draw; fee supply/revenue accounting; exact returned position and amount received. |
| [ops/flash.rs](../../contracts/pool/src/ops/flash.rs) | Positive principal, enablement, reserves, receiver type; exact rounded/minimum fee and checked balance equations; unchanged balance during callback; sufficient allowance; exact post-pull balance; fee credited once; unchanged principal debt; adversarial callback/transfer failure rollback. Accounting-helper proofs alone do not prove the flash-loan flow. |

## Required boundaries outside these directories

- `common/src/constants/`: bind all scale, rate, index, buffer and time constants
  into proof provenance.
- `common/src/types/pool.rs` and `common/src/validation.rs`: prove parameter
  admission establishes every assumption used by curve, scaling and flash
  proofs; raw/newtype and contract serialization round trips.
- Controller accounting: the pool trusts supplied positions and funded amounts.
  Establish `sum(user_supply) + revenue = supplied`,
  `sum(user_debt) = borrowed`, fresh positions for duplicate legs, measured token
  receipts and persistence of returned mutations before claiming end-to-end
  position correctness.
- Controller liquidation: eligibility, price/risk assumptions, bonus, repay
  sizing, seized collateral and credit-mode fee reclassification are not
  implemented entirely in the pool. Full liquidation mathematics must include
  those functions, beyond the pool's settlement and bad-debt handling.
- Token behavior and callbacks: prove against a specified token contract/model,
  including balance reports, allowance and transfer behavior. Across markets
  sharing one token, relate physical balance to the sum of accounting cash.
- Soroban semantics: validate integer ABI, `I256`, storage, contract invocation
  and rollback against SDK/protocol 28. `--always-allocate` alone leaves the
  alternate packed-small argument representation outside coverage.

## Claims that need qualification

- Supply index is monotone during successful interest distribution; bad-debt
  socialization may lower it to `10^24`.
- Utilization is not globally capped at 100%. The rate curve clamps its input;
  selected operation guards have explicit bypasses. Our utilization bound
  assumes borrowed value does not exceed supplied value.
- Exact RAY reward split does not imply exact booked-value conservation:
  fee-share floor rounding and headroom saturation can leave residual value.
- The supply-index floor can leave unbacked residual claims. Successful seizure
  does not prove the market has become fully solvent.
- Bounded indexes do not guarantee that every future position valuation fits
  `i128`, or that all elapsed-time accrual fits the native execution budget.
- Splitting elapsed time changes rounded rates/results. Do not assert arbitrary
  cadence equality or an exact exponential bound.

Native SDK 28 replay on the current production helpers confirmed these existing
specification counterexamples. They are not Komet proofs or claims about deployed
market configurations:

| Proposed claim | Counterexample |
|---|---|
| Borrow rate always increases with utilization for every admitted curve | Parameters accepted by `MarketParamsRaw::verify`: base `0`, slopes each `0.3*RAY`, mid `3` raw, optimal `RAY/2`, max rate/utilization `RAY`, reserve `1000`, decimals `7`, flash disabled. Utilization `2 -> 3` produces per-ms rate `10562921538470931 -> 9506629384623838`. |
| Full-utilization rate exactly equals the configured slope sum (below cap) | Mid `RAY/2`, optimal `RAY-3`, slopes each `RAY/5`: actual per-ms rate `23238427384636049`, expected sum conversion `19013258769247676`. |
| One raw reward raises the supply index by at most one raw unit | Supplied shares `1`, old index `RAY`, reward `1`: new index `2*RAY`. The number of shares must enter the bound. |

Choose intended curve admission/rounding behavior before adopting the first two
as proof obligations. Do not weaken a rule merely to obtain a passing verdict.

## Proof acceptance

1. Pin production source, lockfile, compiler, fork semantics, proof options and
   emitted Wasm hashes. Preserve the actual proof graph and logs.
2. Prove raw arithmetic before composing indexes and operations; use independent
   mathematical postconditions and branch-specific reachability evidence.
3. Require no pending, failing, stuck, bounded or admitted obligations for a
   completed claim, and establish a feasible initial input domain. Infeasible
   child branches require reproducible `UNSAT` evidence for their unchanged
   constraints; unknown results cannot prune them. Inspect actual failure
   causes for negative controls.
4. Compose initialization and every mutation over a preserved state domain.
   Fixed fixtures and one-step checks do not establish arbitrary sequences.
5. Validate relevant fork models independently. Current authorization/event
   no-ops cannot establish those properties; concrete tests alone do not prove
   model equivalence.
6. Count only completed claims over the pinned artifact. Unknown, timeout,
   backend crash, compile-only and fuzz-only results stay explicitly unproved.

Existing [common](../../certora/common/spec/README.md) and
[pool](../../certora/pool/spec/README.md) Certora rules supply useful claim and
fixture designs, but must be checked for domain closure, self-referential
oracles and proof status before reuse. Porting every rule mechanically would
carry their limitations into Komet.

## Implementation order

1. Establish reliable symbolic execution on the pinned SDK 28 model, including
   positive and negative controls. Resolve any relevant semantic discrepancy
   before accepting affected proofs.
2. Prove exact multiply/divide, rounding and rescaling in `fp_core`, then wrapper
   equivalence. Reuse these lemmas instead of adding many overlapping bounds.
3. Compose utilization, curve, compounding, index, fee and scaling proofs;
   establish the numeric domain admitted by actual parameter validation.
4. Prove market initialization and every pool mutation preserve that domain,
   with exact cash/share/index deltas and error rollback.
5. Connect controller positions/liquidation and token/callback behavior. Only
   then assess end-to-end operation and arbitrary-sequence claims.

The number of rules is an implementation detail. Coverage of the specified
branches, dependencies, state domain and external assumptions is the measure
that determines what a verification report may claim.
