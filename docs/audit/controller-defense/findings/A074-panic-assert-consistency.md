# A074 — Panic vs `assert_with_error` consistency

- Agent: A074
- Theme: T4 (input / gate validation surface); adjacency T1 (auth / fail-closed) and T3 (measured-money InternalError equality)
- Severity: info (abort semantics identical; no fund-theft or silent-continue vector). Hygiene residuals: style mix in a few chokepoints; one untyped `.expect` in `common`; overflow error-code split already owned by A058
- Status: defended (every production controller abort on defense paths is a typed `panic_with_error` / `assert_with_error` contract error, full tx rollback). Partial only as **consistency / integrator-docs hygiene**, not as missing gates
- Paths:
  - Shared macros: `soroban_sdk::{assert_with_error, panic_with_error}` (SDK: `assert_with_error!` is `if !cond { panic_with_error!(env, err) }`)
  - Post-pool risk mix: `contracts/controller/src/risk/validation.rs` (`require_post_pool_risk_gates`, `validate_bulk_position_limits`)
  - Auth mix: `contracts/controller/src/account.rs` (`require_owner_or_delegate`, `require_account_owner`, `require_spoke_match`)
  - Liquidation mix: `positions/liquidation/{plan,math,apply,mod}.rs`
  - Flash-position mix: `strategies/flash_position.rs` (`require_flash_position_still_open`, measure vs map get)
  - Payments / token: `payments.rs`, `common/src/token.rs`, `common/src/validation.rs` (`expect_invariant`, `require_*`, `require_cap_within_asset_domain`)
  - Storage Option unwraps: `storage/{protocol,account,spoke}.rs`, `context/{oracle,spoke}.rs`, `external/{price_aggregator,position_nft}.rs`
  - IRM (controller create-market path): `common/src/types/pool.rs` `InterestRateModel::verify`
  - Strategy: `strategies/{multiply,swap,migrate_blend}.rs`
  - Usage: `spoke_usage.rs` (`apply_exit` overflow vs negative)
- Defense: Controller `src/` contains **no** bare `panic!`, `unreachable!`, `todo!`, `unimplemented!`, Rust `assert!`/`debug_assert!`, or untyped `.unwrap()` / `.expect()` on money or auth paths. Failures abort the Soroban invocation with a stable numeric contract error (`docs/reference/errors.md`). `assert_with_error!` is **not** `debug_assertions`-gated; both macros always trap. Option misses and checked-math `None` use `unwrap_or_else(|| panic_with_error!(…))` so missing config / overflow cannot become a host “index out of bounds” trap. User/policy predicates overwhelmingly use `assert_with_error!`.
- Gap: (1) **Same function, two macros** for sibling gates — post-pool LTV/HF use `assert_with_error!` while the min-borrow floor uses `if` + `panic_with_error!` (A067/A072 pointer); liquidation `HealthFactorTooHigh` empty-book vs HF compare; `FlashPositionClosed` Option vs bool asserts; IRM `SlopeNonMonotonic` / one util range vs neighbouring `assert_with_error!`. Same error codes; no skip. (2) **Convention is implicit**, not written next to the macros: `assert_with_error!` = boolean policy; `panic_with_error!` = Option unwrap, multi-clause `if`, `match` `_`, InternalError blocks. (3) **`common/src/validation.rs:55` `.expect(...)`** is an untyped Rust panic if `10^exp` overflowed — unreachable after `require_cap_within_asset_domain`’s decimals check, not a controller entrypoint. (4) **Error-code (not macro) hygiene**: `transfer_amount_measured` overflow → `#14 AmountMustBePositive` vs `balance_delta_since` → `#34 InternalError` (A058). `require_account_owner` mismatch → `#13 AccountNotInMarket` vs `require_owner_or_delegate` → `#44 NotAuthorized` (A003 / `errors.md`). Zero rejected payments use `panic_with_error!(AmountMustBePositive)` after `require_nonneg_amount`’s `assert_with_error!`. (5) `PositionMode` `_` arm panics `InvalidPositionMode` rather than an exhaustive match (Normal is the live occupant of `_`; A068-adjacent).
- Impact: Mixing macros **cannot** leave durable state: both abort; flash-guard / pool commits roll back with the invocation (A007/A030). Integrators see the **same** `Error(Contract, #N)` whether the site used `assert_with_error!` or `panic_with_error!`. Blast radius of the untyped `.expect` is a programming-invariant trap (host error, no contract code) only if `max_cap_for_decimals` is called with a decimals value that still passes `checked_sub` yet overflows `10^exp` — not possible for `asset_decimals <= RAY_DECIMALS`. No path found where one macro “soft-fails” and the other hard-fails.
- Evidence: `errors.md` opening contract (all fallible entrypoints panic with a numeric code); INV-RISK-01 / INV-AUTH-02 / INV-ORACLE-01 / INV-ACCT-03 fail-closed wording; A003, A007, A030, A058, A067, A072, A102 §7 pointer; unit/harness `try_` clients treat both as `Err(Ok(Error))`; Certora `spec_hooks::solvency_gate_checked` sits **between** HF asserts and is independent of which abort macro the floor uses.
- Opinion: Treat this as a **style convention that already exists in practice**, not a missing defense. Do not mechanically rewrite every `if { panic_with_error! }` to `assert_with_error!` — Option unwraps **cannot** be asserts, and compound `if` (floor skip when `floor == 0`, zero-leg match arms, InternalError multi-checks) are clearer as panics. Highest-value optional hygiene: (a) rewrite the **boolean** floor check as `assert_with_error!(floor == 0 || ltv >= floor, MinBorrowCollateralNotMet)` so A067/A072 share one macro in one function; (b) replace `max_cap_for_decimals` `.expect` with `panic_with_error!(MathOverflow)`; (c) document the two-macro convention in `errors.md` or CONTRIBUTING. Do **not** unify `#13` vs `#44` or `#14` vs `#34` under this ticket without an integrator-compat decision.

