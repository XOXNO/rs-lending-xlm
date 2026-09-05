# Independent rates / interest review

Revision: `99613335b410f70ff42dd99d13ff530f6adaee67`.
Scope: every function in `common/src/rates/` and `contracts/pool/src/interest.rs`; callers and guards needed to establish accounting and reachability. Read-only source review. No production, prior audit, or existing test-file edits. Git status retained only the two pre-existing untracked paths reported by the coordinator.

Result: **no newly confirmed exploitable security defect in this slice**. Exact conservation is conditional on representable values and on the explicit loss-index floor. The known large-value accrual freeze remains a real domain limit, not a proof that arbitrarily large markets remain live. The alleged floor-residual theft from fresh depositors is blocked by the current supply guard. Two comments overstate precision guarantees; quantified below.

## Coverage

All 34 graph-discovered function bodies read; cfg variants of dispatch also checked:

- `compound.rs`: `compound_interest`; one-year chunk constant and all eight Taylor terms.
- `curve.rs`: `calculate_annual_borrow_rate`, `calculate_borrow_rate`, `calculate_deposit_rate`, `utilization`.
- `index.rs`: `update_borrow_index`, `update_supply_index`, `supply_index_reward_shortfall`, `calculate_supplier_rewards`, `protocol_fee_shares`.
- `scaling.rs`: `scaled_to_original`, `calculate_scaled_cap`, `calculate_scaled_supply`, `calculate_scaled_supply_ceil`, `calculate_scaled_borrow`, `calculate_scaled_borrow_floor`, `unscale_supply`, `unscale_supply_floor`, `unscale_borrow`, `unscale_borrow_ceil`, `resolve_withdrawal`, `resolve_net_settle`, `resolve_repay`.
- `simulate.rs`: `accrue_step`, `simulate_update_indexes`, `simulate_update_indexes_dispatch` (normal and Certora cfg), `simulate_update_indexes_body`; `AccrualStep` fields and contract.
- `value.rs`: `position_value`, `position_value_floor`, `position_value_ceil`.
- Pool `interest.rs`: `global_sync`, `accrue_chunk`, `add_protocol_revenue`, `apply_bad_debt_to_supply_index`.

Supporting code inspected: pool cache `mod.rs`, `shares.rs`, `scale.rs`; pool ops supply, seize, withdraw, flash, strategy; pool guards and time; `MarketParamsRaw::verify`, `InterestRateModel::verify`, governance `validate_market_creation`, and price-key decimal validation; Ray arithmetic and Bps application; constants; relevant existing rate/interest/flow tests and numeric-bound documentation. Graph searches/call tracing preceded supporting source discovery; test graph gaps were followed with text searches.

## Units and exact accounting

Let `R=10^27`, `M=i128::MAX`, `u=10^(27-decimals)` raw ray per native token unit. The pool itself accepts decimals **0..18**: `MarketParamsRaw::verify` (`common/src/types/pool.rs:63-68`) enforces only `asset_decimals <= WAD_DECIMALS`, and decimals are unsigned. Governance market listing is stricter: `validate_market_creation` (`contracts/governance/src/validate/asset.rs:42-62`) requires token metadata equality and **3..18** decimals before calling `params.verify`. Token price-key validation separately requires 3..18 (`contracts/price-aggregator/src/validation.rs:177-180`; reference price keys require 0). Thus the 3..18 bound belongs to governance/oracle listing, not the standalone pool. Conversion `A_ray=A_native*u` is exact while representable throughout the pool domain 0..18. Let `H(z)=floor(z+1/2)` for nonnegative rational z, `F(z)=floor(z)`, `C(z)=ceil(z)`.

State: supply shares S, debt shares B, revenue shares E, supply index Is, borrow index Ib, cash K in native units. Revenue is **a subset of supply**, `0 <= E <= S`; do not add E again to total supplier liabilities. Pool half-up accounting values are `V=H(S*Is/R)` and `D=H(B*Ib/R)`. Conservative token guard values are `floor(S*Is/(R*u))` and `ceil(B*Ib/(R*u))`. Cash-ray value is `K*u`.

