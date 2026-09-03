# A068 — Mode / SeizeMode enum exhaustive handling in liquidation paths

- Agent: A068
- Theme: T4 (input validation — enum dispatch), T3 (seize delivery branching)
- Severity: info
- Status: defended
- Paths:
  - `common/src/types/controller.rs:227-244` (`SeizeMode` definition + rustdoc)
  - `common/src/types/shared.rs:31-41` (`PositionMode` definition)
  - `contracts/controller/src/lib.rs:152-166` (`liquidate` entrypoint)
  - `contracts/controller/src/lib.rs:487-493` (`get_liquidation_estimate`)
  - `contracts/controller/src/positions/liquidation/mod.rs:46-153` (`process_liquidation`; Option-tagged apply dispatch)
  - `contracts/controller/src/positions/liquidation/mod.rs:170-216` (`resolve_seize_receiver` — sole `match seize_mode`)
  - `contracts/controller/src/positions/liquidation/apply.rs:85-221` (`apply_liquidation_seizures` / `apply_liquidation_share_credit`)
  - `contracts/controller/src/views.rs:189-242` (`liquidation_estimations_detailed` unit selection by mode)
  - `contracts/controller/src/events/mod.rs:9-28` (`EventPositionMode` / exhaustive `From<PositionMode>`)
  - `contracts/controller/src/account.rs:79-114` (`AccountGuard::Multiply` mode equality — contrast surface)
  - `contracts/controller/src/strategies/multiply.rs:155-175` (`validate_multiply_request` `_` catch-all — adjacent pattern)
- Defense: Production liquidation has exactly two `match seize_mode` sites (`resolve_seize_receiver`, estimate view); both enumerate `Transfer` and `Credit(_)` with no wildcard. Apply branching is driven by the `Option` returned from resolve (`None` → Transfer withdraw; `Some` → Credit share path), so a new `SeizeMode` variant cannot compile without updating resolve. Credit receivers are forced to `PositionMode::Normal` (equality check); `Credit(0)` always mints `Normal`. Victim `PositionMode` is intentionally ungated so strategy accounts stay liquidatable. Plan math is mode-agnostic and fills both Transfer and Credit representations on every `SeizeEntry`.
- Gap: (1) Apply site matches `Option`, not `SeizeMode` — a future variant that incorrectly returns `None` would silently take Transfer (compiler exhaustiveness does not catch semantic misfires). (2) Estimate ignores Credit id validity / Normal-mode / spoke gates — keepers can see share units for a receiver that would revert at execution. (3) Harness pins Credit→Multiply reject; Long/Short share the same equality arm but lack dedicated harness cases. (4) Adjacent non-liq `PositionMode` matches elsewhere use `_` / `matches!` (fail-closed, but no compile break on new variants). None of these are fund-safety holes on today’s two-variant `SeizeMode`.
- Impact: No demonstrated path where an unrecognized or half-handled seize mode silently mis-routes collateral, credits a strategy-mode account, or applies Transfer fee minting on a Credit seizure. Blast radius of the residuals is keeper UX (stale estimate) and future-maintainer risk when adding a third delivery mode — not current mainnet fund loss.
- Evidence: ADR-0019; INV-LIQ-01; endpoints.md seize-mode table; rustdoc on `SeizeMode` / `resolve_seize_receiver`; harness `liquidation_seize_modes.rs` (`credit_to_a_strategy_mode_account_reverts`, Transfer↔Credit parity, cash-starved Transfer vs Credit); unit events Transfer/Credit; Certora Transfer-heavy rules plus Credit in `spoke_rules` / `account_isolation_rules`; peers A013, A018, A051, A052, A002.
- Opinion: Exhaustive handling is the right shape today. Keep mode→path coupling inside `resolve_seize_receiver` (or replace the `Option` tag with an explicit internal enum) before any third `SeizeMode` lands.

## Scope and method