---

## Method

1. Read `shared/COORDINATION.md`, `SEED.md`, README format; confirmed `A074-*.md` absent; A102 lists this slot as unfiled with the A067/A072 mix hint.
2. Exhaustive `rg` of `contracts/controller/src` and `common/src` for `assert_with_error!`, `panic_with_error!`, `panic!`, `assert!`, `debug_assert!`, `.unwrap(`, `.expect(`.
3. Classified every controller abort as: user/policy predicate, Option/config miss, checked-math, InternalError invariant, or match `_`.
4. Compared sibling gates in the same function (risk, auth, liquidation plan, flash-position still-open, IRM `verify`, payment zero-leg).
5. Cross-checked `errors.md` (typed codes on the wire), A003/A007/A030/A058/A067/A072, threat-model fail-closed language.

Out of primary claim: which **code number** a gate uses (A003, A058, A061), whether a gate exists at all (A061–A073), Certora storage harness defaults (A035), flash-guard Drop (A030). Those are cited only where they interact with abort **shape**.

---

## Verdict

**Defended** for abort semantics and typed error surface. **Partial** only as maintainer/integrator consistency.

| Strand | Judgment |
|---|---|
| `assert_with_error!` vs `panic_with_error!` host effect | **Equivalent** — both `panic_with_error`; always on in WASM |
| Bare Rust panic / unwrap on controller `src/` | **Absent** |
| Config / NFT / oracle miss | **Fail closed** via `unwrap_or_else(panic_with_error)` |
| User-facing boolean gates | **Almost all `assert_with_error!`** |
| Same-chokepoint macro mix (floor vs HF, HF-too-high, flash still-open, IRM) | **Style only — same codes, same abort** |
| Untyped `.expect` in `max_cap_for_decimals` | **Latent hygiene**, not a live mutator |
| Overflow / auth **code** split | **Owned by A058 / A003**, not a macro bug |

No novel Critical/High. No silent continue, no `debug_assert!` that vanishes in release, no `Result`-returning defense that callers can ignore.

---

## 1. What the two macros actually do

Soroban SDK (conceptual expansion; not vendored in-tree):

```text
assert_with_error!(env, cond, E)  =>  if !cond { panic_with_error!(env, E); }
panic_with_error!(env, E)         =>  env.panic_with_error(E)   // trap
```

Consequences for this protocol:

| Property | `assert_with_error!` | `panic_with_error!` |
|---|---|---|
| Tx abort | Yes | Yes |
| Durable writes | Rolled back | Rolled back |
| Flash flag / pool Cache | Cleared/reverted with tx (A007, A030) | Same |
| RPC / `try_` client | `Error(Contract, #code)` | Same `#code` if `E` is the same |
| Stripped in `--release` | **No** (unlike Rust `assert!`) | No |
| Usable on `Option` | Only via `opt.is_some()` then unwrap | Natural `unwrap_or_else` |

`docs/reference/errors.md` states the product contract: fallible entrypoints fail by panicking with a numeric code; there is no message. **Which macro** produced the panic is invisible on the wire.

Rust `assert!` / `debug_assert!` / `panic!("…")` would **break** that contract (host trap, no `#N`). Controller `src/` does not use them. Tests may `unwrap`/`expect` freely; they are not WASM.

---

## 2. Observed convention (de facto, undocumented)

| Pattern | Macro | Why |
|---|---|---|
| Boolean policy / input (`amount > 0`, flags, HF ≥ 1, caps, pause) | `assert_with_error!` | Reads as the predicate the caller violated |
| `Option` / `Result` miss (storage, cache, NFT `try_owner`) | `panic_with_error!` in `unwrap_or_else` | No boolean without a dummy `is_some()` |
| Checked add/sub/mul `None` | `panic_with_error!(MathOverflow)` | Same |
| Multi-clause invariant (`InternalError` blocks in liq math) | `if { panic_with_error! }` | Avoids a 4-way `&&` assert |
| Early-return inverted auth | `if ok { return }; panic_with_error!(NotAuthorized)` | Mirrors `is_owner_or_delegate` bool helper |
| `match` remainder / unknown mode | `panic_with_error!` | Fail closed on `PositionMode::Normal` in multiply validator |
| Compound skip (`floor == 0`) | `if cond { panic_with_error! }` | Avoids encoding skip as `assert!(floor==0 \|\| …)` |

`common::validation::expect_invariant` is the named form of “Option miss → `#34 InternalError`”.

This split is **load-bearing for readability**, not for safety. A future contributor who uses `panic_with_error!` for a user gate, or `assert_with_error!` for a boolean InternalError, does not weaken the invariant.

---

## 3. Inventory — controller production `src/`

Approximate site counts (macro invocations, not unique errors): **~80 `assert_with_error!`**, **~49 `panic_with_error!`**. Panic sites cluster on storage getters and checked math; asserts cluster on flags, payments, strategy predicates, and risk.

### 3.1 User / policy predicates (`assert_with_error!` majority)

Representative defense paths (not exhaustive of every line):

| Area | File | Typical errors |
|---|---|---|
| Flash reentrancy | `risk/validation.rs` `require_not_flash_loaning` | `#400 FlashLoanOngoing` |
| Post-pool LTV / HF | same, `require_post_pool_risk_gates` | `#100`-class `InsufficientCollateral` |
| Position limits | `validate_bulk_position_limits` | `#109 PositionLimitExceeded` after MathOverflow panic on `checked_add` |
| Freeze / listing | `positions/mod.rs` `enforce_spoke_asset_flags`, `require_can_*` | paused / frozen / no_seize / not collateral / not borrowable |
| Hub / spoke | `config/spoke.rs`, `context/spoke.rs` | `#43 HubNotActive`, deprecated, `#310 SpokeMismatch` |
| Caps | `spoke_usage.rs` `enforce_spoke_cap` | spoke supply/borrow cap errors |
| Liquidation admission | `liquidation/mod.rs`, `apply.rs` | mode, receiver, `CannotCleanBadDebt`, spoke match, share identity |
| Strategies | `swap.rs`, `migrate_blend.rs`, `flash_position.rs` (most sites) | empty swap, Blend approval, receiver, min collateral, measure equality |
| Views | `views.rs` | `#16 InvalidPayments` length |
| Admin | `config/{asset,registry}.rs`, `markets.rs`, `governance.rs` | caps, floor ≥ 0, pool already deployed, wrong token |

These are the T4 “validation” surface A102 cares about. They are consistently asserts except where §4 notes.

### 3.2 Option / config miss (`panic_with_error!` majority)

