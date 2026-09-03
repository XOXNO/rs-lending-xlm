# A013 — Liquidation self-liquidation and Credit seize recipient rules
- Agent: A013
- Theme: T1 (auth / identity gates on permissionless liquidate), T3 (Credit seize delivery)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/lib.rs:152-166` (`liquidate` entrypoint; no `#[when_not_paused]`)
  - `contracts/controller/src/positions/liquidation/mod.rs:46-153` (`process_liquidation`)
  - `contracts/controller/src/positions/liquidation/mod.rs:170-216` (`resolve_seize_receiver`)
  - `contracts/controller/src/positions/liquidation/apply.rs:139-221` (`apply_liquidation_share_credit`)
  - `contracts/controller/src/positions/liquidation/apply.rs:232-251` (`credit_supply_shares`)
  - `contracts/controller/src/positions/liquidation/apply.rs:292-309` (`require_credit_position_limit`)
  - `contracts/controller/src/account.rs:15-77` (`SpokeAdmission`, `create_account_with`)
  - `contracts/controller/src/account.rs:116-143` (`require_owner_or_delegate`)
  - `common/src/types/controller.rs:233-244` (`SeizeMode`)
  - `common/src/errors.rs:159` (`SelfLiquidationNotAllowed = 133`)
  - `scripts/permissionless_entrypoints.txt:65` (`controller::liquidate`)
  - `contracts/position-nft/src/contract.rs:54-80` (monotonic mint; id 0 reserved)
- Defense: what exists
- Gap: residual coverage / naming / docs only (no fund-safety hole found)
- Impact: none demonstrated for bypass of credit-back or unauthorized receiver credit
- Evidence: INV-LIQ-01, INV-AUTH-02/03, ADR-0019, ADR-0009, harness + unit + Certora pins below
- Opinion: receiver-side identity gate is the right shape; caller-side self-liquidation is intentionally open

## Scope

Audit of (1) whether an account owner may liquidate their own account, (2) Credit-mode seize recipient admission, (3) `Credit(0)` account creation (including deprecated spokes), and (4) the hard rule that seized shares must not be credited back to the liquidated account.

Out of scope for depth (peer agents): Transfer money movement (A051), Credit share math / fee reclassification (A052), bad-debt authority split (A014), flash reentrancy (A007).

## Verdict

**Defended.** Caller identity is permissionless (owner included). The only remaining identity guard is receiver-side and fires only in `SeizeMode::Credit`: the receiving account id must not equal the liquidated account id. `Credit(0)` mints a fresh Normal-mode account owned by the liquidator on the victim's spoke, with an explicit deprecated-spoke exemption so liquidation stays live after `remove_spoke`. Existing receivers are gated on owner-or-delegate, same spoke, and `PositionMode::Normal`, with a second spoke assert inside the apply path.

No path was found that credits the liquidated account, creates a Credit receiver in a foreign spoke, or lets an unrelated address take share credit onto an account they do not control.

---

## 1. Entrypoint and auth surface

`liquidate` is permissionless: liquidator signature only, no owner check on the victim, no `#[when_not_paused]` (exits stay open under protocol pause — `docs/reference/endpoints.md` cross-cutting rule 10).

```152:166:contracts/controller/src/lib.rs
    fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
        seize_mode: SeizeMode,
    ) -> u64 {
        positions::liquidation::process_liquidation(
            &env,
            &liquidator,
            account_id,
            &debt_payments,
            seize_mode,
        )
    }
```

`process_liquidation` immediately:

1. `liquidator.require_auth()`
2. `validation::require_not_flash_loaning(env)`
3. **`resolve_seize_receiver` before any token movement** (fail closed on bad receiver)
4. plan → repay → seize → finalize → optional bad-debt cleanup

Declared in `scripts/permissionless_entrypoints.txt` with INV-AUTH-03 / INV-LIQ-01 / INV-LIQ-02 and an explicit note that Credit mode forbids the liquidated account as receiver.

---

## 2. Self-liquidation (caller = owner) — allowed by design

