# A075 — Fuzz/proptest coverage vs validation surface gaps

- Agent: A075
- Theme: T4 / T8 (untrustworthy input validation vs residual-search evidence)
- Severity: low (evidence / generator bias; no missing runtime gate found)
- Status: partial (success-path and accounting properties are dense; T4 *shape* negatives are almost entirely unit/harness, not randomized)
- Paths:
  - libFuzzer: `tests/fuzz/fuzz_targets/{flow_e2e,flow_strategy,aggregator,fp_math,fp_ops,rates_and_index,pool_native}.rs`; shared oracles `tests/fuzz/src/{invariants,decode}.rs`
  - Proptest: `tests/test-harness/tests/fuzz/{accounting_conservation,ops,migrate_blend,liquidation_vs_reference,strategy_router_invariants,strategy_multiply_budget,privileged_auth_rejects}.rs`
  - Harness wrappers that freeze the generator alphabet: `tests/test-harness/src/ops/supply.rs` (`try_supply` always one `(HubAssetKey, i128)`), `src/liquidation.rs` (`try_liquidate` → `SeizeMode::Transfer`), `src/strategy/actions.rs` (`try_flash_position`)
  - Validation SoT: `common/src/validation.rs`; `contracts/controller/src/risk/validation.rs`; `contracts/controller/src/payments.rs`; `positions/mod.rs` `FreezePolicy`; `strategies/flash_position.rs` `validate_refund_assets` / `validate_collaterals`; `strategies/migrate_blend.rs` `validate_migration_request`
- Defense: Randomized suites **do** search post-pool solvency (A072), pool accounting conservation, strategy HF/allowance/guard hygiene, Blend cap-too-low revert, liquidation *math* vs `BigRational`, aggregator stale/tolerance/sanity (A065 upstream), and owner-auth-before-validation (T1, not T4). Failed `try_*` ops in `flow_e2e` / `flow_strategy` assert full state rollback (`assert_state_preserved_on_failure`).
- Gap: Generators are biased toward **listed, positive, single-leg, in-band** calls. They do **not** randomly emit the T4 invalid shapes A061–A074 actually gate: empty/duplicate/oversize Vecs, negative/`i128` wrap amounts, unknown spoke/hub ids, freeze/`no_seize`/`paused` matrices, `SeizeMode::Credit` / estimate-vs-execute, over-length `refund_assets`, oversized `Bytes`, unapproved Blend (except one deterministic test), IRM/sync tampering, or panic-macro consistency. `flow_e2e` oracle jitter is **clamped inside** the sanity/tolerance band, so it cannot rediscover aggregator `failure`. A068 Long/Short Credit rejects and A070 over-length refunds remain harness-thin and **absent** from fuzz alphabets (A102 §7 pointer).
- Impact: A validation regression (e.g. dropping `require_non_empty_payments`, loosening `validate_refund_assets` length, skipping `BlendPoolNotApproved`) is **unlikely to fail** `make fuzz` / `make proptest` / PR `fuzz.yml`. Blast radius of that evidence hole is **undetected gate drift**, not a demonstrated fund-theft path: unit/harness/Certora still own the negatives (A061–A074, A108). Residual-search for G-VAL-1 (`no_seize`+supply), G-VAL-2 (uncapped Vecs), G-SLIP (PayDust), and A080 usage holes is likewise not in today’s random alphabets — A108 already names the missing `prop_*`.
- Evidence: This file’s §2–§6 inventories vs A061–A074; A102 §7 / §8; A108 §6.1 named `prop_*` (still absent); `tests/fuzz/README.md`; `tests/test-harness/tests/fuzz/README.md`; peers A015, A056, A062, A064, A068, A069, A070, A071, A085.
- Opinion: Treat fuzz/proptest as **invariant hunters on well-typed money paths**, not as a substitute for T4 negative tests. Highest-value randomized additions are **dedicated** shape properties (error-code exact, PIN vs CLOSE as in A108), not stuffing freeze flags into `flow_e2e` (that would either hide A064 G1 or flail on expected availability). Do not change aggregate-and-sum into reject-duplicates to make a fuzz target “pass.”