| Getter / miss | Error |
|---|---|
| `get_pool` | `#30 PoolNotInitialized` |
| aggregators | `#27 AggregatorNotSet` |
| position limits | `#29 PositionLimitsNotSet` |
| NFT address | `#53 PositionNftNotSet` |
| NFT `try_owner` / account maps | `#24 AccountNotFound` / `#13 AccountNotInMarket` |
| spoke config | `#300 SpokeNotFound` |
| spoke asset (hard) | `#301 AssetNotInSpoke` (`lib.rs` view, `config/asset.rs`, Cache) |
| owner | `#32 OwnerNotSet` |
| accumulator (claim) | `#211 NoAccumulator` (A039) |
| cached price | `#216 OracleNotConfigured` (A065 / A087) |
| collateral / debt position | `CollateralPositionNotFound` / `DebtPositionNotFound` |
| `require_spoke_usage_context` after ensure | `#34 InternalError` (A083 pin already loaded) |

Fail closed: missing protocol config cannot proceed as a default (contrast Certora harness defaults — A035, out of scope).

### 3.3 Checked math

`MathOverflow` `#33` via `unwrap_or_else(panic_with_error)` in multiply USD math, protocol storage, usage exit `checked_sub`, liquidation curve/math, payment aggregate add, `common/src/math/*`. FP helpers in `common` always panic typed; they never `unwrap`.

`apply_exit` then **asserts** `next >= 0` as `#34 InternalError`. Negative usage without i128 overflow is a bookkeeping bug (under-exit), not a user amount; overflow of i128 is `#33`. Split is intentional.

---

## 4. Same-function mixes (A102’s pointer)

### 4.1 Post-pool risk — floor vs HF (A067 / A072)

```43:60:contracts/controller/src/risk/validation.rs
    assert_with_error!(
        env,
        totals.ltv_collateral >= totals.total_debt,
        CollateralError::InsufficientCollateral
    );
    spec_hooks::solvency_gate_checked(account);
    assert_with_error!(
        env,
        totals.health_factor >= Wad::ONE,
        CollateralError::InsufficientCollateral
    );
    let floor = storage::get_min_borrow_collateral_usd_wad(env);
    if floor != 0 && totals.ltv_collateral.raw() < floor {
        panic_with_error!(env, CollateralError::MinBorrowCollateralNotMet);
    }
```

Why the mix exists: the floor is **skipped** when `floor == 0` (admin disable) and when the account is debt-free (early return above). Encoding that as one assert is `floor == 0 || ltv >= floor`. Semantically identical. Certora ghost `solvency_gate_checked` is recorded **after** LTV and **before** HF/floor — independent of the floor’s macro. **No gate is weaker.**

`validate_bulk_position_limits` in the same file already mixes `checked_add` panic with an assert on `total <= max` — the correct Option-vs-bool split.

### 4.2 Liquidation — `HealthFactorTooHigh`

```25:49:contracts/controller/src/positions/liquidation/plan.rs
    if account.borrow_positions.is_empty() {
        panic_with_error!(env, CollateralError::HealthFactorTooHigh);
    }
    // ...
    assert_with_error!(
        env,
        totals.health_factor < Wad::ONE,
        CollateralError::HealthFactorTooHigh
    );
```

Empty borrow book is a fast path (skip risk totals / prices) with the **same** code as HF ≥ 1 WAD. Could be `assert_with_error!(!borrow_positions.is_empty(), HealthFactorTooHigh)`. Fail closed either way; cannot liquidate a healthy or debt-free book.

Plan construction then uses **only** `panic_with_error!(InternalError)` inside `LiquidationPlan::validate` / `split_seized_shares` for conservation identities, while `apply.rs` re-checks share identity with `assert_with_error!(…, InternalError)`. Same `#34`, two macros — InternalError is allowed both ways.

`FullCloseRequired` is a user-facing panic in a multi-condition `if` (payment vs ideal vs bonus cap). An assert would be a long boolean; panic is appropriate.

### 4.3 Flash position still-open