1. Read `shared/COORDINATION.md` + `SEED.md` (findings-only; no git ops; no production Rust edits).
2. Inventory every production `match` / equality / dispatch on `SeizeMode` and on `PositionMode` that liquidation touches.
3. Distinguish compiler exhaustiveness from semantic tagging (`Option` as mode proxy).
4. Cross-check estimate vs execution, Credit(0) vs Credit(id), victim mode vs receiver mode.
5. Read peers A013 (receiver identity), A018 (strategy PositionMode), A051 (Transfer money), A052 (Credit money) before claiming novelty.
6. Out of scope as primary claims: fee product math (A053), storage layout (A026), spoke usage (A084), bad-debt socialization (A014/A027).

---

## Verdict

**Defended.** Both live `SeizeMode` variants are named in every production match that selects delivery units or resolves a receiver. Transfer and Credit apply paths are disjoint functions with incompatible fee mechanics (mint-withhold vs absorb-reclassify — ADR-0019), and the only bridge between caller input and those functions is an exhaustive match that returns `None` or `Some(receiver)`. Receiver `PositionMode` is equality-gated to `Normal`; strategy modes cannot silently accept share credit. No wildcard on `SeizeMode` exists in controller liquidation code.

---

## 1. Enum inventory (source of truth)

### 1.1 `SeizeMode` — two variants, call-wide

```233:244:common/src/types/controller.rs
pub enum SeizeMode {
    /// The pool transfers the underlying tokens to the liquidator, withholding the protocol
    /// fee from the outbound amount.
    Transfer,

    /// The seized supply shares are credited to a controller account instead of being
    /// withdrawn. `0` creates a fresh account owned by the liquidator and bound to the
    /// liquidated account's spoke; any other id must already exist, not be the liquidated
    /// account itself, be owned by (or delegated to) the liquidator, sit in the liquidated
    /// account's spoke, and be in `PositionMode::Normal`.
    Credit(u64),
}
```

Design constraints encoded in rustdoc / ADR-0019 / endpoints:

| Constraint | Why it matters for exhaustiveness |
|---|---|
| One mode per call, not per asset | Pro-rata seizure across all collaterals — no per-leg mode matrix to leave incomplete |
| `Credit(0)` vs `Credit(id≠0)` is payload branching, not a third variant | Same enum arm; id `0` is the create sentinel (NFT id 0 reserved) |
| Return `u64`: receiver id or `0` for Transfer | Transfer and “no receiver” share the numeric zero by design |

`#[contracttype]` without a separate `#[repr(u32)]` — XDR/Soroban encoding is the two-variant shape. Adding a third variant is a contract ABI change; old WASM cannot accept an unknown discriminant as a silent default.

### 1.2 `PositionMode` — four variants

```36:41:common/src/types/shared.rs
pub enum PositionMode {
    Normal = 0,
    Multiply = 1,
    Long = 2,
    Short = 3,
}
```

Liquidation cares about this enum only on the **Credit receiver** (must be `Normal`). The **victim** may be any mode; plan/apply never read `account.mode`. That asymmetry is intentional liveness: leveraged accounts must remain liquidatable.

Wire twin `EventPositionMode` maps `Normal → None` and the three strategy modes 1:1 via an exhaustive `From` impl (`events/mod.rs:20-28`) — no `_` arm.

---

## 2. Every production `SeizeMode` match / dispatch site

| Site | Arms | Wildcard? | Role |
|---|---|---|---|
| `resolve_seize_receiver` (`liquidation/mod.rs:178-181`) | `Transfer` → `return None`; `Credit(id)` → `id` | No | Sole execution gate that interprets caller mode |
| `liquidation_estimations_detailed` (`views.rs:213-223`) | `Transfer` → asset units; `Credit(_)` → RAY shares via `split_seized_shares` | No (`Credit(_)` ignores id) | Unit selection for the non-persisting estimate |
| `process_liquidation` apply (`mod.rs:88-102`) | `match &mut receiver { None ⇒ seizures; Some ⇒ share_credit }` | N/A (matches `Option`, not `SeizeMode`) | Secondary tag after resolve |
| `process_liquidation` finalize (`mod.rs:138-148`) | `if let Some(...)` second position batch | N/A | Credit-only second account finalize |