---

## 0. Mission and method

**Question:** For each Wave 4 validation concern (A061–A074), does randomized search (libFuzzer + proptest) actually generate the *invalid* inputs those gates reject, or only exercise the happy/economic neighborhood while unit/harness pins do the negatives?

**Method:**

1. Read `COORDINATION.md`, `SEED.md`, finding format, manifest A075, A102 §7 pointer (this file was unfiled).
2. Inventory every `fuzz_target!` and every `proptest!` / `prop_*` under `tests/fuzz/` and `tests/test-harness/tests/fuzz/`.
3. Trace how harness `try_*` builds Vecs, `SeizeMode`, refund lists, and swap `Bytes`.
4. Cross-walk A061–A074 defenses/gaps and A108’s named missing `prop_*`.
5. No production Rust. No git. No disagreement file (agrees with A102/A108 on evidence debt).

**Non-claims:** This agent does not re-rate A064 G1, A056 slippage, or A062 Vec caps. Those remain product residuals. This file rates **coverage of their search space**.

---

## 1. Verdict

**Partial.** Randomized testing is strong where it was designed to be strong (conservation, HF after success, liquidation ULPs, aggregator config, auth-before-body). It is **structurally blind** to most T4 validation negatives because:

1. **Wrapper collapse.** `try_supply` / `try_borrow` / `try_liquidate` always emit a **one-element** payment Vec of a **named listed** asset with a **non-negative f64→i128** amount and (for liquidate) **`SeizeMode::Transfer`**.
2. **Alphabet collapse.** `flow_e2e` has 10 ops; none are `set_paused` / `set_frozen` / `set_no_seize` / `deprecate_spoke` / unknown `spoke_id` / multi-leg payments / `flash_position` / `migrate_from_blend` / `Credit`.
3. **Safety clamps.** Oracle jitter in `flow_e2e` is reduced modulo tolerance and **asserted to stay inside** `min_sanity_price_wad..=max_sanity_price_wad`.
4. **Success-biased properties.** `prop_multiply_succeeds_*`, `prop_swap_collateral_conserves_position_delta`, `prop_migrate_blend_reconciles_*` *require* Ok; they cannot be the place invalid shapes live.
5. **CI time.** PR smoke is `FUZZ_TIME=30` / `fuzz-contract=60` plus default proptest cases (32 / 16 / 4 / 32 depending on module). That budget is spent on the existing alphabets, not on a second negative generator.

**Severity stays Low:** missing randomized negatives do not create theft. They mean `make fuzz` is not a regression net for T4 gates. Unit/harness density for amounts, flags, listing, Blend `#42`, flash refunds (except over-length), and position limits remains the real defense evidence.

---

## 2. Inventory of randomized artifacts (what exists)

### 2.1 libFuzzer (`tests/fuzz/`, `make fuzz` / `make fuzz-contract`)

| Target | Controller validation touched? | What it actually mutates |
|---|---|---|
| `fp_math` / `fp_ops` | no | Pure `common` fixed-point |
| `rates_and_index` | no (pool math) | IRM / index / fee split arithmetic — A073’s *pool* SoT, not controller sync *reads* |
| `pool_native` | no | Pool constructor, indexes, cash/revenue invariants |
| `aggregator` | **A065 adjacent — yes** | Arbitrary prices, ages, extra tolerance, sanity min/max, single vs dual source; asserts resolution vs `failure` |
| `flow_e2e` | **A072 + rollback; weak T4** | 5-byte ops: supply/borrow/withdraw/repay/liquidate/flash-loan/oracle-jitter/advance/claim/clean-bad-debt. 3 listed assets, 2 users. Amounts from `amount_for_value` / wallet mix / HF capacity (including *over* capacity). Bad flash receiver. Liquidation optional 50% USDC / 1.5× XLM **via `set_price`**, not via jitter. |
| `flow_strategy` | **A072 + strategy hygiene** | multiply / swap_debt / swap_collateral / rdwc / advance. Honest mock swap (`min_out` ≈ 0.97× spot). `PositionMode` ∈ {Multiply, Long, Short} for multiply. Same 3 listed assets. **No** empty `Bytes`, **no** `flash_position`, **no** migrate. |