### Accrual

Source: `common/src/rates/simulate.rs:54`, `common/src/rates/index.rs:29-98`, `contracts/pool/src/interest.rs:41-54`.

For one supported interval:

```
U = H(D*R/V), or 0 if V=0
annual = min(piecewise_curve(min(U,R)), max_borrow_rate)
r = H(annual / MILLISECONDS_PER_YEAR)
p1 = r*delta_ms
pk = H(p(k-1)*p1/R), k=2..8
factor = R + p1 + sum(H(pk/k!), k=2..8)
Ib' = min(H(Ib*factor/R), 10^36)
interest = H(B*Ib'/R) - H(B*Ib/R)
fee = H(interest*reserve_factor/10000)
reward = interest - fee
Is' = max(Is, min(floor((V+reward)*R/S), 10^36))
      (Is unchanged when S=0, reward=0, or V=0)
distributed = H(S*Is'/R) - V
shortfall = reward - distributed
effective_fee = fee + shortfall = interest - distributed
q = min(floor(effective_fee*R/Is'), M-S)
S' = S+q; E'=E+q; B'=B; K'=K
```

For valid initial Is within the cap, `0 <= distributed <= reward`. This follows directly from flooring the index computed from integer `V+reward`, and from clamping no lower than the old index. Saturating that index division cannot over-distribute because the upper index cap only decreases the result. Thus reward/fee splitting is exact before converting the fee to shares.

Moreover `q*Is'/R <= effective_fee`. Since effective_fee is integral, the increase `H((S+q)*Is'/R)-H(S*Is'/R)` is at most effective_fee. Therefore:

```
H(S'*Is'/R)-H(S*Is/R) <= H(B*Ib'/R)-H(B*Ib/R)
```

Accrual never worsens the half-up cash+debt versus supply backing gap. Without share-headroom saturation, unallocated residual is at most `ceil(Is'/R)` raw ray per step. With `Is' <= 10^36`, that is at most `10^9` raw ray = `10^-18` token. A universal one-raw-ray bound is incorrect at large supply indexes. Saturation can leave a larger residual; it remains unclaimed surplus rather than excess claims.

`E+q` cannot overflow if E<=S, because q<=M-S. Current fee headroom covers BOTH total-supply and revenue additions. Prior fee-overflow hypotheses that omit this cap do not apply to this revision.

### External fees

`protocol_fee_shares` floors, and `add_protocol_revenue` adds the identical q to E and S at the stored supply index.

- Flash: principal return cancels principal payout; cash increases by fee, total claims increase by at most fee. Booking follows repayment/balance checks.
- Strategy: debt is minted for gross amount A; cash decreases by A-fee; fee supply shares claim at most fee.
- Liquidation withdrawal: gross W supply is burned, fee supply is minted, cash decreases W-fee. Partial burn is ceil-scaled; full burn pays floor-valued gross. Mint-before-burn may reduce available fee headroom in an extreme share-saturated state, which underbooks revenue; it does not overcredit the protocol or recipient.
- Deposit-side seize reclassifies existing supply into revenue: E increases, S unchanged, no cash movement.

### Losses

Source: `contracts/pool/src/ops/seize.rs:25-29`, `contracts/pool/src/interest.rs:75-90`.

For seized debt shares b:

```
W = ceil(b*Ib/R)
V = H(S*Is/R)
Wcap = min(W,V)
remaining = V-Wcap
f = floor(remaining*R/V)
Is' = max(floor(Is*f/R), R/1000)
B' = B-b; S'=S; E'=E; K'=K
```

V=0 skips index mutation; debt shares still burn. Away from the index floor, both floors are conservative: supply claim reduction covers W, which is no smaller than the aggregate half-up debt-value reduction caused by burning b. Revenue bears the same pro-rata loss as other supply.