INV-LIQ-01: liquidation is permissionless **and** an account owner may liquidate its own account. There is **no** caller≠owner check.

| Mode | Owner as liquidator | Collateral destination | Seizure undone? |
|---|---|---|---|
| `Transfer` | Allowed | Underlying tokens to liquidator address (wallet) | No — leaves the lending account |
| `Credit(other_id)` | Allowed if `other_id` passes receiver gates | Shares on `other_id` | No — different account id |
| `Credit(0)` | Allowed | Fresh account owned by liquidator | No — new id |
| `Credit(same account_id)` | **Rejected** `#133` | Would return shares to victim | Yes — blocked |

Historical note: error name `SelfLiquidationNotAllowed` no longer means “owner cannot liquidate self”. It means “cannot credit seize back into the liquidated account”. Skills and INV-LIQ-01 document the rename; the Rust variant name remains the old wording.

### Evidence (caller self-liq allowed)

| Test / rule | What it pins |
|---|---|
| `tests/test-harness/tests/controller/liquidation.rs` — `test_self_liquidation_allowed` | Owner liquidates own account; debt falls |
| same file — `test_third_party_supply_self_liquidation_allowed` | Still allowed after third-party top-up (INV-AUTH-03) |
| `tests/test-harness/tests/controller/security_audit_extended.rs` — `refutation_owner_can_self_liquidate` | Explicit refutation of “owner blocked” |
| `tests/test-harness/tests/controller/position_nft.rs` — `self_liquidation_is_allowed` | After NFT transfer + adopt, new owner can self-liquidate |

Threat model / STRIDE: I5 and the liquidate sequence diagram state permissionless including owner; Credit mode only adds receiver ≠ liquidated account.

---

## 3. `resolve_seize_receiver` — Credit recipient rules

Single gate for delivery destination (`mod.rs:170-216`):

```170:216:contracts/controller/src/positions/liquidation/mod.rs
fn resolve_seize_receiver(
    env: &Env,
    liquidator: &Address,
    account_id: u64,
    account: &Account,
    seize_mode: SeizeMode,
    cache: &mut Cache,
) -> Option<(u64, Account)> {
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

    // Crediting the liquidated account would hand its own collateral straight back and undo
    // the seizure.
    assert_with_error!(
        env,
        requested != account_id,
        CollateralError::SelfLiquidationNotAllowed
    );

    let receiver = storage::get_account(env, requested);
    account::require_owner_or_delegate(env, requested, liquidator, &receiver.owner);
    assert_with_error!(
        env,
        receiver.spoke_id == account.spoke_id,
        SpokeError::SpokeMismatch
    );
    assert_with_error!(
        env,
        receiver.mode == PositionMode::Normal,
        GenericError::AccountModeMismatch
    );

    Some((requested, receiver))
}
```

### Gate matrix (`SeizeMode::Credit(id)`, `id != 0`)

| Check | Error | Rationale |
|---|---|---|
| `requested != account_id` | `#133 SelfLiquidationNotAllowed` | Credit-back undoes seizure (INV-LIQ-01) |
| Account exists | `#… AccountNotFound` via `get_account` | No silent create for nonzero ids |
| `require_owner_or_delegate(liquidator, receiver)` | `#… NotAuthorized` | INV-AUTH-02 on the **receiver**, not the victim |
| `receiver.spoke_id == account.spoke_id` | `#… SpokeMismatch` | ADR-0009 / ADR-0019 — no foreign risk regime |
| `receiver.mode == PositionMode::Normal` | `#… AccountModeMismatch` | Strategy modes (Multiply/Long/Short) carry invariants this path does not establish |

`Transfer` returns `None`; pool pays the liquidator in underlying — no account-id receiver to confuse with the victim.

### Defense in depth after resolve