`flow_e2e` / `flow_strategy` postconditions: on Ok, user HF floor (borrow/withdraw/strategy) + non-negative reserves; on Err, snapshot equality; always pool accounting + flash-guard clear. That is **A072 + A007/A030 hygiene**, not A061–A064 shape search.

### 2.2 Proptest (`tests/test-harness/tests/fuzz/`, `make proptest`)

| Property / test | Default cases | Validation role |
|---|---|---|
| `prop_accounting_conservation` | 32 | Sequence of `LendingOp` via `try_*`; asserts pool laws, index monotonicity, leftover ≤ 4, guard/allowance. **Swallows** contract errors. |
| `prop_migrate_blend_reconciles_same_asset` | 32 | In-range coll/supply/debt + cap buffer; **requires success**. HF ≥ 1. |
| `prop_migrate_blend_cap_too_low_reverts` | 32 | Cap 0.5× liability → mock health `#1`. Not `#42` / empty / dup. |
| `migrate_blend_rejects_empty_duplicate_unapproved_zero_cap` | deterministic | **Best T4 pin inside the fuzz crate:** empty `#16`, dup debt `#18`, zero cap `#14`, unapproved `#42`. |
| `owner_only_*` / `governance_*_reject_unauthed_before_validation` | deterministic | T1 auth-before-body; dummy `SpokeAssetArgs` with `no_seize: false` only. |
| `prop_valid_multiply_fits_default_budget` | 4 | Valid multiply CPU/mem + HF |
| `prop_multiply_succeeds_with_safe_hf_and_clean_router` | 16 | Valid multiply must Ok |
| `prop_swap_collateral_conserves_position_delta` | 16 | Valid USDC→USDT exact Δ |
| `empty_swap_payload_reverts_without_state_or_guard_leak` | deterministic | A069 emptiness (required-swap) |
| `prop_liquidation_matches_bigrational_reference` | 32 | In-LTV then crash price; **Transfer** liquidate vs reference ULPs |

`op_strategy()` weights: supply 4, repay 2, advance 2, others 1, script 2. Script legs are still **one payment per `SupplyOp`/`BorrowOp`**, never duplicate keys in one Vec, never empty Vec, never negative.

`LendingOp::FlashPosition` always: `FlashPositionMode::Success`, empty `refunds`, one USDC min collateral, ETH debt 1.0 — exercises **empty-refund allowed** (A070) and never over-length / dup / overlap / unlisted.

`LendingOp::Liquidate` always `try_liquidate` → **Transfer**, one debt leg, USDC price forced to `WAD/2`.

### 2.3 What is *not* a fuzz target

- No `prop_*` for `validate_bulk_position_limits` random slot counts.
- No `prop_*` for `FreezePolicy` matrices.
- No `prop_*` for keeper / mutator Vec length (A015/A062; A108 CLOSE names).
- No `prop_*` for `SeizeMode::Credit` vs estimate.
- No libFuzzer op for `flash_position` or `migrate_from_blend` (`flow_e2e`/`flow_strategy` omit both).
- No random `Bytes` length (A069); strategy fuzz always `mock_swap_payload_xdr` / `build_aggregator_swap`.

---

## 3. Coverage matrix — A061–A074 vs randomized search

Legend: **R** = randomized generator can hit the *invalid* (or boundary) case with non-trivial probability; **D** = deterministic test that happens to live under `tests/.../fuzz/`; **U** = unit/harness/Certora only (outside fuzz trees); **C** = clamped / success-biased so the invalid case is actively avoided; **—** = not applicable.

