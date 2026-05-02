# Domain 3 — Interest, Index, Math (Efficiency Audit)

**Phase:** Efficiency / scalability review (post-soundness)
**Files in scope:**
- `controller/certora/spec/interest_rules.rs` (471 lines, 14 rules + 1 sanity)
- `controller/certora/spec/index_rules.rs` (144 lines, 5 rules + 1 sanity)
- `controller/certora/spec/math_rules.rs` (394 lines, 12 rules + 7 `*_sanity` companions)

**Production reference:**
- `pool/src/interest.rs` — `global_sync` (lines 22-40), `global_sync_step` (42-65),
  `apply_bad_debt_to_supply_index` (115-163), `add_protocol_revenue_ray` (87-99)
- `controller/src/oracle/mod.rs:492-508` — `update_asset_index` (summarised)
- `common/src/rates.rs` — borrow rate model (11-42), `compound_interest` (70-105),
  `simulate_update_indexes` (162-204)
- `common/src/fp.rs` (`Ray`/`Wad`/`Bps` newtypes, line ranges 24-202),
  `common/src/fp_core.rs` (`mul_div_half_up:13-20`, `mul_div_half_up_signed:35-49`,
  `rescale_half_up:56-77`, `div_by_int_half_up:81-91`, `to_i128:94-97`)
- `common/src/constants.rs:1-77`: `RAY = 10^27`, `WAD = 10^18`, `BPS = 10_000`,
  `MILLISECONDS_PER_YEAR = 31_556_926_000`,
  `SUPPLY_INDEX_FLOOR_RAW = WAD = 10^18`, `MAX_BORROW_RATE_RAY = 2 * RAY`

**Totals (efficiency):** prune=4 tighten=11 split=5 keep=8 missing=4

**Severity legend (efficiency)**
- **prune**: rule wastes prover time without distinguishing coverage value (drop or feature-gate).
- **tighten**: scope is needlessly large — `nondet` ranges, multi-region traversal, or
  `I256` operands the rule does not require. Tightening cuts wall time without losing the
  property.
- **split**: the rule conflates two regimes (e.g., region 1 vs region 3, floor vs cap, neg
  vs pos). Splitting halves each subgoal's solver pressure.
- **keep**: rule is already at a good cost/value ratio.

---

## Efficiency rubric (applied throughout)

1. **Compound-interest scope.** Each `compound_interest` call walks an 8-term Taylor
   expansion (`rates.rs:88-104`). Each term is one `Ray::mul` (= one `mul_div_half_up`
   = one I256 mul + I256 add + I256 div + `to_i128` panic branch) plus one `div_by_int`
   (= one i128 add + i128 div with checked-add panic branch). The Ray ops cascade — `x_pow8`
   is `x.mul(x).mul(x).mul(x).mul(x).mul(x).mul(x).mul(x)`, **eight stacked I256 multiplies**.
   A rule that calls `compound_interest` twice (every monotonic rule) doubles this. With
   `delta_ms` left as a free `u64`, the prover explores all branches of the early-return
   on `delta_ms == 0` plus the I256 overflow-guard `to_i128` branch on `r * d`.
2. **I256 vs i128.** Every `mul_div_half_up` or `mul_div_half_up_signed` introduces 4
   `I256::from_i128`, 1 `mul`, 1-2 `div`, 1 `add`/`sub`, 1 `to_i128` panic site. Rules
   that linearise the property to plain i128 (`mul_half_up_rounding_direction:175`,
   `div_half_up_rounding_direction:221-222`, `signed_mul_away_from_zero:322-323`) are
   2-3× cheaper per assertion than the I256-formulated variants they replaced.
3. **Slope-region traversal.** `calculate_borrow_rate` (`rates.rs:20-34`) has three
   piecewise-linear regions selected by an `if/else if/else` on `utilization < mid` and
   `< optimal`. A rule that lets `utilization` range over `[0, RAY]` forces the prover
   to explore every region for every call. Rules that pin `utilization` to one region
   (e.g., `utilization < params.mid_utilization_ray`) shrink the per-rule search by ~3×.
4. **Floor/ceiling/half-up.** Three rounding modes coexist: `mul_div_half_up` (all
   half-up, away-from-zero positive), `mul_div_floor` (truncate toward zero), and
   `mul_div_half_up_signed` (away-from-zero on both signs). Bundling more than one in
   one rule asserts a property over a union of cases — each case has its own arithmetic
   normal form. Splitting by rounding direction is consistently a win.
5. **Bounded `nondet` integers.** A free `i128` is `2^128` cases. A `cvlr_assume!(0..=RAY)`
   bound keeps the prover on the protocol-realistic 10^27 range. The interest rules
   already do this in most places; the pool sync rules (`index_rules` R1-R4) do not.
6. **Pool-state rules vs primitive rules.** A primitive rule (`mul_half_up_commutative`)
   touches one operation. A composite rule (`supplier_rewards_conservation`) chains
   `mul + mul + sub + mul` (the production split). A full-flow rule (`supply_index_*_after_accrual`)
   reaches into `Controller::supply -> ... -> get_sync_data` cross-contract. Each tier
   has its own scope budget.
7. **Sanity duplicates.** A `cvlr_satisfy!` companion to a `cvlr_assert!` rule with the
   same preconditions is a tax on every CI run. Soundness is unchanged whether the
   companion exists; it is purely solver overhead.

**Complexity classes used below**

| Class           | Cost profile                                        | Scope budget                       |
|-----------------|-----------------------------------------------------|------------------------------------|
| **primitive**   | one `mul_div_*` or one `rescale_half_up` call       | full input range OK; cap at protocol scale |
| **compound**    | 2-3 `mul_div_*` calls, no Taylor expansion          | tighten to one rounding regime; bound to `<= RAY * 100` |
| **rate-curve**  | one `calculate_borrow_rate` call (≤3 region branches) | pin to one region per rule (`util < mid`, `mid <= util < optimal`, `optimal <= util`) |
| **taylor**      | one `compound_interest` (8 stacked Ray::muls)       | `delta_ms <= MS_PER_YEAR`; `rate <= MAX_BORROW_RATE_RAY/MS_PER_YEAR`; **never two calls when one suffices** |
| **full-flow**   | enters production code crossing `summarized!` site  | only meaningful if all reachable summaries are wired (today: only `update_asset_index` and helpers — pool calls are havoc) |

---

## interest_rules.rs

### `interest_rules.rs::nondet_valid_params` (helper, lines 30-70)

**Severity:** tighten
**Complexity class:** scope-amplifier (every rate-curve and taylor rule inherits its
breadth)
**Why:** Caps each rate parameter at `RAY * 10` (lines 49, 53-56), 5× the production cap
of `MAX_BORROW_RATE_RAY = 2 * RAY` (`common/src/constants.rs:42`). Each
`calculate_borrow_rate` call inside a rule becomes a piecewise function over a 5×-larger
slope space; each `compound_interest` call inherits that range through `rate * delta_ms`.
For every rule that takes `params = nondet_valid_params(&e)`, this is the dominant
contributor to the per-call SMT model size.