```372:385:contracts/controller/src/strategies/flash_position.rs
    assert_with_error!(
        env,
        !account.is_empty() && !account.debt_free(),
        StrategyError::FlashPositionClosed
    );
    let Some(pos) = account.borrow_positions.get(debt.clone()) else {
        panic_with_error!(env, StrategyError::FlashPositionClosed);
    };
    assert_with_error!(
        env,
        pos.scaled_amount > 0 && !account.supply_positions.is_empty(),
        StrategyError::FlashPositionClosed
    );
```

Three clauses, one error (`#505`-class `FlashPositionClosed`). The middle **must** panic because `get` returns `Option`. Measure equality on the same file uses `assert_with_error!(measured == reported, InternalError)` (T3; A043/A045) plus map-get panics for refund baselines — Option vs bool again.

### 4.4 Auth — `NotAuthorized` vs `AccountNotInMarket`

```132:158:contracts/controller/src/account.rs
    if is_owner_or_delegate(...) {
        return;
    }
    panic_with_error!(env, GenericError::NotAuthorized);
    // ...
    assert_with_error!(env, owner == *caller, GenericError::AccountNotInMarket);
    // ...
    if spoke_id != account.spoke_id {
        panic_with_error!(env, SpokeError::SpokeMismatch);
    }
```

Macro mix is the inverted-early-return vs boolean assert vs compound if. **Error codes** differ on purpose: NFT owner mismatch on delegate admin is `#13`; stranger calling supply/borrow is `#44` (A003, `errors.md`). Cache `ensure_spoke_context` uses `assert_with_error!(ctx.spoke_id() == spoke_id, SpokeMismatch)` — same `#310` as `require_spoke_match`’s panic. Isolating usage (A083) does not depend on which macro.

### 4.5 Multiply mode `_` arm

```162:175:contracts/controller/src/strategies/multiply.rs
    match mode {
        PositionMode::Multiply => { assert_with_error!(..., AssetsAreTheSame); }
        PositionMode::Long | PositionMode::Short => { assert_with_error!(..., AssetsAreTheSame); }
        _ => panic_with_error!(env, CollateralError::InvalidPositionMode),
    }
```

`PositionMode` is `Normal | Multiply | Long | Short`. `_` is **Normal** (and any future XDR tag). Fail closed: Normal cannot take the multiply entry. An exhaustive `PositionMode::Normal => panic_with_error!(InvalidPositionMode)` would be compile-time tighter (A068 style). Convert-swap miss uses `if let else { panic_with_error!(ConvertStepsRequired) }` — Option-shaped.

### 4.6 Payments zero-leg vs `require_positive_amount`

`require_nonneg_amount` asserts `amount >= 0` (`#14`). Rejected zeros then `panic_with_error!(AmountMustBePositive)`. Equivalent to `assert_with_error!(amount > 0)` after the nonneg check, but the match arm is shared with MeansAll sticky-zero. Same `#14`. A061 owns sign/zero policy; this file only notes the macro.

### 4.7 IRM `InterestRateModel::verify` (create-market / governance)

`common/src/types/pool.rs`: most curve checks are `assert_with_error!`; slope monotonicity and one `max_utilization` range use `if { panic_with_error! }` because they are **disjunctions** (`s1 < base || s2 < s1 || …`). Same fail-closed create-market path (A073 read-trust is separate: live markets already passed `verify`).

### 4.8 Token measurement (A058)

`assert_with_error!(amount > 0, caller_error)` then `checked_sub` → `panic_with_error!(AmountMustBePositive)` on overflow. `balance_delta_since` panics `#34` on the same shape of overflow. **Macro is consistent with Option unwrap; the code number is not.** Owned by A058; listed here so A102’s “error-code surface” pointer is answered.

---

## 5. The one untyped panic in shared validation

```49:70:common/src/validation.rs
    let upscale = 10i128
        .checked_pow(exp)
        .expect("10^(RAY_DECIMALS - asset_decimals) fits i128 ...");
    // ...
    if RAY_DECIMALS.checked_sub(asset_decimals).is_none() {
        panic_with_error!(env, CollateralError::AssetDecimalsTooHigh);
    }
    assert_with_error!(..., cap <= max_cap_for_decimals(...), InvalidBorrowParams);
```