No other controller production file matches on `SeizeMode`. Entrypoints (`lib.rs`) pass the enum through untouched. Plan (`plan.rs`) is deliberately mode-blind: every `SeizeEntry` carries both asset (`amount`/`protocol_fee`) and share (`scaled_amount`/`bonus_scaled`/`liquidation_fees`) representations so either apply path can consume the fields it needs (documented on `SeizeEntry` rustdoc).

### 2.1 Resolve is the exhaustive choke point

```178:215:contracts/controller/src/positions/liquidation/mod.rs
    let requested = match seize_mode {
        SeizeMode::Transfer => return None,
        SeizeMode::Credit(id) => id,
    };

    if requested == 0 {
        return Some(account::create_account_with(
            env,
            liquidator,
            account.spoke_id,
            PositionMode::Normal,
            cache,
            SpokeAdmission::AllowDeprecated,
        ));
    }
    // ... self-id, owner/delegate, spoke, Mode::Normal ...
    Some((requested, receiver))
```

Properties:

1. **Compiler exhaustiveness** — a third `SeizeMode` variant fails to compile here until an arm is written.
2. **Fail-before-money** — resolve runs before any repay/seize token movement (`mod.rs:62-65`).
3. **Credit payload split is total** — `id == 0` create vs `id != 0` load; no fallthrough.
4. **Create path hard-codes `PositionMode::Normal`** — cannot mint a Multiply/Long/Short Credit(0) receiver.

### 2.2 Apply branching via `Option` (semantic tag)

```88:102:contracts/controller/src/positions/liquidation/mod.rs
    match &mut receiver {
        None => {
            apply::apply_liquidation_seizures(env, liquidator, &mut account, &seized, &mut cache)
        }
        Some((_, receiving_account)) => {
            apply::require_credit_position_limit(env, receiving_account, &seized);
            apply::apply_liquidation_share_credit(
                env,
                &mut account,
                receiving_account,
                &seized,
                &mut cache,
            );
        }
    }
```

| Tag | Function | Fee path | Pool cash |
|---|---|---|---|
| `None` (Transfer) | `apply_liquidation_seizures` | Pool `withhold_liquidation_fee` (mint revenue backed by withheld cash) | Debited |
| `Some` (Credit) | `apply_liquidation_share_credit` | `PoolSeizeEntry { Deposit }` → `absorb_supply_as_revenue` (reclassify) | Untouched |

The two apply functions are separate entry points with no internal `SeizeMode` switch — correct, so Transfer mint logic cannot be “accidentally selected” inside the Credit function. Coupling correctness rests entirely on resolve returning the right `Option`.

**Maintainer residual (not a current bug):** if a future third mode were added and its resolve arm returned `None` by mistake, the Transfer withdraw path would run. Rust exhaustiveness would be satisfied; semantic mode→path mapping would not. Mitigation when extending: introduce an explicit internal `enum SeizeDelivery { Transfer, Credit { receiver } }` (or match `seize_mode` again at the apply site) so the tag cannot collapse distinct modes onto `None`.

### 2.3 Estimate match — units only, no admission

```213:223:contracts/controller/src/views.rs
        let (seized_amount, fee_amount) = match seize_mode {
            SeizeMode::Transfer => (entry.amount, entry.protocol_fee),
            SeizeMode::Credit(_) => {
                let (fee_scaled, _) = split_seized_shares(
                    env,
                    Ray::from(entry.scaled_amount),
                    Ray::from(entry.bonus_scaled),
                    entry.liquidation_fees,
                );
                (entry.scaled_amount, fee_scaled.raw())
            }
        };
```

- Exhaustive on today’s variants.
- `Credit(_)` correctly ignores the id for **reporting units** (shares vs assets).
- Does **not** call `resolve_seize_receiver` — by design for a pure view (`Cache::new_view`, no persist). Consequence: `get_liquidation_estimate(..., Credit(strategy_id))` or `Credit(missing_id)` still returns share-denominated numbers that execution would reject. Documented as simulation; residual is keeper/integrator UX, not silent wrong settlement.

Harness pins mode-aware estimate units (`liquidation_seize_modes.rs` Transfer vs Credit magnitude divergence).

---

## 3. `PositionMode` inside liquidation

### 3.1 Receiver — equality to `Normal` (not a full match)