The nonzero floor is a deliberate exception: residual `S*(R/1000)/R = S/1000` raw-ray claims can remain even after complete backing loss. This can be material; it is not one-raw-unit dust. Current `ops/supply.rs:27` calls `require_backed_market` BEFORE mint/credit. Guard uses floored total supply against cash + ceiled total debt (`guards.rs:49-58`). It prevents an honest new deposit entering a materially underbacked floor market. Recapitalization can cure the shortfall before new supply. The loss floor protects division and limits share inflation to 1000x; it does not imply all losses always leave a backed/open market.

### Directed conversions and closes

Source: `common/src/rates/scaling.rs:35-188`.

- Supply mint: `floor(A_ray*R/Is)`; exact claim <= transferred value.
- Borrow mint: `ceil(A_ray*R/Ib)`; exact debt >= received gross value.
- Partial withdrawal: ceil supply shares; burned exact claim >= cash paid.
- Partial repay: floor debt shares; burned exact liability <= cash paid.
- Full withdrawal: closes at half-up displayed threshold but pays floor supply value; it does not pay the rounded-up display. If a request equals floor but is below the half-up threshold, ceil partial burn still cannot exceed the position.
- Full repay: only when payment >= ceil debt, burns all shares, refunds payment-ceil debt. Ceiling boundary cannot forgive unpaid debt.
- Net settle: `min(request, floor(supply), ceil(debt))`; closes each side only when that conservative side is exhausted. Partial supply ceil and debt floor ensure debt removed cannot exceed supply surrendered. Zero/nonpositive overlap is a no-op.
- Cap: `min(floor(cap_ray*R/index), M)` after asset-domain validation. When the mathematical cap exceeds M shares, saturation admits all representable share totals, which are still below the actual mathematical cap; this is not a cap bypass.
- USD valuation: half-up variant is a reporting/intermediate value; floor/ceil variants apply their direction at multiplication, ray->wad conversion and price multiplication. With nonnegative validated price they bound the exact result. Negative values/invalid denominators are caller-contract violations, not independently reachable attacks through these helpers.

## Boundary / candidate dispositions

1. **Fee supply denominator missing minted revenue: refuted.** Both mutator and simulator call `accrue_step`. Mutator lands revenue into both books before the next chunk. Simulator updates its local S by precisely the same q (`simulate.rs:159-183`). No view/mutation drift or double-counting found.
2. **Floor-residual theft from fresh supply: refuted on current supply entry.** Existing raw-cache demonstrations deliberately bypass the entry guard. Real entry-sequence regression passed (details below). Its initial S=B, K=0 state is seeded with `edit_state`; it proves subsequent entry guards and recap recovery, not full ordinary origination->total-wipeout reachability. Ordinary draws retain a 2% cash buffer; repeated legitimate losses may eventually reach the floor, but that sequence was not independently reproduced here.
3. **Borrow-index cap multiplication overflow: refuted in validated rate/time domain.** `InterestRateModel::verify` makes all slopes nonnegative and <= max<=2R; positive ordered breakpoints exclude zero divisors. Curve summation is bounded by 8R before final cap. Maximum one-year factor is ~7.387301587. Even at index 10^36, multiplication produces ~7.3873e36 < M. At index cap debt stops growing silently; existing tests pass. This is an explicit long-horizon policy limit.
4. **Raw-value overflow before index cap: real known limit, integration rerun NOT EXECUTED.** Value `scaled*index/R` and supplier `V+reward` must fit i128. Approximate maximum is 170.141 billion whole tokens, independent of decimals. At floor index, share headroom instead limits deposits to ~170.141 million whole tokens. Accrued debt can reach the value limit long before index 10^36. Existing source reproducer `tests/test-harness/tests/controller/large_positions_and_long_horizons.rs:321` raises caps, starts one billion tokens at 98% utilization on the XLM curve, advances until `MathOverflow`, then asserts repay and withdraw also fail. Current rerun blocked by environment/compiler identity errors described below; do not cite a fresh successful integration reproduction. `docs/reference/numeric-bounds.md` already explicitly records this limit.
5. **Zero/empty markets: no new accounting failure.** B=0 creates no supplier reward/revenue, though the borrow index can grow at base rate. S=0 leaves Is unchanged and routes all new interest to revenue. S=0/B>0 is not normally reachable via ordinary withdraw or net-settle guards; helper tests exercising it are robustness checks. At/after maximum borrow index no further interest is charged. Unsupported arbitrary over-cap indexes can cause negative-delta panics; market creation and actual accrual do not produce them.
6. **Cadence: no fee-erasure finding.** Frequency influences interest because utilization is resampled and the Taylor series is finite. Both paths use the same one-year partition for a single long gap, so they agree bit-for-bit for the same snapshot/time. Tests cover per-second/per-ledger/per-day one-day windows and daily/hourly yearly windows. At very small economic values, supplier-index quantization can redirect supplier reward to protocol revenue; residue is measured in ray/share quanta, not native-token pre-rounding. No material attacker extraction shown.
7. **Precision statement, informational only.** `compound.rs:25-30` says the result is always below e^x. Half-up power/term rounding makes that universal statement false. Exact replay using rate raw 30,000,000,000 and delta=1000 yields growth factor above exp(exact exponent) by ~0.55 raw ray (5.5e-28). Annual->per-ms rounding also means 1% and 5% one-year growth can exceed exp(annual rate) by ~1.9095e-18 and ~4.5280e-18. These are negligible precision errors, not a fee-stealing or solvency issue. `contracts/pool/tests/interest.rs` around 1090 also describes <=~1 raw ray residual irrespective of book; the general bound depends on Is, as derived above. Existing tested ordinary-index regimes remain valid.