| ID | Gate (short) | libFuzzer | Proptest | Where negatives actually live | Randomized gap |
|---|---|---|---|---|---|
| **A061** | sign / zero / empty / overflow | C — amounts from `u8`/`u32` f64 scales; `1e-7` dust; never `i128::MIN` or empty Vec | C — `1u32..` supplies; withdraw 0 in script = MeansAll **valid** | `common/tests/validation.rs` `require_positive_amount` / `require_non_empty_payments`; `payments` unit; harness zero/negative strategy tests | **No random negative/empty/overflow payment Vecs** |
| **A062** | Vec length / duplicates | C — always 1-leg | C — 1-leg; script multi-*calls* not multi-dupes; keepers `UpdateAccountThreshold` one id | Harness aggregate-sum; flash/migrate dup reject; views 256 | **Never generates len>5 payments, 257 keepers, liquidate vs estimate 256, or payment dupes** (dupes would *succeed* via sum — do not fuzz-require reject) |
| **A063** | spoke/hub active | C — `HARNESS_SPOKE` / listed hubs only | C | Harness `spoke.rs`, deprecated liveness, integration xfails | **No unknown `spoke_id` / `hub_id` in alphabets** |
| **A064** | listing / FreezePolicy / `no_seize` | — no flag ops | — dummy `no_seize: false` in auth fixtures only | Unit `flags.rs`; harness pause/freeze/`no_seize` | **G-VAL-1 unsearchable; supply-under-`no_seize` never appears as a random op** |
| **A065** | freshness / sanity on risk paths | **R on `aggregator`**; **C on `flow_e2e` jitter** | C — liquidate uses `set_price` inside typical bands | Aggregator + `oracle/tolerance/*`; Certora freshness* | Flow fuzz **will not** rediscover plant-stale or out-of-band `prices()` failure; aggregator target **will** |
| **A066** | max supply/debt slots | C — ≤3 assets, limits 5 | C — same | Unit `controller/tests/validation.rs`; harness limit exceed / top-up / Credit / swap_collateral free-slot | **Never opens 6th key; never random `swap_debt` at cap** (A066 UX residual unfuzzed) |
| **A067** | min-borrow floor | C — bootstrap/collateral ≫ $5 default | C | `min_borrow_collateral.rs`; admin 0-disable | **No random floor raise / dust-collateral borrow**; Certora floor=0 (A067) |
| **A068** | SeizeMode exhaustive | C — Transfer only | C — Transfer only; multiply Long/Short **as strategy**, not as Credit receiver | `liquidation_seize_modes.rs` Transfer/Credit + Credit→**Multiply** reject | **Credit never in fuzz; estimate vs execute never; Long/Short Credit one-liners still missing (A068 §)** |
| **A069** | `data` / swap `Bytes` size & emptiness | C — structured XDR / receiver ToXdr; `data` not length-fuzzed | D — empty swap revert; conservation uses valid payloads | Harness empty-swap / flash adversarial; aggregator `MAX_PROGRAM_BYTES` | **No `prop` on `swap.len()` / `data.len()`; opacity = A056** |
| **A070** | `refund_assets` unique + listed + ≤ max_supply | — not in `flow_*` | C — **empty list only** in `try_flash_position_op` | Dup / overlap / unlisted / paused coll harness; **no over-length** (A070 gap 3) | **Over-length and multi-hub listing asymmetry unfuzzed and under-harnessed** |
| **A071** | Blend allowlist | — no migrate in `flow_*` | D — unapproved `#42` in `migrate_blend_rejects_*`; success props use approved mock | Harness `test_migrate_unapproved_*`; unit `process_migrate_blend_rejects_unapproved_pool`; `blend.sh` | **Random migrate never points at `Address::generate`** |
| **A072** | post-pool LTV/HF | **R** — over-capacity borrow, over-safe withdraw; Ok ⇒ HF floor | **R** — failed ops allowed; success strategy props require HF ≥ 1 | INV-RISK-01; Certora `solvency_gate_checked` | **Best covered Wave 4 item in fuzz.** Floor strand (A067) still not. |
| **A073** | IRM / sync read trust | `rates_and_index` = pool math **R**; controller sync consumers **—** | — | Pool `verify`; governance `validate_irm_*`; harness flashloanable / util cap | **No fuzz of lying `get_sync_data` or owner-direct bad decimals** (A073 accepted Sensitive residual) |
| **A074** | panic vs `assert_with_error` | — | — `try_` collapses both to `Err` | Source audit A074 | **Not a randomized property**; integrators already cannot tell macros apart on the wire |