`max_cap_for_decimals` returns `0` when decimals exceed `RAY_DECIMALS` **before** `checked_pow`. The `.expect` fires only if `exp` is in range yet `10^exp` overflows `i128` — it does not for `exp ≤ RAY_DECIMALS` (27). Controller listing/cap paths go through `require_cap_within_asset_domain`, which panics typed `#` first. Residual: if a future caller uses `max_cap_for_decimals` alone as a “safe” helper, a violated comment becomes a **host** panic, not `#33`. P5 hygiene: `unwrap_or_else(|| panic_with_error!(env, MathOverflow))` needs an `Env` (the helper currently takes none) or keep the comment and a `const` table.

This is the only `.expect(` / `panic!(` in `common/src` + controller `src` production code besides SDK traps inside token/oracle clients.

---

## 6. What this is **not**

| Non-issue | Why |
|---|---|
| Soft `Result` defenses that callers drop | Controller gates panic; they do not return `Result` for solvency/auth |
| `debug_assert!` stripped in WASM | Unused |
| Flash callback continuing after assert | Callback panic reverts the whole tx (A007/A030/A069) |
| Views vs mutators using different abort kinds | Views also `assert_with_error!` / storage panics; detailed prices use soft `quotes` **by design** (A065) — that is API softness, not a macro |
| Certora vs production | Harness may default storage (A035); production getters still panic. Spec ghost after LTV assert is unrelated to floor macro |
| Pool / aggregator internals | In scope only where controller calls `verify` / `prices`. Pool has its own assert/panic mix under the same SDK rules |

---

## 7. Tests and integrators

Harness and unit tests that `try_supply` / `try_liquidate` and match `Error::from(CollateralError::…)` cannot distinguish which macro fired. Pins for `#126` floor, `#100` insufficient collateral, `#44` auth, `#505` flash closed remain valid under a purely stylistic rewrite.

No dedicated test is required for “macro consistency.” A rewrite of the floor to `assert_with_error!` should keep existing `min_borrow_collateral.rs` pins (A067).

---

## 8. Peer map

| Peer | Relation |
|---|---|
| A067 / A072 | Floor vs HF mix — **style**; gates defended |
| A102 | Flagged this slot; no independent gap beyond hygiene |
| A003 | Auth panic vs assert; `#13` vs `#44` is code mapping |
| A007 / A030 | Panic (either macro) still rolls back flash flag / tx |
| A058 | Overflow **code** split on measured Δ |
| A061 | Zero/sign policy; Rejected-zero panic vs `require_positive_amount` assert |
| A065 / A087 | Oracle miss is `panic_with_error!` — fail closed |
| A068 | Multiply `_` vs exhaustive `PositionMode` |
| A083 | SpokeMismatch assert in Cache vs panic in `require_spoke_match` |
| A039 | Accumulator miss panic on claim |

Disagreement: none. A102’s “mix both for floor vs HF” is **confirmed and non-impacting**.

---

## 9. Optional hygiene (not P0)

| ID | Change | Risk if skipped |
|---|---|---|
| H1 | Floor: `assert_with_error!(floor == 0 \|\| ltv >= floor, MinBorrowCollateralNotMet)` | None — docs/onboarding only |
| H2 | Document two-macro convention in `errors.md` (“assert = predicate, panic = Option / compound / InternalError blocks”) | Contributors keep mixing harmlessly |
| H3 | `max_cap_for_decimals`: typed overflow or `const` pow table | Host trap only on impossible `exp` |
| H4 | `InterestRateModel::verify` disjunctions → `assert_with_error!(monotonic, SlopeNonMonotonic)` | None |
| H5 | Multiply `PositionMode::Normal =>` exhaustive | Compile fail on new modes (align A068) |
| Anti | Do not replace Option unwraps with `assert!(is_some()); unwrap()` | Worse; second unwrap could be untyped |
| Anti | Do not change `#13`/`#44`/`#14`/`#34` in this cleanup | Integrator break; A003/A058 owners |

---

## 10. Opinion (restated)

**Fail-closed typed panics are consistent enough that the abort mechanism is not a vulnerability class.** The codebase already avoids the dangerous tools (Rust `assert!`, bare `unwrap`). Remaining mixes are the SDK’s two spellings of the same trap, chosen by whether the check is a boolean or an `Option`/compound. Wave-4 validation residual for A074 is **info / hygiene**, not a missing `assert_with_error!` on a money path.