```209:213:contracts/controller/src/positions/liquidation/mod.rs
    assert_with_error!(
        env,
        receiver.mode == PositionMode::Normal,
        GenericError::AccountModeMismatch
    );
```

| Receiver mode | Outcome |
|---|---|
| `Normal` | Admitted (then spoke/auth already passed) |
| `Multiply` / `Long` / `Short` | `#25 AccountModeMismatch` |

Rationale in resolve rustdoc: strategy modes carry invariants this path does not establish (no `strategy_finalize`, no multiply asset-pair rules, no flash-position refund contract).

`Credit(0)` bypasses the equality check by construction — create always stores `Normal`.

Harness: `credit_to_a_strategy_mode_account_reverts` creates `PositionMode::Multiply` and expects `ACCOUNT_MODE_MISMATCH`. Long/Short are the same boolean (`!= Normal`); no separate harness cases — **coverage residual only**, same arm.

### 3.2 Victim — ungated (intentional)

`build_liquidation_plan` / repay / seize never inspect `account.mode`. A Multiply/Long/Short account with HF &lt; 1 is liquidatable in both SeizeModes. That is required for protocol solvency; gating the victim on `Normal` would strand leveraged debt.

Contrast (non-liq): `AccountGuard::Multiply` requires `account.mode == mode` on strategy verbs (`account.rs:107-111`). Liquidation is the opposite policy: ignore victim mode, constrain receiver mode.

### 3.3 Adjacent `PositionMode` patterns (context, not liq bugs)

| Site | Pattern | On new variant |
|---|---|---|
| `EventPositionMode::from` | Exhaustive four-arm match | Compile break |
| `validate_multiply_request` | `Multiply` / `Long\|Short` / `_ => InvalidPositionMode` | `_` swallows new variants (fail-closed, no compile break) |
| `flash_position` mode gate | `matches!(Multiply \| Long \| Short)` | New variant rejected until list updated (fail-closed) |

These reinforce that liquidation’s receiver check (`== Normal`) is the strictest “only Normal” gate on the Credit path, while strategy code uses fail-closed wildcards elsewhere. A068 does not re-litigate A018; noted only because the agent id pairs Mode with SeizeMode.

---

## 4. Incomplete-handling failure modes checked (and rejected)

| Hypothesis | Result |
|---|---|
| Transfer arm missing → Credit path runs with no receiver | Impossible — resolve returns `None`; apply takes Transfer |
| Credit arm missing → fall into Transfer | Impossible — match is exhaustive; would not compile |
| `Credit(id)` treated as Transfer when id is 0 | No — `id == 0` creates Normal account and returns `Some` |
| Credit applies Transfer fee mint | No — separate function; absorb path only (A052 / ADR-0019) |
| Transfer applies share credit | No — `receiver` is `None`; no second account |
| Strategy-mode Credit receiver accepted | Rejected `#25` |
| Credit back to victim id | Rejected `#133` before load (A013 / INV-LIQ-01) |
| Estimate units wrong for mode | Match selects asset vs shares; harness pins divergence |
| Plan omits Credit fields for Transfer calls | Plan always fills both representations |
| Return `0` ambiguous with Credit | Credit returns minted/existing id ≥ 1; Transfer returns `0` |
| Script-runner `mode => mode` catch-all | Test tooling only; Credit ids resolved, Transfer passed through — not deployable controller code |

---

## 5. Control-flow map (mode → path)

```
liquidate(..., seize_mode)
  └─ process_liquidation
       ├─ resolve_seize_receiver(seize_mode)     # EXHAUSTIVE match SeizeMode
       │    ├─ Transfer        → None
       │    └─ Credit(id)
       │         ├─ id == 0    → create Normal + AllowDeprecated → Some
       │         └─ id != 0    → ≠victim, auth, same spoke, mode==Normal → Some
       ├─ build_liquidation_plan                 # mode-agnostic; dual SeizeEntry fields
       ├─ apply_liquidation_repayments           # shared
       ├─ scale_seizures_to_received             # shared
       ├─ match Option receiver                  # SEMANTIC tag
       │    ├─ None → apply_liquidation_seizures      # Transfer
       │    └─ Some → require_credit_position_limit
       │              + apply_liquidation_share_credit # Credit
       ├─ finalize victim Both
       └─ if Some → record LiqCredit + finalize receiver Supply
```