**Summary counts (invalid-shape search):** A072 **R**; A065 **R** only on aggregator target; A071 **D**; A069 emptiness **D**; A061–A064, A066–A068, A070 over-length, A073 controller-read, A074 **not R**.

---

## 4. Generator mechanics that hide T4

### 4.1 One-leg listed payments

```27:28:tests/test-harness/src/ops/supply.rs
        let assets: Vec<(HubAssetKey, i128)> = vec![&self.env, (hub_asset(asset_addr), amount)];
        let returned_id = ctrl.supply(&addr, &account_id, &spoke, &assets);
```

`try_supply` / `try_borrow` / `try_repay` / `try_withdraw` / `try_liquidate` share this pattern (`asset_payment_vec`). Fuzz **cannot** reach `require_non_empty_payments` fail, `aggregate_payments` duplicate-sum, or `MathOverflow` on `checked_add` of two huge legs without a new generator that builds raw `Vec`s.

### 4.2 Transfer-only liquidation

```99:113:tests/test-harness/src/liquidation.rs
    pub fn try_liquidate(
        ...
    ) -> Result<(), soroban_sdk::Error> {
        self.try_liquidate_with_mode(
            ...
            SeizeMode::Transfer,
        )
```

`flow_e2e` `Op::Liquidate` and `LendingOp::Liquidate` call `try_liquidate`. Credit admission, Credit(0) mint `Normal`, Credit→Long/Short, and `get_liquidation_estimate` share-unit / execution mismatch (A068 gaps 2–3) are **outside** the random alphabet.

### 4.3 Oracle jitter is a non-adversary

```446:476:tests/fuzz/fuzz_targets/flow_e2e.rs
            let max_dev = (10_000 - lower)
                .min(tol)
                .min(100)
                .saturating_sub(2)
                .max(0);
            ...
            assert!(
                twap >= oracle.min_sanity_price_wad && twap <= oracle.max_sanity_price_wad,
                "oracle jitter escaped configured sanity band"
            );
```

This is correct for keeping `flow_e2e` from drowning in expected `prices()` traps, but it means **A065 fail-closed is not a `flow_e2e` property**. The dedicated `aggregator` target is the randomized SoT for that gate. Plant-stale-leg *liquidation DoS* (A065 residual) is still not in either alphabet (would need a dual-source / per-leg poison op).

### 4.4 Flash position refunds: empty-only

```479:491:tests/test-harness/tests/fuzz/ops.rs
    let mut mins: Vec<(controller::types::HubAssetKey, i128)> = Vec::new(&t.env);
    mins.push_back((hub_asset(t.resolve_asset("USDC")), coll_raw));
    let refunds: Vec<Address> = Vec::new(&t.env);
    let _ = t.try_flash_position(
        ...
        &mins,
        &refunds,
    );
```

`validate_refund_assets` length cap (`len() ≤ max_supply_positions`) is therefore never stressed. Empty is explicitly allowed (A070). Over-length remains A070 gap (3) **and** A075 gap.

### 4.5 Strategy `Bytes`

`flow_strategy` `build_steps` and conservation `build_aggregator_swap` always produce a **valid, small** mock program. Empty is a **unit** test in `strategy_router_invariants.rs`. Random garbage / multi-megabyte `Bytes` is A069 Low hygiene — host metering, not conservation.

### 4.6 Conservation `try_*` swallows T4

`execute_op` uses `let _ = t.try_supply(...)`. A broken empty-Vec check would not fail conservation unless someone passed empty Vecs. The suite asserts **accounting after whatever happened**, not “invalid shape ⇒ `#N`.”