The soundness audit (`certora-review/03:39-99`) already flagged this for under-constraint
relative to production. The efficiency cost is the **same fix**: cutting each `<= RAY * 10`
to `<= MAX_BORROW_RATE_RAY = 2 * RAY` shrinks the parameter polytope by ~5× in each of
four dimensions (base, slope1, slope2, slope3) — a cumulative 5^4 = 625× reduction in
the worst-case SMT search. Adding the slope-monotonicity assumptions (`slope2 >= slope1`,
`slope3 >= slope2`) further constrains the axis-aligned bounding box to a tetrahedral
slice (~1/6 of the cube).

The unbounded `asset_decimals: u32` on line 40 also leaks into any rule that downstream
calls `Ray::to_asset(asset_decimals)` or `rescale_half_up(_, _, asset_decimals)`. None of
the `interest_rules` use `asset_decimals` (it is harmless dead-weight here), but it adds
a `u32` (~4B values) to the model. **Either constrain it (`<= 27`) or drop the field
entirely from the helper** since the rules do not exercise it.

**Estimated speedup if tightened:** 5× to 30× on every rule that uses `nondet_valid_params`
(13 of 14 rules). The compound rate-curve+taylor rules (Rules 4, 5, 12) are the largest
beneficiaries.

---

### `interest_rules.rs::borrow_rate_zero_utilization` (lines 79-94)

**Severity:** keep (after H2 from soundness audit)
**Complexity class:** rate-curve (region 1 only — `utilization == ZERO < mid_utilization`)
**Why:** Single `calculate_borrow_rate` call at `Ray::ZERO`, one region (1 — base+contribution
where contribution=0). The rule does not invoke regions 2/3. Already cheap.

The soundness audit recommended replacing the tautology re-implementation with a property
assertion (`rate <= cap_per_ms + 1` and `<= base_per_ms + 1`). That change is a **net
efficiency win** too: removing the `if base > max { max } else { base }` branch from the
rule body cuts one prover branch. Patch is the same as soundness item H2.

**Cost note:** the asserted equality `rate.raw() == expected` becomes a single linear
constraint after `nondet_valid_params` is tightened; with the current `<= RAY * 10`
bound, the prover may explore the cap-fires vs cap-doesn't-fire branch redundantly.

---

### `interest_rules.rs::borrow_rate_monotonic` (lines 102-117)

**Severity:** split
**Complexity class:** rate-curve (currently spans all 3 regions)
**Why:** `util_a` and `util_b` are both free over `[0, RAY]`. The prover must consider
every (region(util_a), region(util_b)) pair: 3×3 = 9 region-pair cases. Most are
trivially satisfied by piecewise monotonicity; the interesting cases are the boundary
crossings (regions 1→2, 2→3, 1→3). The current single rule re-derives all 9 cases on
every CI run.

**Recommendation:** keep one global rule (the property is a global invariant) **but**
split into three pinned rules for fast feedback:

```rust
#[rule] fn borrow_rate_monotonic_in_region1(e: Env) {
    // util_a, util_b in [0, mid)
    cvlr_assume!(util_b < params.mid_utilization_ray);
    // ... rest as before
}
#[rule] fn borrow_rate_monotonic_in_region2(e: Env) {
    cvlr_assume!(params.mid_utilization_ray <= util_a && util_b < params.optimal_utilization_ray);
}
#[rule] fn borrow_rate_monotonic_in_region3(e: Env) {
    cvlr_assume!(params.optimal_utilization_ray <= util_a);
}
```

Plus retain `borrow_rate_monotonic` as the all-regions umbrella rule (run on full
verification, skip on PR-gating CI). Each per-region rule is ~1/3 the SMT work of the
umbrella.

The cap branch `if annual_rate > max_rate` (`rates.rs:36-40`) doubles the path count
again — it fires or doesn't independently in each region. With `nondet_valid_params`
tightened to `MAX_BORROW_RATE_RAY`, base+s1+s2+s3 is bounded above by `4 * MAX_BORROW_RATE_RAY
= 8 * RAY`, well below the cap on most inputs; the cap-fires branch becomes a thin slice.

**Estimated speedup:** 3× per per-region rule vs the global rule.

---

### `interest_rules.rs::borrow_rate_capped` (lines 125-140)

**Severity:** keep (already cheap)
**Complexity class:** rate-curve (all 3 regions, but only one assertion per region path)
**Why:** Single `calculate_borrow_rate` call, two assertions (`rate.raw() <= cap + 1` and
`>= 0`). Both are linear in the rate raw value. The 3-region path explosion is unavoidable
because the property is global; trying to split would lose the cap's relevance.

**Note:** with `nondet_valid_params` tightened to `MAX_BORROW_RATE_RAY`, the cap is
exactly `2 * RAY / MS_PER_YEAR ≈ 6.34e16`, a known constant from the prover's perspective
once the assumption fires. The cap branch becomes a clean linear constraint instead of
a region-selection mux.

---

### `interest_rules.rs::borrow_rate_continuity_at_mid` (lines 148-169)
### `interest_rules.rs::borrow_rate_continuity_at_optimal` (lines 178-199)

**Severity:** tighten
**Complexity class:** rate-curve × 2 (each rule calls `calculate_borrow_rate` twice;
both regions adjacent to the boundary fire)
**Why:** Each rule calls `calculate_borrow_rate` twice — once at `boundary - 1` (one
region), once at `boundary` (the next region). Production code (`rates.rs:20-34`)
selects the region via `<` comparisons, so `boundary - 1` is in region N and `boundary`
is in region N+1. **Both calls walk a full I256 mul/div chain through `Ray::mul` and
`Ray::div`** plus the cap branch.

Two efficiency improvements:

1. **Pin the region via the helper.** With `nondet_valid_params` tightened (and the
   cap constraint pinned via `params.base_borrow_rate_ray + params.slope1_ray <
   params.max_borrow_rate_ray` as the soundness audit recommends in patch M5), the
   prover does not explore the cap branch at the boundary. Each call collapses to a
   linear formula in `(base, slope1, slope2, mid, optimal)`.
2. **Drop the `mid - 1` step**, replace with the boundary-value equality test from
   the soundness audit's M5 patch. The `mid - 1` step doubles the rate-curve work
   (two calls instead of one) for a property that is cleaner expressed as
   "rate(mid) == base + slope1 / MS_PER_YEAR within rounding". One call at the
   boundary, one linear assertion.

**Estimated speedup:** 2× by going from two rate-curve calls to one.

---

### `interest_rules.rs::deposit_rate_zero_when_no_utilization` (lines 207-223)

**Severity:** keep
**Complexity class:** primitive (one `calculate_deposit_rate` call, short-circuits at
`util == ZERO`)
**Why:** `calculate_deposit_rate` returns `Ray::ZERO` immediately at `rates.rs:52-54`.
The prover sees a single early-return path, no I256 work. As cheap as it gets.

---

### `interest_rules.rs::deposit_rate_less_than_borrow` (lines 232-252)