- `apply_liquidation_share_credit` re-asserts `receiver.spoke_id == account.spoke_id` before each leg.
- Conservation identity `seized_scaled - liquidator_scaled == fee_scaled` asserted per leg.
- `require_credit_position_limit` runs before credit apply; liquidator can fall back to `Credit(0)`.
- `credit_supply_shares` stamps **current listing** risk params on new slots; never imports the victim's stale tuple (ADR-0019).
- Account↔account share move deliberately skips spoke supply-cap entry so a full spoke stays liquidatable; only the fee exits usage.

---

## 4. Cannot credit the liquidated account

### Mechanism

Equality on **account id**, not address. Same Stellar address may own many accounts; crediting a *different* id owned by the same person is allowed and is the intended self-help / keeper pattern. Crediting the *same* id is rejected before the receiver is even loaded.

Order matters: self-check runs **before** `get_account` / auth / spoke / mode, so a malicious `Credit(victim_id)` fails even if the liquidator is the victim's owner or delegate.

### Why Transfer does not need this check

Transfer withdraws shares from the victim and pays **tokens** to `liquidator`. Even when `liquidator` is the owner, collateral leaves the account's supply map. There is no “credit back” path.

### Id-reuse / `Credit(0)` collision

Position NFT mint is sequential and **burn does not recycle ids** (`contracts/position-nft/README.md`; constructor burns id 0 so first live id is 1). `Credit(0)` therefore cannot mint the liquidated account's id. Certora rule `credit_zero_liquidation_creates_receiver_in_deprecated_spoke` additionally pins the next mint id away from the victim when proving deprecated-spoke liveness.

### Evidence

| Artifact | Pin |
|---|---|
| Harness `credit_back_into_the_liquidated_account_reverts` | `Credit(alice_id)` → `#133` |
| INV-LIQ-01 text | `requested != account_id`, error 133 |
| `docs/reference/errors.md` row 133 | Raised when seizure receiver is the liquidated account |
| `common/src/types/controller.rs` SeizeMode rustdoc | Explicitly forbids liquidated account as Credit id |
| STRIDE I5 / sequence note | Credit mode: receiver ≠ liquidated account |

No Certora **assert** rule was found that universally proves `receiver_id != account_id` for all Credit calls (isolation rules name distinct target/receiver ids by fixture). Residual formal gap only; runtime + harness cover the property.

---

## 5. `Credit(0)` account creation

### Behavior

| Property | Value |
|---|---|
| Owner | `liquidator` (already authed) |
| Spoke | **Victim's** `account.spoke_id` (cannot choose another) |
| Mode | Always `PositionMode::Normal` |
| Spoke admission | `SpokeAdmission::AllowDeprecated` |
| Persistence | NFT mint + `set_account_meta`; returned id is the liquidate return value |
| Creation event | **None** — announced only via second `UpdatePositionBatchEvent` `account_attributes` (ADR-0019 / endpoints rule 9) |

```15:24:contracts/controller/src/account.rs
pub(crate) enum SpokeAdmission {
    /// New exposure: the spoke must be active.
    ActiveOnly,
    /// A liquidation seizure receiver: the spoke need only exist. Seizure
    /// reduces risk in that spoke, and a deprecated spoke may hold live
    /// positions forever because deprecation is one-way and unchecked.
    AllowDeprecated,
}
```

`AllowDeprecated` only loads `spoke_config` (must exist); it does **not** call `active_spoke`. Ordinary `create_account` / supply / multiply stay `ActiveOnly` and reject deprecated spokes.

### Liveness rationale (INV-LIQ-01)

`remove_spoke` performs no usage check and deprecation is one-way. Live underwater positions can remain forever in a deprecated spoke. Without `Credit(0)` + `AllowDeprecated`, a liquidator with no prior account there would be forced onto `Transfer`, which needs pool cash — exactly when markets are stressed.

### Evidence

| Artifact | Pin |
|---|---|
| `deprecated_spoke_liquidation_liveness.rs` | Transfer fails cash-starved; `Credit(0)` creates receiver on deprecated spoke; supply/multiply stay closed |
| Certora `credit_zero_liquidation_creates_receiver_in_deprecated_spoke` | Satisfies `receiver != 0` under deprecated spoke |
| Unit `liquidation_zero_threshold.rs` | `Credit(0)` opens fresh id `> VICTIM` |
| Harness seize-mode suite | Multiple `Credit(0)` happy paths + position-limit fallback to fresh account |