## Executed validation

- `RUSTC_WRAPPER= cargo test --offline -p common rates:: -- --test-threads=2`: **76 passed, 0 failed**, 2.06s reported test time. Includes structured grid, 120,000-case pseudorandom index conservation sweep, extreme caps, index growth, conservative repayment/net-settle boundaries, simulator zero and long-interval boundaries.
- `RUSTC_WRAPPER= cargo test --offline -p pool interest:: -- --test-threads=2`: **22 passed, 0 failed**, 10.01s. Log: `/private/tmp/astra-rates-pool-interest.log`.
- `RUSTC_WRAPPER= cargo test --offline -p pool test_floor_wipeout_blocks_supply_until_recap_then_new_deposit_is_safe -- --nocapture`: **1 passed, 0 failed**. Log: `/private/tmp/astra-rates-floor.log`. Seize/supply/recap/withdraw real contract methods; initial accounting state manually seeded.
- Independent integer/Decimal reference: `python3 /private/tmp/astra-rates-reference.py`; 105 directed-conversion rational checks pass and precision counterexamples printed to `/private/tmp/astra-rates-reference.log`. This is an independent equation replay, not an additional execution of Soroban production code.
- Attempted `RUSTC_WRAPPER= cargo test --offline -p test-harness --test controller a_whale_market_at_sustained_high_utilization_hits_the_ray_value_ceiling_before_the_index_cap -- --nocapture`: **compilation failed; zero target tests executed**. E0277 `TryFromVal` / `FromVal` failures in interfaces; diagnostic describes multiple `soroban_env_common` identities while both notes point at 27.0.1 source. Log: `/private/tmp/astra-rates-whale-controller.log`. Coordinator observed the same issue and owns isolated-target retry; it has since reported that its lifecycle target built successfully with an isolated target directory, with that run still in progress at this update. This is not a successful whale-test result. This worker did not alter source/dependencies or attempt broad repair. Two preceding target-selection probes (`smoke_test`, then `--lib`) provided no whale execution and are not counted as validation.

## Limits

This review establishes local source and host-test behavior; no deployment/network proof. No financial exploitation found from the tiny rounding discrepancies. Existing domain-limit documentation was used as an acceptance/context aid, not as a substitute for source derivation. Claims of full closed-market reachability or fresh execution of the whale integration test would exceed this worker's evidence.