**Severity:** tighten + (split, per soundness H3)
**Complexity class:** compound (two `mul_div_half_up`: one inside `calculate_deposit_rate`,
one in the rule's `upper_bound`)
**Why:** Each `mul_div_half_up` is 4 I256 ops + 1 panic branch. The current rule does:
- inside `calculate_deposit_rate`: `Ray::mul + Bps::apply_to` = 2 calls
- in the rule body: `mul_div_half_up(util, br, RAY)` = 1 call

= 3 I256 cascades per rule run. The soundness H3 patch adds another `mul_div_half_up`
for the tight bound (`util * br * (BPS - rf) / BPS`), bringing it to 4. The tradeoff
is acceptable because the sound version catches a real bug (reserve-factor inversion)
that the loose form misses.

**Tightening:** the input ranges `(0..=RAY)` for `utilization` and `borrow_rate` are
already protocol-realistic. `reserve_factor_bps` is `(0..BPS) = (0..10_000)`, fine.
Inputs are well-bounded.

**Split (per soundness H3):** add `deposit_rate_zero_when_rf_at_or_above_bps` as a
**separate primitive rule** (one `calculate_deposit_rate` call, one assertion). It tests
the defense-in-depth branch at `rates.rs:59-61`. Cheap because `calculate_deposit_rate`
short-circuits to `Ray::ZERO` on the out-of-range path before any I256 arithmetic runs.

**Estimated cost:** the tight version is ~30% slower than the loose one but catches
a documented production defense the current rule misses.

---

### `interest_rules.rs::compound_interest_identity` (lines 260-268)

**Severity:** keep
**Complexity class:** primitive (early-return at `delta_ms == 0`)
**Why:** `compound_interest` returns `Ray::ONE` at `rates.rs:71-73` before any I256
math. The 8-term Taylor expansion is **never instantiated** in this rule. The rate is
free over `[0, RAY]` but does not enter the model after the early-return. Already optimal.

---

### `interest_rules.rs::compound_interest_monotonic_in_time` (lines 277-292)
### `interest_rules.rs::compound_interest_monotonic_in_rate` (lines 301-316)

**Severity:** tighten (per soundness M1) + split
**Complexity class:** taylor × 2 (each rule does **two `compound_interest` calls** —
the most expensive thing in this file)
**Why:** Each `compound_interest` call instantiates 8 stacked `Ray::mul` (each = 1
`mul_div_half_up` = ~6 I256 ops + 1 panic branch) plus 7 `div_by_int_half_up` calls
(plain i128 ops with one panic branch each). That's ~55 arithmetic ops per call, ~110
per rule. The 8-term Taylor expansion is the canonical TAC blow-up source noted in the
prompt's efficiency rubric.

The current rate cap `rate <= div_by_int_half_up(RAY, MS_PER_YEAR)` (~3.17e16, =
1 RAY/year) keeps `x = rate * delta_ms` ≤ `RAY` in the worst case. Soundness M1 raises
this to `2 * RAY/year` to match production envelope; the worst-case `x` becomes `2 RAY`
and the high-power Taylor terms grow as `(2 RAY)^8 / 40_320 ≈ 2.56e236 / 4e4 ≈ 6.4e231`.
**Still well inside I256** (max ~5.7e76 wait — actually I256 is signed 256-bit so
max ~1.7e77), so no I256 overflow… **but**: `(2 RAY)^9 = 5.12e242` is still inside I256.
The 8-term truncation is fine.

The efficiency cost of soundness M1 is real but bounded. Each Taylor term doubles in
magnitude per power; the I256 ops themselves do not get more expensive — they always
run on 256-bit operands. **The cost increase is ~0%**: the prover does the same number
of operations, just on slightly larger constants.

**Split:** the two rules (`monotonic_in_time` and `monotonic_in_rate`) are independent
properties and already separate. **Keep them separate**. Do **not** combine.

**Tightening that costs nothing:**
- The `t1 < t2` case requires the prover to consider `t1 = 0` (early-return) vs `t1 > 0`
  (Taylor path). Add `cvlr_assume!(t1 > 0);` to drop the early-return branch — the
  property still holds, but the prover skips one path.
- Same for `monotonic_in_rate`: add `cvlr_assume!(r1 > 0);` (already done at line 306).

**Estimated speedup:** ~10-15% by dropping the `t1 == 0` branch.

---

### `interest_rules.rs::compound_interest_ge_simple` (lines 331-351)

**Severity:** tighten (per soundness M1) + missing companion
**Complexity class:** taylor (one `compound_interest` call)
**Why:** Single `compound_interest` call. Cheaper than the monotonic rules (one Taylor
expansion vs two). The `simple = RAY + x` is a plain i128 ADD; the assertion
`factor.raw() >= simple - 2` is a single linear inequality.

**Same M1 fix from soundness:** raise `max_rate` from `RAY/year` to `2 * RAY/year`. Costs
nothing.

**Missing companion:** a **upper-envelope rule** (`factor <= 1 + x + x^2 + ...`) is
listed missing in soundness H6. From an efficiency standpoint, an upper envelope rule
is a second taylor call in a fresh rule, ~1× this rule's cost. **Worth the budget**;
catches the doubled-rate regression that no current rule catches.

---

### `interest_rules.rs::supplier_rewards_conservation` (lines 362-408)

**Severity:** tighten
**Complexity class:** compound × 2 (production `calculate_supplier_rewards` does 2
`Ray::mul` + 1 `Bps::apply_to`; the rule body does 2 more `mul_div_half_up` for
reconstruction). **5 I256 cascades per call.**
**Why:** This is the single most expensive non-Taylor rule in the file. Each
`calculate_supplier_rewards` call (`rates.rs:126-144`) does:
- `borrowed.mul(env, old_borrow_index)` = 1 I256 cascade
- `borrowed.mul(env, new_borrow_index)` = 1
- `Bps::apply_to(accrued)` = 1 (the production fee calculation)

The rule body adds:
- `mul_div_half_up(borrowed, old_borrow_index, RAY)` = 1 (reconstruction)
- `mul_div_half_up(borrowed, new_borrow_index, RAY)` = 1
- `mul_div_half_up(accrued_interest, reserve_factor_bps, BPS)` = 1 (expected_fee)

= 6 I256 cascades total per rule run.

**Tightening:**
1. **Drop redundant reconstruction.** The rule reconstructs `accrued_interest` via two
   `mul_div_half_up` calls when production already returns `(supplier_rewards,
   protocol_fee)`. The conservation property is `rewards + fee == accrued`, where
   `accrued = mul(borrowed, new) - mul(borrowed, old)`. If we instead define
   `accrued = supplier_rewards + protocol_fee` (the asserted invariant in reverse), the
   reconstruction collapses to identity and the rule reduces to:

   ```rust
   // Original product split: rewards + fee should equal new_debt - old_debt.
   let new_debt = Ray::from_raw(borrowed).mul(&e, Ray::from_raw(new_borrow_index)).raw();
   let old_debt = Ray::from_raw(borrowed).mul(&e, Ray::from_raw(old_borrow_index)).raw();
   let accrued = new_debt - old_debt;
   let sum = supplier_rewards.raw() + protocol_fee.raw();
   cvlr_assert!((sum - accrued).abs() <= 4);
   ```

   But `Ray::mul` *is* `mul_div_half_up`, so this saves nothing. The real saving is
   to **drop the `expected_fee` assertion as a separate rule**:

   ```rust
   #[rule] fn supplier_fee_matches_reserve_factor(e: Env) { /* just the fee_diff check */ }
   ```

   Two cheap rules (5 cascades each, but each focused on one property) verify faster
   than one heavy rule (6 cascades) because the prover can short-circuit each rule
   independently.

2. **Tighten input ranges.** `borrowed <= RAY * 1_000_000` (line 374) is 10^33, well
   beyond any realistic position. Cut to `RAY * 1_000` (= 10^30) — still 1000 RAY of
   debt, still extreme but representable as a single position. Similarly `new_borrow_index
   <= RAY * 10` (line 375) covers ~10× index growth, generous; cut to `RAY * 8` to
   match `compound_interest`'s e^2 ≈ 7.4 envelope.

3. **Tolerance bound `<= 1`** (line 398) is too tight — soundness audit recommends
   `<= 4` for four cascading half-up rounds. Either tighten with `borrowed % RAY == 0`
   or accept `<= 4`. The `<= 4` route is cheaper for the solver (fewer false negatives
   means fewer search-space dives).

**Estimated speedup:** 30-40% by splitting the two assertions into separate rules and
tightening tolerance.

---

### `interest_rules.rs::update_borrow_index_monotonic` (lines 416-428)

**Severity:** tighten (per soundness M3)
**Complexity class:** compound (one `update_borrow_index` = one `Ray::mul` = one I256
cascade; one comparison)
**Why:** Single I256 cascade in the production call, one i128 comparison in the
assertion. Already cheap.

**Soundness M3** adds `cvlr_assume!(interest_factor <= 8 * RAY)`. From an efficiency
standpoint, this **shrinks the I256 multiplication bound**: with `old_index <= ?`
unbounded today (only `>= RAY`) and `interest_factor` unbounded, the prover must
consider `old_index * interest_factor` reaching i128::MAX, potentially triggering
the `to_i128` panic branch in `mul_div_half_up`. Bounding `interest_factor <= 8 * RAY`
caps the product at `index_max * 8 * RAY`. Combined with a similar `old_index <= 8 * RAY`
bound (currently absent), the panic branch becomes unreachable and the prover can prune
it. **Net efficiency win.**

---

### `interest_rules.rs::update_supply_index_monotonic` (lines 437-457)

**Severity:** split (per soundness M4) + tighten
**Complexity class:** compound (production `update_supply_index` does 2 `Ray::mul`,
1 `Ray::div`, 1 `Ray::add` if `rewards != 0` and `supplied != 0`; otherwise short-circuits)
**Why:** The current rule covers both branches: short-circuit (rewards == 0 or
supplied == 0) and full-flow. The full-flow path is **3 I256 cascades** + addition.

The short-circuit branches are **cheaper** but coverage-incomplete: a regression that
breaks the short-circuit and produces non-zero rewards from zero rewards would still
satisfy `new >= old`.

**Split per soundness M4** into two rules:

1. `update_supply_index_idempotent_when_no_rewards` — assumes `rewards == 0`, asserts
   `new == old`. Hits only the short-circuit path (1 comparison, no I256 work). **Very
   cheap.**
2. `update_supply_index_increases_with_rewards` — assumes `rewards > 0` and `supplied
   > 0`, asserts strict `new > old`. Hits only the full-flow path (3 cascades).

The split has the same total I256 work as the union rule but each subrule is independent
in the prover, can be verified in parallel, and produces cleaner failure messages on
regression.

**Tightening:** `supplied <= RAY * 1_000_000` and `old_index <= RAY * 10` (lines 446-447)
are protocol-realistic. Already bounded.

---

### `interest_rules.rs::interest_rules_sanity` (lines 463-471)

**Severity:** prune
**Complexity class:** rate-curve (one `calculate_borrow_rate` call)
**Why:** Standard `cvlr_satisfy!(rate.raw() > 0)` reachability. Soundness audit lists
this as sound. From an efficiency view, the rule is **redundant** — every rate-curve
rule above already exercises the same call with assertions. A separate satisfy-only
rule pays solver time for a property that any one of the assertion rules implicitly
satisfies (`borrow_rate_monotonic` proves a non-empty domain by construction).

**Prune** unless the codebase has a deliberate reachability-suite convention. If kept,
move under `#[cfg(feature = "certora_sanity")]` so it does not run in PR-gating CI.

---

## index_rules.rs

### `index_rules.rs::supply_index_above_floor` (lines 25-35)
### `index_rules.rs::borrow_index_gte_ray` (lines 43-52)

**Severity:** prune (until pool summary is wired) → tighten (after wiring)
**Complexity class:** full-flow (cross-contract `get_sync_data` is pure havoc today)
**Why:** Both rules read `crate::storage::market_index::get_market_index(&e, &asset)`
(`certora.rs:105-114`), which calls `LiquidityPoolClient::new(env, &pool_addr)
.get_sync_data().state`. **No `summarized!` macro is wired to `get_sync_data`** — I
verified this with a repo-wide grep. The `get_sync_data_summary` function exists at
`controller/certora/spec/summaries/pool.rs:343-391` but is **orphaned** (no `apply_summary!`
binds it).

Effect: the cross-contract call returns havoced `PoolState` fields. The prover models
`state.supply_index_ray` and `state.borrow_index_ray` as **arbitrary i128**, including
zero or negative. The assertion `>= SUPPLY_INDEX_FLOOR_RAW` (line 34) and `>= RAY`
(line 51) are trivially falsifiable. **The rules are unsound** (already noted in
`certora-review/03:558-617`).

**Efficiency impact:** the rules are not just unsound but **expensive** — the prover
has no constraint to lean on, so it explores the full 2^128 space of `i128`. Every CI
run pays solver time on a vacuous assertion. **Prune from CI until the pool summary
is wired** (then mark as `tighten` to add input bounds on the asset address).

**Wiring fix (out of scope here, but noted):** add `crate::summarized!(...)` around
the `get_sync_data` cross-contract call site in `controller/src/cache/mod.rs:147-156`
or `controller/src/storage/certora.rs:105-114`. Once wired, the rules become primitive
(one read, one assertion).

---

### `index_rules.rs::borrow_index_monotonic_after_accrual` (lines 60-76)
### `index_rules.rs::supply_index_monotonic_after_accrual` (lines 84-97)

**Severity:** prune (until pool mutation summaries wired) → tighten (after wiring)
**Complexity class:** full-flow × 2 (two cross-contract reads + one
`Controller::supply` traversal)
**Why:** Each rule reads index `before`, calls `compat::supply_single` (which calls
`Controller::supply`), then reads index `after`. **Three cross-contract calls per rule**
(`get_sync_data` × 2, `LiquidityPool::supply` × 1) — none of which have a `summarized!`
binding today. The before/after reads are independently havoced; the prover sees no
correlation between them. The supply call mutates pool state via fully havoced
`LiquidityPoolClient::supply`, returning a havoced `PoolPositionMutation` in production
but a fresh nondet in summary mode (which is also not wired here).

The assertion `borrow_after >= borrow_before` over two unrelated nondet values
**fails for the prover at any input** — the prover picks `before > after`. The rule
either fails-vacuously or passes only because the prover picks favorably.

**Cost profile:** worst of all rules in this domain. Three full cross-contract
exploration paths in the prover, no constraint linkage. **Prune from CI immediately.**

**Wiring fix:** after `get_sync_data_summary` AND `supply_summary` are both wired (the
soundness audit C2 + a separate fix for `pool::supply`), the rule becomes a primitive
"before nondet bounded, after nondet bounded by `>= before`" check (the `pool.rs:55-60`
`nondet_market_index_monotone` helper is **already defined** for exactly this purpose
but isn't reachable until summaries are wired).

---

### `index_rules.rs::indexes_unchanged_when_no_time_elapsed` (lines 105-134)

**Severity:** keep
**Complexity class:** primitive × 3 (three short-circuit paths exercised)
**Why:** This is the **only sound rule in `index_rules.rs`** because it tests math
primitives directly without any cross-contract call. Each of the three calls
(`compound_interest(_, _, 0)`, `update_borrow_index(old, RAY)`, `update_supply_index(
_, old, ZERO)`) hits an early-return path in production:
- `compound_interest:71-73` returns `Ray::ONE` on `delta_ms == 0`.
- `update_borrow_index` is a single `Ray::mul`, but `old.mul(RAY) == old` algebraically
  via `mul_div_half_up(old, RAY, RAY) = (old*RAY + RAY/2)/RAY = old + 0`. One I256
  cascade.
- `update_supply_index:114-116` returns `old_index` on `rewards == ZERO`.

**Total: 1 I256 cascade + 2 short-circuit returns.** Cheap and high-coverage. Keep
as-is.

---

### `index_rules.rs::index_sanity` (lines 140-144)

**Severity:** prune
**Complexity class:** full-flow (cross-contract `get_sync_data` havoc)
**Why:** Same root cause as R1/R2: reads through unsummarised cross-contract path,
and the assertion is `cvlr_satisfy!(idx.supply > 0 && idx.borrow > 0)`. Trivially
satisfiable by the prover picking any positive nondet. **Provides no meaningful
reachability signal** (already flagged in soundness audit).

Drop or move under `#[cfg(feature = "certora_sanity")]`.

---

## math_rules.rs

### `math_rules.rs::mul_half_up_commutative` (lines 23-38)

**Severity:** keep
**Complexity class:** primitive × 2 (two `mul_div_half_up` calls)
**Why:** Two I256 cascades, one i128 equality check. Inputs bounded `(0..=RAY)` for
all three operands. The prover can typically dispatch this without branching because
I256 multiplication is commutative by construction; the half-up bias `+ d/2` is
symmetric in `(x, y)` — both rules collapse to the same I256 expression up to argument
order. Often resolves in seconds.

Already cost-optimal.

---

### `math_rules.rs::mul_half_up_zero` (lines 44-61)

**Severity:** keep
**Complexity class:** primitive × 2 (two short-circuit-friendly calls — `0 * y` and
`x * 0`)
**Why:** Two `mul_div_half_up` calls with one operand pinned to `0`. The I256
multiplication `0 * y = 0` simplifies before the half-up `+ d/2` term — the prover
can fold `0` constants. The `+ d/2` becomes the only non-zero summand, and `(d/2) / d
= 0` for `d >= 2`. Cheap.

Note: the comment on line 54 ("p/2 / p = 0 for any p >= 2") is correct **and** the
edge case `p == 1` is also fine (`1/2 = 0`). The rule does not need a separate `p == 1`
branch.

---

### `math_rules.rs::mul_half_up_identity` (lines 67-91)

**Severity:** keep
**Complexity class:** primitive (one I256 cascade)
**Why:** `mul_div_half_up(a, RAY, RAY) = a + 0 = a`. The prover sees the I256 product
`a * RAY`, then `+ RAY/2`, then `/ RAY`. Algebraically `(a*RAY + RAY/2) / RAY = a` for
non-negative `a`. Single cascade, single equality check.

The `*_sanity` companion (lines 84-91) is **pure duplication** — same inputs, same call,
asserts `result == a` via `cvlr_satisfy!` instead of `cvlr_assert!`. **Prune the sanity
companion** (severity: prune).

**Estimated speedup:** removing the sanity duplicate halves the rule's CI cost.

---

### `math_rules.rs::div_half_up_inverse` (lines 97-111)

**Severity:** tighten (per soundness M8)
**Complexity class:** compound × 2 (two `mul_div_half_up` calls — round-trip)
**Why:** Round-trip `mul_div_half_up(mul_div_half_up(a, b, RAY), RAY, b)`. Two I256
cascades. The current bound `b > 0 && b <= RAY * 100` (line 104) allows `b = 1`, which
makes the recovered intermediate `~RAY^2 * 100`, overflowing i128 inside the second
cascade's `to_i128` (the I256 itself is fine, but the conversion panics). **The rule
fails not from a rounding violation but from a panic branch the prover cannot prove
unreachable.**

The soundness audit M8 patch adds `b >= RAY / 1_000`, which keeps the recovered value
finite (`recovered ~ a` ≤ `RAY * 100` ≤ i128::MAX). **From an efficiency standpoint,
this prunes the panic branch entirely.** The prover stops exploring the
`MathOverflow` panic path and dispatches faster.

The `*_sanity` companion (lines 113-124) is **pure duplication** with `cvlr_satisfy!`.
Prune (`prune` severity).

**Estimated speedup:** 20-30% by eliminating the panic-branch exploration.

---

### `math_rules.rs::div_half_up_zero_numerator` (lines 130-142)

**Severity:** keep
**Complexity class:** primitive (one short-circuit-friendly call)
**Why:** `mul_div_half_up(0, RAY, b) = (0 + b/2) / b = 0` for `b > 0`. Single cascade,
the `0 * RAY` term folds; the prover sees only `(b/2) / b = 0`. Cheap.

---

### `math_rules.rs::mul_half_up_rounding_direction` (lines 161-176)

**Severity:** tighten (missing upper bound, per soundness M7)
**Complexity class:** primitive (one I256 cascade, but the assertion is **linear in
i128**)
**Why:** The reformulation to linear arithmetic (lines 148-160 comment) is excellent —
the previous version computed `floor` via I256 and timed out. The current version uses
`result * WAD >= a * b - (WAD - 1)` over plain i128 with `a, b <= 10^14` to keep `a*b`
inside i128. **This is the textbook efficiency pattern** — push the property into
linear i128 wherever possible.

**Coverage gap (per soundness M7):** missing the upper bound `result * WAD <= a * b
+ WAD`. From an efficiency standpoint, adding the upper bound is **one extra linear
inequality** — negligible cost increase, doubles the bug-catching power.

The `*_sanity` companion (lines 178-188) tests only `result >= 0` — different inputs
(`<= WAD * 100`), so not pure duplication. But still a coverage tax for marginal
gain. **Move under `#[cfg(feature = "certora_sanity")]`.**

---

### `math_rules.rs::div_half_up_rounding_direction` (lines 205-223)

**Severity:** keep
**Complexity class:** primitive (one I256 cascade, two linear assertions)
**Why:** Two-sided linear envelope `floor <= result <= floor + 1`. The reformulation
(lines 196-204 comment) is the same efficiency win as Rule 6 — linear i128 instead of
I256 floor extraction. Already cost-optimal.

The two-sided form catches both rounding-down and rounding-up regressions in one rule.
No split needed.

---

### `math_rules.rs::rescale_upscale_lossless` (lines 229-244)

**Severity:** tighten (per soundness audit's nit) → split
**Complexity class:** primitive (one `rescale_half_up` call, no I256 — pure i128
multiplication)
**Why:** Hardcoded `from = 7, to = 18`. Single i128 multiplication by `10^11`
(`fp_core.rs:62-64`). Very cheap per invocation.

**Tightening (per soundness audit):** parametrise over decimal pairs. From an
efficiency view, **parametrising adds two `u32` to the model** (4B each) — but with
`from <= 27 && to <= 27 && from <= to`, the search space is a triangle of
~28 × 28 / 2 = 392 pairs. The prover dispatches each pair as an independent linear
constraint; the cost of the parametric rule is **at most ~10× the single-pair rule**,
not 392×, because most pairs are equivalent up to scaling.

**Caution:** for `(from, to)` pairs where `to - from > 18`, the upscale factor `10^(to-from)`
is `>= 10^19`. With `x` near `WAD = 10^18`, the product `x * 10^19 > i128::MAX`. The
production code uses `checked_mul` and panics with "rescale_half_up upscale overflow"
(`fp_core.rs:62-64`). **Add `cvlr_assume!(diff <= 18);` to keep the rule on the
overflow-free branch** (the soundness patch already does this).

**Split:** the lossless property holds only on the upscale path (`to > from`). The
production function has three branches (same/up/down). **Don't try to unify** — keep
this rule for upscale, `rescale_roundtrip` for the upscale+downscale composition, and
optionally add a `rescale_downscale_half_up` for the downscale-only path.

---

### `math_rules.rs::rescale_roundtrip` (lines 259-277)

**Severity:** tighten (per soundness — only one decimal pair tested)
**Complexity class:** primitive × 2 (two `rescale_half_up` calls — no I256, pure i128)
**Why:** Same hardcoded `7 -> 18 -> 7` pair. Two i128 multiplications + one half-up
adjustment. The roundtrip `(x * 10^11 + 5e10) / 10^11` recovers `x` exactly when
`x >= 0` and the residue `5e10 < 10^11` is dropped by integer division.

**Same parametrisation tightening as Rule 8.** Cost increase is similar (10-20×) but
catches downscale-precision regressions on production-relevant decimal pairs (USDC at
6 decimals, BTC at 8, ETH at 18, RAY at 27).

**Sanity companion (lines 279-287):** pure duplication. Prune.

---

### `math_rules.rs::signed_mul_away_from_zero` (lines 307-322)

**Severity:** keep (post-P1a-fix) + add direction rule
**Complexity class:** primitive (one I256 cascade, but reformulated to linear i128
assertions)
**Why:** **The P1a fix is excellent for efficiency.** The previous version did an
I256 floor computation that timed out the solver (per the comment on lines 293-295,
the prior run reported `signed_mul_away_from_zero: solving threw an exception`). The
current symmetric envelope `a * b - RAY <= result * RAY <= a * b + RAY` is
**linear over i128** (because `a, b <= 10^14` keeps `a*b` inside i128). The I256
work happens inside `mul_div_half_up_signed` itself — the rule's **assertion** is
linear, and that's where the prover spent the time previously.

**Verification:** the input bounds `(-100_000_000_000_000..0)` for `a` and `(0,
100_000_000_000_000]` for `b` keep `a*b` in `[-10^28, 0]`, well inside i128
(`max ~ 1.7 * 10^38`). No `to_i128` panic branch reachable.

**Soundness audit (P1a, 03:802-877) called out:** the **previous** assertion
direction was wrong (`result * RAY <= a * b` falsifies on legitimate inputs like
`a = -34, b = RAY/10`). The current code (`322-323`) has the correct symmetric
envelope. The P1a fix is in.

**Additional efficiency tightening:** the soundness audit also recommends adding a
**direction rule** that exercises exact-half rounding (`a * b == -3 * RAY - RAY/2 ->
result == -4`). This is a separate **primitive** rule with concrete (non-nondet)
inputs — the prover dispatches it in milliseconds. Adding the direction rule is a
**zero-cost test of the away-from-zero semantics** that the symmetric envelope alone
cannot pin.

**Sanity companion (lines 326-336):** input range `(-(RAY * 100)..0)` and `(0..=RAY *
100]` allows `|a*b|` up to `RAY^2 * 10^4 = 10^58`. **This range is too wide for a
sanity satisfy** — the I256 cascade in `mul_div_half_up_signed` accepts it, but the
resulting `result` value is up to `100 * RAY` and the satisfy condition `result < 0`
must be findable by the prover in that range. Cheap to satisfy but irrelevant to
coverage. **Prune.**

---

### `math_rules.rs::i256_no_overflow` (lines 343-360)

**Severity:** keep
**Complexity class:** primitive (one I256 cascade with pinned bounds — the rule's
sole job is to prove the panic branch unreachable)
**Why:** The whole point of the rule is to prove that `mul_div_half_up(a, b, RAY)`
does not panic for `a, b <= 10 * RAY`. The intermediate `a * b` reaches `100 RAY^2 =
10^56`, well inside I256 (max ~5.7e76). The result `100 RAY = 10^29` fits i128 (max
~1.7e38). **The I256 work is exactly the production code's hot path** — verifying it
is the rule's purpose. Cannot be tightened without losing coverage.

The bounds `(0..=10 * RAY)` are the largest realistic protocol values (an index
product `(10 * RAY) * (10 * RAY) = 100 * RAY^2`). This is **the canonical
no-overflow proof for the entire fp_core layer**. Keep verbatim.

**Sanity companion (lines 364-374):** asserts `result > 0` for the same input range.
With `a, b > 0` (implied by `(0..=10*RAY)` minus zero), the I256 product is positive
and the result is `>= 1`. The satisfy is trivially provable. **Prune** — doesn't add
coverage beyond the assertion rule.

---

### `math_rules.rs::div_by_zero_sanity` (lines 382-394)

**Severity:** tighten or prune
**Complexity class:** primitive (one panicking call)
**Why:** Asserts that `mul_div_half_up(a, RAY, 0)` panics by reaching `cvlr_assert!(false)`.
Soundness audit (`certora-review/03:912-934`) flagged this as **prover-modeling-dependent** —
if the prover treats I256 division-by-zero as a partial function returning a fresh
nondet (a sound abstraction), the rule passes despite no runtime panic.

From an efficiency standpoint, the rule pays solver time to test the prover's panic
modeling, which is **not a property of the production code**. The relevant property
"production rejects div-by-zero" is better tested via the `to_i128` overflow path
in a separate rule (the i128 conversion panic with `MathOverflow`).

**Recommendation:** prune unless the team has independently confirmed that the
Certora prover models I256 division-by-zero as a panicking partial function. If kept,
move under `#[cfg(feature = "certora_panic_modeling")]` so it does not run in the
default verification suite.

---

### `math_rules.rs::*_sanity` (lines 84-91, 113-124, 178-188, 246-253, 279-287, 324-334,
364-374)

**Severity:** prune (all 7 sanity duplicates)
**Complexity class:** primitive × 7
**Why:** Each `*_sanity` rule duplicates the preconditions of its corresponding
assertion rule and replaces `cvlr_assert!` with `cvlr_satisfy!`. **Pure duplication
of solver work** — the assertion rule already proves the property holds for all
inputs in the range, which trivially implies the satisfy condition is reachable.

The 7 sanity rules collectively double the math file's CI cost without adding
coverage. **Move all under `#[cfg(feature = "certora_sanity")]` or delete.**

If the codebase has a deliberate "every rule has a sanity companion" convention,
document it in the spec module's docstring; otherwise prune.

**Estimated CI speedup:** ~40-50% on `math_rules.rs` alone by pruning the 7 sanity
duplicates.

---

## Summary integrity (`update_asset_index_summary`, `summaries/mod.rs:95-110`)

The summary returns a `MarketIndex` with two assumptions:
1. `supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW` (line 101). ✓ Matches
   `pool/src/interest.rs:158-162` floor.
2. `borrow_index_ray >= RAY` (line 102). ✓ Matches `update_borrow_index` monotonicity
   from initial `RAY`.

**Note on prior soundness audit:** the previous review (`certora-review/03:954-994`)
flagged a third assumption `borrow_index >= supply_index` that has since been
**removed** (lines 103-105 explicitly document why: bad-debt write-down via
`pool::seize_position` can drop supply below borrow). Good fix.

**Efficiency note:** the summary returns two independent nondets. From the prover's
view, this is two `i128` constraints. The `cvlr_assume!` calls fire at the summary
boundary; downstream rules that read the returned `MarketIndex` see a constrained
value, not a havoc. **This is the canonical efficiency pattern** — the I256 work of
the real `simulate_update_indexes` (one `compound_interest` Taylor expansion + 2
`Ray::mul` for index updates) is replaced with two i128 constraints. Estimated cost
saving per call: ~50× (Taylor expansion is the dominant term).

**Missing constraint (efficiency angle):** the summary does not bound the
`current_timestamp_ms - last_timestamp` delta. Production caps this via `global_sync`'s
`MAX_COMPOUND_DELTA_MS = MILLISECONDS_PER_YEAR` chunking (`pool/src/interest.rs:17`),
so a multi-year-idle market accrues in 1-year steps. The summary returns a single
`MarketIndex` regardless of `delta_ms` — sound, because the post-conditions hold
across any number of chunks — but **callers do not see the chunking structure**. No
rule today depends on this, but if a future rule asserts "compound interest factor
is bounded", it cannot derive the bound from the summary alone.

**Recommendation:** keep the summary as-is. The chunking is a production invariant
that the rules in `interest_rules.rs` exercise directly via `compound_interest_*`
rules — the orchestration is correctly factored out.

---

## Pool summaries — wiring gap

**Critical efficiency observation not noted in the prior review:** the `pool.rs`
summaries (`controller/certora/spec/summaries/pool.rs:76-403`) define
`supply_summary`, `borrow_summary`, `withdraw_summary`, `repay_summary`,
`update_indexes_summary`, `add_rewards_summary`, `flash_loan_*_summary`,
`create_strategy_summary`, `seize_position_summary`, `claim_revenue_summary`, and
`get_sync_data_summary` — **none of which are wired** to a `summarized!`
invocation. I verified via repo-wide grep:

```
grep -rn "supply_summary|get_sync_data_summary|..." controller/src pool/src
# returns NO matches outside the summary file itself
```

Effect on `index_rules.rs` R1-R4:
- `supply_index_above_floor` (R1) and `borrow_index_gte_ray` (R2) read indexes via
  `get_market_index -> LiquidityPoolClient::get_sync_data()`. Pure havoc.
- `borrow_index_monotonic_after_accrual` (R3) and `supply_index_monotonic_after_accrual`
  (R4) call `compat::supply_single -> Controller::supply -> ... -> LiquidityPoolClient::supply`.
  Pure havoc.

**Cost:** every CI run pays solver time on these rules, which either fail-vacuously or
pass on prover-favorable nondet picks. **Estimated waste:** 4 of the 5 non-sanity
`index_rules` are vacuous; pruning them from CI until the wiring is fixed saves the
solver from exploring 2^128 × N nondet draws.

**Wiring fix (out of scope for this efficiency audit but blocking for index_rules
soundness):** add `crate::summarized!(get_sync_data_summary, pub fn get_sync_data(...) {...})`
around the production `LiquidityPool::get_sync_data` site in `pool/src/lib.rs`. The
same pattern for `supply`, `borrow`, etc. The summary functions are already written
and waiting; only the macro invocations are missing.

---

## Missing rules (efficiency-prioritised coverage gaps)

The soundness audit lists 16 missing-rule items (`certora-review/03:1009-1031`). From
an efficiency standpoint, the cheapest high-value additions are:

| #  | Rule                                                       | Class      | Cost estimate          |
|----|------------------------------------------------------------|------------|------------------------|
| M1 | `compound_interest_upper_envelope` (catches doubled-rate)  | taylor     | ~1× existing taylor rule |
| M2 | `apply_bad_debt_to_supply_index_clamps_at_floor`           | compound   | ~3× primitive (3 cascades for the production fn) |
| M3 | `add_protocol_revenue_ray_skips_at_floor`                  | primitive  | very cheap (early-return path) |
| M4 | `borrow_index_unchanged_at_zero_utilization`               | compound   | 1 compound_interest + 1 update_borrow_index = 1 taylor |

M1, M3, M4 are zero-to-low cost additions. M2 is a real cost increase but **catches
the most security-critical regression in the entire interest module** (a regression in
the floor clamp drains supplier funds). The soundness audit lists M2 as critical
(C3); the efficiency audit agrees.

---

## Action items (efficiency-tagged)

### Immediate prune (cuts CI time, no soundness change)

- **E1.** Drop or feature-gate the 7 `*_sanity` rules in `math_rules.rs` (lines 84-91,
  113-124, 178-188, 246-253, 279-287, 326-336, 364-374). Estimated 40-50% speedup on
  `math_rules.rs`.
- **E2.** Drop or feature-gate `interest_rules_sanity` (`interest_rules.rs:463-471`)
  and `index_sanity` (`index_rules.rs:140-144`).
- **E3.** Prune `index_rules.rs` R1-R4 from CI **until** `get_sync_data_summary` and
  `supply_summary` are wired via `crate::summarized!`. Today they are unsound + slow.

### High-value tightening (better coverage at same/lower cost)

- **E4.** Tighten `nondet_valid_params` (`interest_rules.rs:30-70`) caps from `RAY * 10`
  to `MAX_BORROW_RATE_RAY = 2 * RAY`. **5-30× speedup on every rule that uses it
  (13 of 14 rules).** Same patch as soundness H1.
- **E5.** Tighten `div_half_up_inverse` lower bound on `b`
  (`math_rules.rs:104`) from `b > 0` to `b >= RAY / 1_000`. Eliminates the
  `to_i128` panic-branch exploration. **20-30% speedup on this rule.** Same patch
  as soundness M8.
- **E6.** Tighten `update_borrow_index_monotonic` upper bound on `interest_factor`
  (`interest_rules.rs:419`) to `<= 8 * RAY`. Prunes the I256 overflow panic branch.
  Same patch as soundness M3.

### Splits (parallel verification + clearer failures)

- **E7.** Split `borrow_rate_monotonic` into per-region rules + retain global rule
  (`interest_rules.rs:102-117`). Each per-region rule is ~1/3 the cost of the global
  rule and runnable in parallel.
- **E8.** Split `update_supply_index_monotonic` into idempotent-when-no-rewards and
  strict-increase-with-rewards rules (`interest_rules.rs:437-457`). Same patch as
  soundness M4.
- **E9.** Split `supplier_rewards_conservation` into a sum-conservation rule and a
  fee-correctness rule (`interest_rules.rs:362-408`). Each subrule has 5 I256 cascades
  vs the union's 6, and produces clearer failure messages on regression.

### Cheap missing rules (catch security-critical regressions)

- **E10.** Add `compound_interest_upper_envelope` (catches doubled-rate). One taylor
  call + one linear assertion. Same patch as soundness H6.
- **E11.** Add `apply_bad_debt_to_supply_index_clamps_at_floor` (catches the most
  severe regression in the module — supplier-fund drain). Same patch as soundness C3.
  Lives in the pool spec, not controller — note for tracking.
- **E12.** Add `borrow_index_unchanged_at_zero_utilization` (catches dropped
  zero-utilization short-circuit). One taylor + one update_borrow_index call = ~1×
  existing taylor rule cost.

### Wiring (unblock 4 currently-vacuous rules)

- **E13.** Wire `get_sync_data_summary` (`pool.rs:343-391`) and `supply_summary`
  (`pool.rs:76-93`) via `crate::summarized!` at their production callsites in
  `pool/src/lib.rs`. After wiring, `index_rules` R1-R4 become primitive (one
  constrained read + one assertion). **This is the highest-leverage single change
  in the entire spec layer**, but it is out of scope for the efficiency audit (it
  requires editing `pool/src/lib.rs` and the `summarized!` macro plumbing).

---

## Per-rule efficiency table (summary)

| Rule                                      | File:lines       | Class       | Severity   | Notes                          |
|-------------------------------------------|------------------|-------------|------------|--------------------------------|
| `nondet_valid_params` (helper)            | interest:30-70   | scope-amp   | tighten    | E4: 5-30× speedup              |
| `borrow_rate_zero_utilization`            | interest:79-94   | rate-curve  | keep       | already cheap                  |
| `borrow_rate_monotonic`                   | interest:102-117 | rate-curve  | split      | E7: per-region split           |
| `borrow_rate_capped`                      | interest:125-140 | rate-curve  | keep       | already cheap                  |
| `borrow_rate_continuity_at_mid`           | interest:148-169 | rate-curve  | tighten    | drop `mid - 1` step; 2× speedup |
| `borrow_rate_continuity_at_optimal`       | interest:178-199 | rate-curve  | tighten    | same                           |
| `deposit_rate_zero_when_no_utilization`   | interest:207-223 | primitive   | keep       | short-circuit, optimal         |
| `deposit_rate_less_than_borrow`           | interest:232-252 | compound    | tighten    | + add rf-overflow rule (E10)   |
| `compound_interest_identity`              | interest:260-268 | primitive   | keep       | early-return path              |
| `compound_interest_monotonic_in_time`     | interest:277-292 | taylor × 2  | tighten    | M1 cap raise; ~10% speedup     |
| `compound_interest_monotonic_in_rate`     | interest:301-316 | taylor × 2  | tighten    | same                           |
| `compound_interest_ge_simple`             | interest:331-351 | taylor      | tighten    | E10: add upper envelope        |
| `supplier_rewards_conservation`           | interest:362-408 | compound × 6 | tighten    | E9: split into 2 rules         |
| `update_borrow_index_monotonic`           | interest:416-428 | compound    | tighten    | E6: bound interest_factor      |
| `update_supply_index_monotonic`           | interest:437-457 | compound    | split      | E8: idempotent + strict        |
| `interest_rules_sanity`                   | interest:463-471 | rate-curve  | prune      | E2                             |
| `supply_index_above_floor`                | index:25-35      | full-flow   | prune→tighten | E3, blocked on E13          |
| `borrow_index_gte_ray`                    | index:43-52      | full-flow   | prune→tighten | E3, blocked on E13          |
| `borrow_index_monotonic_after_accrual`    | index:60-76      | full-flow   | prune→tighten | E3, blocked on E13          |
| `supply_index_monotonic_after_accrual`    | index:84-97      | full-flow   | prune→tighten | E3, blocked on E13          |
| `indexes_unchanged_when_no_time_elapsed`  | index:105-134    | primitive × 3 | keep       | best rule in `index_rules`    |
| `index_sanity`                            | index:140-144    | full-flow   | prune      | E2                             |
| `mul_half_up_commutative`                 | math:23-38       | primitive × 2 | keep       | already optimal                |
| `mul_half_up_zero`                        | math:44-61       | primitive × 2 | keep       | short-circuit-friendly         |
| `mul_half_up_identity`                    | math:67-91       | primitive   | keep       | + prune sanity (E1)            |
| `div_half_up_inverse`                     | math:97-111      | compound × 2 | tighten    | E5: lower bound on `b`         |
| `div_half_up_zero_numerator`              | math:130-142     | primitive   | keep       | short-circuit                  |
| `mul_half_up_rounding_direction`          | math:161-176     | primitive   | tighten    | + missing upper bound (M7)     |
| `div_half_up_rounding_direction`          | math:205-223     | primitive   | keep       | already linearised             |
| `rescale_upscale_lossless`                | math:229-244     | primitive   | tighten    | parametrise decimals           |
| `rescale_roundtrip`                       | math:259-277     | primitive × 2 | tighten    | parametrise decimals           |
| `signed_mul_away_from_zero` (post-P1a)    | math:307-322     | primitive   | keep       | P1a fix is in; no regression   |
| `i256_no_overflow`                        | math:343-360     | primitive   | keep       | canonical no-overflow proof    |
| `div_by_zero_sanity`                      | math:382-394     | primitive   | tighten/prune | depends on prover modeling  |
| `*_sanity` × 7                            | math, throughout | primitive   | prune      | E1: 40-50% file speedup        |

---

## Provenance

All line numbers are from the working tree at `/Users/mihaieremia/GitHub/rs-lending-xlm`,
branch `main`, at the time of audit. Production code references cite
`common/src/rates.rs`, `common/src/fp_core.rs`, `common/src/constants.rs`,
`pool/src/interest.rs`, `controller/src/oracle/mod.rs`, `controller/src/cache/mod.rs`,
`controller/src/storage/certora.rs`, and the spec files at the file:line ranges shown.

The wiring gap (orphaned `pool.rs` summaries) was confirmed via repo-wide grep for
`supply_summary|borrow_summary|get_sync_data_summary|update_indexes_summary|...` —
the only matches are inside the summary file itself; no `summarized!` invocation
binds them. This finding is novel relative to the prior soundness review.