Same-spoke rule cannot be bypassed via creation: spoke is taken from the victim, not caller input.

---

## 6. Attack / bypass attempts considered

| Attempt | Outcome |
|---|---|
| Owner liquidates self in Transfer | Allowed; collateral exits to wallet |
| Owner/delegate `Credit(victim_id)` | `#133` before auth/spoke checks |
| Third party `Credit(victim_id)` | `#133` (same id check) |
| `Credit(bobs_account)` without control | `NotAuthorized` |
| `Credit(other_spoke_account)` | `SpokeMismatch` |
| `Credit(Multiply/Long/Short account)` | `AccountModeMismatch` (Multiply harness-tested; Long/Short same enum arm) |
| `Credit(missing_id)` | `AccountNotFound` |
| `Credit(0)` into foreign spoke | Impossible — spoke copied from victim |
| `Credit(0)` after deprecate | Allowed (intentional) |
| Mint collision recreating victim id | Prevented by monotonic NFT counter |
| Estimate view with `Credit(victim_id)` | View-only; does **not** call `resolve_seize_receiver` — no write, no bypass |
| Reenter via flash during liquidate | Blocked by `require_not_flash_loaning` (A007) |
| Cap-full spoke blocks Credit seize | Skipped for A↔A move; only fee usage exit — intentional ADR-0019 |

---

## 7. Residual gaps (non-blocking)

1. **Error naming.** `SelfLiquidationNotAllowed` reads as “no self-liq” to integrators; behavior is credit-back only. Docs/skills already clarify; renaming would be a breaking error-code concern (discriminants are stable).
2. **`docs/reference/endpoints.md` Credit(id) bullet** lists ownership, spoke, Normal mode but omits “≠ liquidated account” in that paragraph (it appears in INV-LIQ-01 / permissionless file / SeizeMode rustdoc). Doc completeness only.
3. **Threat-model row** “Share-credit liquidation” cites INV-LIQ-02 for receiver routing; the identity property is INV-LIQ-01 (+ INV-AUTH-02 on receiver). Cross-ref hygiene.
4. **Test coverage holes (behavior still enforced in code):**
   - No harness test for `Credit(Long|Short)` rejection (only Multiply).
   - No positive harness test that an **active delegate** of the receiver may liquidate and credit that receiver (negative unauthorized path exists).
   - Owner self-liq harnesses use Transfer; no dedicated “owner + `Credit(0)`” case (still covered by general Credit(0) paths with LIQUIDATOR ≠ victim).
5. **Formal:** no dedicated CVL assert that ∀ Credit liquidations, `receiver ≠ target`. Isolation rule assumes distinct ids.

None of the above yields an undefended money path under current source.

---

## 8. Impact quantification

| Scenario if gate failed | Blast |
|---|---|
| Credit-back to liquidated account | Seizure cancelled in-place: debt reduced, collateral restored → unhealthy account can be “liquidated” without losing collateral; liquidator pays debt for no net seize (grief / broken incentive); protocol fee path may still skim — accounting confusion |
| Credit to uncontrolled account | Theft of seized shares onto victim-chosen or third-party account |
| Credit across spoke | Import foreign / stale risk regime (ADR-0009 break); possible under-collateralized borrow later |

**Observed residual risk with gates intact:** integrator confusion from error name / missing endpoints sentence; indexer miss of `Credit(0)` accounts without reading position-batch attributes (documented).

---

## Opinion

The design correctly separated **caller** permissionlessness (including self-help liquidation) from **receiver** integrity (cannot undo seize by crediting the same account). Binding Credit receivers to the victim's spoke with `AllowDeprecated` on create is the right liveness trade for deprecated spokes. Enforcement is early, single-sourced in `resolve_seize_receiver`, and reinforced in apply. Treat as **defended** for T1/T3 identity on liquidate; residual items are documentation and coverage polish, not open bypasses.