Estimate path shares the plan, then only the views match for unit selection — no resolve, no apply.

---

## 6. Evidence matrix

| Claim | Artifact |
|---|---|
| Two-variant SeizeMode + Normal receiver rule | `common/src/types/controller.rs` rustdoc; ADR-0019; endpoints.md table |
| Exhaustive resolve match | `liquidation/mod.rs:178-181` |
| Exhaustive estimate match | `views.rs:213-223` |
| Credit(0) → Normal | `create_account_with(..., PositionMode::Normal, ...)` |
| Strategy receiver reject | harness `credit_to_a_strategy_mode_account_reverts` |
| Transfer vs Credit money split | A051 / A052; ADR-0019 auditor focus (mint vs absorb) |
| Cash-starved Transfer / live Credit | harness `cash_starved_market_blocks_transfer_but_not_credit`; deprecated-spoke Credit(0) liveness |
| Mode-aware estimate units | harness estimate Transfer vs Credit magnitude |
| Self-credit blocked | harness + INV-LIQ-01 + A013 |
| Event mode mapping exhaustive | `events/mod.rs` + unit `events.rs` From pins |
| Formal Credit paths | Certora `spoke_rules` Credit legs; `account_isolation_rules` names both principals |
| Formal Transfer paths | `liquidation_rules.rs`, `flash_loan_rules.rs`, `compat.rs` fixture Transfer |

---

## 7. Gaps and non-gaps

### Defended (no action)

- Compiler-enforced exhaustiveness on both production `match seize_mode` sites.
- Disjoint apply functions with incompatible fee semantics.
- Receiver `PositionMode::Normal` equality + Credit(0) hard-coded Normal.
- Mode-agnostic plan with dual field representation.
- Victim strategy modes remain liquidatable.

### Residuals (info)

1. **`Option` as mode tag** — future third seize mode must not reuse `None` for “non-Credit”; prefer an explicit delivery enum or a second exhaustive match at apply time.
2. **Estimate skips admission** — document for keepers (already implied by “simulates… without persisting”); optional harden: call a pure validation of Credit id without writes (not required for fund safety).
3. **Long/Short Credit reject untested directly** — same `== Normal` arm as Multiply; add two one-liners if tightening coverage.
4. **Certora asymmetry** — many rules hard-code `SeizeMode::Transfer`; Credit is covered in spoke/isolation specs but not every Transfer rule has a Credit twin. Formal residual, not runtime hole.
5. **Non-liq `PositionMode` `_` arms** — outside liquidation; fail-closed; mentioned for Mode-enum completeness of the A068 title.

### Explicit non-claims

- Does not assert Transfer/Credit economic fee magnitude parity (A053).
- Does not re-derive share conservation or absorb-vs-mint (A052).
- Does not re-audit receiver auth/spoke/self-id beyond noting they sit inside the Credit arm (A013).

---

## 8. Peer alignment

| Peer | Overlap | Agreement |
|---|---|---|
| A013 | Credit receiver gates including `mode == Normal` | Agree — A068 treats that as the PositionMode half of exhaustive handling |
| A018 | Strategy PositionMode on multiply/flash | Agree — complementary; liquidation does not use AccountGuard |
| A051 | Transfer path when resolve returns `None` | Agree — Option tag correctly selects Transfer money flow |
| A052 | Credit path when `Some` | Agree — separate apply; no Transfer fee mint |
| A002 | Permissionless liquidate + Credit admission | Agree on Normal-mode receiver requirement |

No disagreement file warranted.

---

## 9. Opinion

For a two-variant delivery enum, the implementation is clean: one exhaustive resolve match owns semantics, apply code is mode-free and path-specialized, and PositionMode policy is “receiver Normal / victim any.” The only structural smell is encoding Transfer as `Option::None`, which is fine until a third delivery mode appears — at which point the tag should be promoted to an explicit internal enum so exhaustiveness and semantics stay coupled. No current fund-safety gap from incomplete enum handling.