---

## 5. Thin spots called out by A102 / A068 / A070 (explicit)

A102 asked this file to map:

| Pointer | Finding |
|---|---|
| A061–A074 negatives | §3 matrix: only A072 (and aggregator A065) are truly random-negative; A071/A069 empty are deterministic-in-fuzz-crate |
| A068 estimate vs execution | **Zero** randomized or fuzz-crate tests of `get_liquidation_estimate(Credit(bad_id))` vs `liquidate` revert. Harness estimate coverage exists elsewhere; Credit *execution* pins exist; **asymmetry is unfuzzed** |
| A068 Long/Short harness | Confirmed still thin at harness; **also absent from fuzz**. `flow_strategy` `pick_mode` Long/Short is *multiply admission*, not Credit receiver `== Normal` |
| A070 over-length | Confirmed: no harness `refund_assets.len() > max_supply_positions`; fuzz always `len==0` |

---

## 6. What fuzz *does* buy validation (do not underrate)

These are real T4-adjacent properties and should stay:

| Property | Why it matters for validation |
|---|---|
| Over-capacity borrow / over-safe withdraw in `flow_e2e` | Hits `require_post_pool_risk_gates` (A072) and proves revert atomicity |
| `assert_state_preserved_on_failure` | Any gate that panics mid-tx (A074 macros included) must not leak tokens/positions |
| `prop_accounting_conservation` after flash/strategy/migrate/script | Guards “validation passed but money drifted” |
| `aggregator` arbitrary In | Independent of controller Cache; INV-ORACLE fail-closed |
| `prop_migrate_blend_cap_too_low_reverts` | Neighbor of A071: money does not move when Blend health rejects |
| Deterministic `#42` / empty / dup debt in `migrate_blend.rs` | A071/A062/A061 pins colocated with properties |
| Auth-before-validation matrices | Prevents “validation ran with stolen auth” (A002/A003); not a substitute for amount checks |
| Bad flash-loan receiver in `flow_e2e` | A044/A007 more than A061; still a fail-closed callback |

---

## 7. Named coverage holes (tests only; no production code)

Aligned with A108 §6.1; **do not invent a second name** where A108 already named one. A075 adds only items A108 did not list that are Wave 4 validation-specific.

### 7.1 Already named in A108 (adopt)

| Name | After? | Closes A075 cell |
|---|---|---|
| `prop_keeper_vec_above_max_always_rejects` | CLOSE RB-07 | A062/A015 |
| `prop_payment_vec_above_max_always_rejects` | CLOSE if mutator cap ships | A062 |
| `prop_no_seize_supply_rejected_after_option_c` | CLOSE RB-05 | A064 G1 |
| `prop_swap_rejects_below_controller_min_out` | CLOSE RB-03 | A056/A069 economic, not size |

PIN `prop_no_seize_supply_still_allowed` (or harness twin A108 §4.3) is **required** before stuffing `no_seize` into `flow_e2e`, or the fuzzer will “fail” on today’s spec.

### 7.2 A075-specific (Wave 4 shape; still missing)

| Name | After? | Property |
|---|---|---|
| `prop_empty_and_negative_payment_vecs_reject` | REGRESS A061 | Random empty / `{0}` rejected-zero / negative i128 on supply/borrow/repay; exact `#14`/`#16` |
| `prop_duplicate_payment_legs_sum_not_double_pool` | REGRESS A062 | Two legs same hub → one pool apply (do **not** assert reject) |
| `prop_unknown_spoke_or_hub_rejects_on_entry` | REGRESS A063 | Random unseeded ids on supply/borrow |
| `prop_credit_seize_rejects_long_and_short_receivers` | REGRESS A068 | Same arm as Multiply; fills harness thin spot |
| `prop_liquidation_estimate_credit_invalid_id_diverges_from_execute` | PIN A068 | Estimate returns numbers; `liquidate` reverts — documents asymmetry |
| `prop_flash_refund_over_max_supply_rejects` | REGRESS A070 | `refund_assets.len() == max_supply_positions + 1` → `#16` |
| `prop_flash_refund_unlisted_and_duplicate` | REGRESS A070 | Already harness; optional random Address shuffle |
| `prop_unapproved_blend_pool_always_42` | REGRESS A071 | Random unapproved Address; not only the one unit |
| `prop_stale_or_insane_price_fails_gated_borrow` | REGRESS A065 | Drive aggregator `failure` then `try_borrow` — complement of clamped jitter |
| `prop_bytes_over_soft_cap_rejected` | CLOSE if A069 size cap ships | Else skip; do not pretend host metering is a contract code |

Optional libFuzzer: a **`flow_validation`** target whose op-0 selects *shape* (empty vec, dup, bad spoke, Credit, refund list) instead of extending `flow_e2e` (keeps conservation alphabets economical).

---

## 8. CI implication

`.github/workflows/fuzz.yml` PR smoke (`make fuzz` 30s, `make fuzz-contract` 60s, `make proptest` defaults) **cannot** compensate for alphabet holes: more cases on the same `op_strategy` still produce one-leg listed Transfer liquidations.

Nightly `PROPTEST_CASES=5000` (workflow_dispatch default) deepens **conservation and ULP**, not T4 negatives.

`make fuzz-coverage-all` will show controller `payments.rs` / `validate_refund_assets` **lightly** hit from success paths (empty refunds, single-leg aggregate). Do not read high line coverage on `require_post_pool_risk_gates` as coverage of `require_positive_amount`.

---

## 9. Cross-links

| Peer | Relation |
|---|---|
| A102 | This file **fills** the unfiled A075 row; G-VAL product gaps unchanged; re-open A102 evidence §8 “Fuzz” column |
| A108 | Named `prop_*` for highest residuals; A075 maps **Wave 4 validation** specifically and adds A061/A063/A068/A070/A071/A065 shape props |
| A061–A074 | Runtime judgments stand; this file is **evidence topology** only |
| A015 / A062 | Uncapped Vecs: fuzz will not find length DoS; CLOSE props wait on RB-07 |
| A064 | Randomized flag search without PIN would mis-fire on G-VAL-1 |
| A065 | Aggregator fuzz is the right layer; `flow_e2e` jitter is intentionally non-adversarial |
| A068 | Compiler exhaustiveness ≠ fuzz Credit/Long/Short |
| A069 / A056 | Size vs slippage; empty swap already D-tested |
| A070 | Empty refunds *are* fuzzed; over-length is not |
| A071 | `#42` is D-tested in the fuzz crate — best-in-class for Wave 4 inside `tests/.../fuzz/` |
| A072 | Do not add more happy-path HF props; they already exist |
| A073 / A074 | Not fuzz-shaped; keep unit/source |
| A085 | Usage conservation `prop_*` is T5; out of A075 primary except “same generator-bias class” |

**Disagreement:** none. A108’s “do not add a fuzz target that requires aggregate-and-sum to become reject-duplicates” is **affirmed**.

---

## 10. Opinion (action)

1. **Do not** treat green `make proptest` / `make fuzz-contract` as “validation surface covered.”
2. **Do** keep conservation + aggregator + A072 over-capacity as the randomized money/oracle net.
3. **Do** add small **error-code** properties for empty/negative Vecs, unknown spoke, refund over-length, Credit Long/Short, unapproved Blend — cheap, shrinkable, CI-serial with existing `--test fuzz`.
4. **Do not** merge FreezePolicy into `flow_e2e` until A064 PIN/CLOSE exists.
5. **Optional** `flow_validation` libFuzzer if byte-level Vec encoding is desired; not required if proptest covers the same enums.

**Fund-safety:** no new Critical/High from this coverage audit. **Residual:** Low evidence debt on T4 negatives; Medium product items (A064 G1, A056) remain owned elsewhere and remain **unfuzzed as residuals** (expected until CLOSE tests).
