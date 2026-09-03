# A001 — ControllerInterface / ControllerAdmin entrypoint macro inventory

- Agent: A001
- Theme: T1
- Severity: info
- Status: defended
- Paths: `contracts/controller/src/lib.rs:94-559` (`ControllerInterface`), `contracts/controller/src/lib.rs:561-847` (`ControllerAdmin`); expected gates from `docs/reference/invariants.md` INV-HALT-01 (`:469-477`), `docs/reference/endpoints.md` pause column (`:25-28`, `:54-61`, `:127-132`, `:183-198`, `:415-421`, `:443-452`, `:524-531`), `docs/explanation/threat-model.md:437-439`
- Defense: Global pause and owner macros are applied exactly where INV-HALT-01 / endpoints.md require them. Risk-increasing and strategy surfaces carry `#[when_not_paused]`; intentional exits / liquidation / cleanup / recap / TTL / revoke stay open. Admin mutators carry `#[only_owner]` except the deliberate `accept_ownership` exception.
- Gap: No mutator is missing an *expected* pause or owner gate relative to the docs of record. Residual notes only: (1) INV-HALT-01 prose under-lists gated verbs that code and endpoints already mark gated; (2) strategy “exit-like” verbs (`repay_debt_with_collateral`, `swap_*`) remain pause-gated by design, so leverage unwind during a global pause must use bare `withdraw`/`repay`.
- Impact: None for funds under current expected policy. A mistaken future removal of `#[when_not_paused]` on supply/borrow/strategies would reopen risk during halt; a mistaken *addition* of pause to withdraw/repay/liquidate would trap exits (INV-HALT-01 / ADR-0008).
- Evidence: Macro sites below; INV-HALT-01; INV-AUTH-04 (admin pause asymmetry lives in governance, not these macros); `scripts/permissionless_entrypoints.txt` (auth category, not pause); harness `tests/test-harness/tests/controller/supply.rs` (`test_supply_rejects_when_paused`), `withdraw.rs` (`test_withdraw_allowed_when_paused`), `liquidation.rs` (`test_liquidation_allowed_when_paused`), `security_audit_extended.rs` (`poc_global_pause_blocks_risk_increasing_allows_exit_and_liq`).
- Opinion: Entrypoint macro surface matches the halt design. Treat any future edit that flips a row in the tables below as a security change requiring INV-HALT-01 + endpoints.md updates in the same PR.

## Method

1. Enumerated every `fn` in `impl ControllerInterface for Controller` and `impl ControllerAdmin for Controller` in `contracts/controller/src/lib.rs`.
2. Recorded presence/absence of `#[when_not_paused]` and `#[only_owner]` on the impl method (macros are not on the trait in `interfaces/controller`).
3. Classified mutator vs view by whether the entrypoint is a state-changing user/admin verb (views excluded from “missing gate” flags; listed for completeness).
4. Compared each mutator to the expected pause/owner policy in INV-HALT-01, endpoints.md, and threat-model pause bullet.

Expected policy used for flagging:

| Class | Expected `#[when_not_paused]` | Expected `#[only_owner]` |
|---|---|---|
| Risk-increasing position / strategy / flash / keeper accrual & revenue & threshold / grant delegate | yes | no |
| Safe exit / liquidation / dust clean / recapitalize / renew TTL / revoke delegate | no (must stay open) | no |
| `ControllerAdmin` mutators | no (must work while paused, esp. `unpause` / `upgrade`) | yes |
| `accept_ownership` | no | no (pending-owner completion) |
| Views | n/a | n/a |

---

## A. `ControllerInterface` mutators — gate matrix

All lines cite the `fn` definition in `contracts/controller/src/lib.rs`. Macro attribute lines are one line above the `fn` when present.

| # | Entrypoint | Line | `#[when_not_paused]` | `#[only_owner]` | Expected pause | Gate verdict | Severity if wrong |
|---|---|---:|---|---|---|---|---|
| 1 | `supply` | 99 | yes (`:98`) | no | gated | match | high if ungated |
| 2 | `borrow` | 113 | yes (`:112`) | no | gated | match | high if ungated |
| 3 | `withdraw` | 127 | no | no | open | match | high if gated |
| 4 | `repay` | 139 | no | no | open | match | high if gated |
| 5 | `liquidate` | 152 | no | no | open | match | critical if gated |
| 6 | `clean_bad_debt` | 171 | no | no | open | match | medium if gated |
| 7 | `flash_loan` | 179 | yes (`:178`) | no | gated | match | high if ungated |
| 8 | `flash_position` | 195 | yes (`:194`) | no | gated | match | high if ungated |
| 9 | `multiply` | 231 | yes (`:230`) | no | gated | match | high if ungated |
| 10 | `swap_debt` | 265 | yes (`:264`) | no | gated | match | medium if ungated |
| 11 | `swap_collateral` | 291 | yes (`:290`) | no | gated | match | medium if ungated |
| 12 | `repay_debt_with_collateral` | 318 | yes (`:317`) | no | gated (strategy) | match | low/info if opened — see note N2 |
| 13 | `migrate_from_blend` | 348 | yes (`:347`) | no | gated | match | high if ungated |
| 14 | `update_indexes` | 377 | yes (`:376`) | no | gated | match | low if ungated |
| 15 | `claim_revenue` | 385 | yes (`:384`) | no | gated | match | low if ungated |
| 16 | `update_account_threshold` | 394 | yes (`:393`) | no | gated | match | medium if ungated |
| 17 | `recapitalize` | 401 | no | no | open | match | medium if gated |
| 18 | `renew_account` | 407 | no | no | open | match | medium if gated (TTL trap) |
| 19 | `add_delegate` | 415 | yes (`:414`) | no | gated | match | medium if ungated |
| 20 | `remove_delegate` | 421 | no | no | open | match | medium if gated |

**Mutator count:** 20. **Pause-gated:** 13. **Intentionally open:** 7. **Owner-gated on this trait:** 0 (correct — owner surface is `ControllerAdmin`).

### Flagged pause/owner gaps on `ControllerInterface`

**None.** Every mutator’s macros match the expected policy above.

---

## B. `ControllerInterface` views (no pause/owner macros; not mutators)

Listed so the inventory is complete against `impl ControllerInterface` (`:427-558`). None should carry `#[when_not_paused]` or `#[only_owner]`.

| Entrypoint | Line | Macros | Verdict |
|---|---:|---|---|
| `is_liquidatable` | 427 | neither | ok |
| `get_health_factor` | 434 | neither | ok |
| `get_total_collateral_usd` | 440 | neither | ok |
| `get_total_borrow_usd` | 445 | neither | ok |
| `get_collateral_amount` | 451 | neither | ok |
| `get_borrow_amount` | 457 | neither | ok |
| `get_account_positions` | 462 | neither | ok |
| `get_account_attributes` | 473 | neither | ok |
| `account_exists` | 478 | neither | ok |
| `get_liquidation_estimate` | 487 | neither | ok |
| `get_liquidation_collateral` | 498 | neither | ok |
| `get_ltv_collateral_usd` | 504 | neither | ok |
| `get_pool_address` | 509 | neither | ok |
| `get_market_index` | 514 | neither | ok |
| `get_market_indexes_detailed` | 522 | neither | ok |
| `get_spoke` | 527 | neither | ok |
| `get_spoke_asset` | 533 | neither | ok |
| `get_spoke_usage` | 540 | neither | ok |
| `price_aggregator` | 545 | neither | ok |
| `get_min_borrow_collateral_usd` | 551 | neither | ok |
| `is_blend_pool_approved` | 556 | neither | ok |

**View count:** 21. **Unexpected macros:** none.

---

## C. `ControllerAdmin` cross-check (`impl ControllerAdmin for Controller`)

| # | Entrypoint | Line | `#[only_owner]` | `#[when_not_paused]` | Expected | Gate verdict | Severity if wrong |
|---|---|---:|---|---|---|---|---|
| 1 | `set_swap_aggregator` | 566 | yes (`:565`) | no | owner, not pause | match | critical if ungated |
| 2 | `set_price_aggregator` | 573 | yes (`:572`) | no | owner, not pause | match | critical if ungated |
| 3 | `set_accumulator` | 580 | yes (`:579`) | no | owner, not pause | match | high if ungated |
| 4 | `set_position_limits` | 588 | yes (`:587`) | no | owner, not pause | match | high if ungated |
| 5 | `set_min_borrow_collateral_usd` | 596 | yes (`:595`) | no | owner, not pause | match | high if ungated |
| 6 | `set_position_manager` | 606 | yes (`:605`) | no | owner, not pause | match | high if ungated |
| 7 | `approve_blend_pool` | 616 | yes (`:615`) | no | owner, not pause | match | high if ungated |
| 8 | `revoke_blend_pool` | 626 | yes (`:625`) | no | owner, not pause | match | medium if ungated |
| 9 | `create_hub` | 636 | yes (`:635`) | no | owner, not pause | match | medium if ungated |
| 10 | `add_spoke` | 643 | yes (`:642`) | no | owner, not pause | match | medium if ungated |
| 11 | `remove_spoke` | 650 | yes (`:649`) | no | owner, not pause | match | high if ungated |
| 12 | `set_spoke_liquidation_curve` | 658 | yes (`:657`) | no | owner, not pause | match | high if ungated |
| 13 | `add_asset_to_spoke` | 682 | yes (`:681`) | no | owner, not pause | match | high if ungated |
| 14 | `edit_asset_in_spoke` | 690 | yes (`:689`) | no | owner, not pause | match | high if ungated |
| 15 | `set_spoke_asset_flags` | 698 | yes (`:697`) | no | owner, not pause | match | high if ungated |
| 16 | `remove_asset_from_spoke` | 718 | yes (`:717`) | no | owner, not pause | match | medium if ungated |
| 17 | `deploy_pool` | 729 | yes (`:728`) | no | owner, not pause | match | critical if ungated |
| 18 | `deploy_position_nft` | 736 | yes (`:735`) | no | owner, not pause | match | critical if ungated |
| 19 | `create_liquidity_pool` | 753 | yes (`:752`) | no | owner, not pause | match | high if ungated |
| 20 | `upgrade_liquidity_pool_params` | 768 | yes (`:767`) | no | owner, not pause | match | high if ungated |
| 21 | `upgrade_pool` | 778 | yes (`:777`) | no | owner, not pause | match | critical if ungated |
| 22 | `upgrade_position_nft` | 785 | yes (`:784`) | no | owner, not pause | match | critical if ungated |
| 23 | `force_socialize_bad_debt` | 793 | yes (`:792`) | no | owner, not pause | match | high if ungated |
| 24 | `pause` | 802 | yes (`:801`) | no | owner, not pause | match | critical if ungated |
| 25 | `unpause` | 808 | yes (`:807`) | no | owner, **must not** be pause-gated | match | critical if pause-gated |
| 26 | `upgrade` | 815 | yes (`:814`) | no | owner, not pause | match | critical if ungated |
| 27 | `migrate` | 822 | yes (`:821`) | no | owner, not pause | match | high if ungated |
| 28 | `transfer_ownership` | 835 | yes (`:834`) | no | owner, not pause | match | critical if ungated |
| — | `get_app_version` | 827 | no | no | view | ok (not a mutator) | — |
| 29 | `accept_ownership` | 844 | no | no | pending owner; not `only_owner` | match | critical if `only_owner` added (would deadlock transfer) |

**Admin mutators:** 29 (`accept_ownership` included). **`#[only_owner]`:** 28. **Deliberate non-owner mutator:** `accept_ownership` (`:844`). **Any admin method with `#[when_not_paused]`:** none (correct).

### Flagged admin gaps

**None** against endpoints.md administration section (`:524-531`): every mutating admin entry carries `#[only_owner]` except `accept_ownership`; `get_app_version` is ungated read.

---

## D. Residual notes (not missing-gate bugs)

### N1 — INV-HALT-01 prose under-lists gated verbs — severity: info

INV-HALT-01 (`docs/reference/invariants.md:474-476`) names: supply, borrow, flash loan, multiply, swaps, migrate, and delegate grants.

Live code *also* pause-gates (and endpoints.md documents):

- `flash_position` — `lib.rs:194`
- `update_indexes` — `lib.rs:376`
- `claim_revenue` — `lib.rs:384`
- `update_account_threshold` — `lib.rs:393`

Code and endpoints agree; the invariant sentence is incomplete. Remediating the doc reduces future “is this a gap?” noise. Not a runtime undefended path.

### N2 — Strategy unwind blocked while globally paused — severity: info (design)

`repay_debt_with_collateral`, `swap_debt`, and `swap_collateral` are pause-gated (`:317`, `:264`, `:290`) per endpoints.md strategies section (“All are pause-gated”). Users can still `withdraw` / `repay` while paused. Impact: during a global pause, leveraged accounts cannot use the one-shot strategy exits and must use multi-step bare exits. Documented intentional; not a missing `#[when_not_paused]` on an expected-open path, nor a missing open attribute on an expected-gated path.

### N3 — Trait vs impl

`interfaces/controller/src/lib.rs` and `admin.rs` declare bare trait methods with no macros. Enforcement is solely on the `#[contractimpl]` methods in `contracts/controller/src/lib.rs`. Future codegen or alternate impls must re-apply the same attributes; the access-control checker / wasm gates are the backstop (out of A001 scope; see A010).

### N4 — Constructor

`Controller::__constructor` (`lib.rs:88`) is outside both traits. No pause/owner macro (construction-time). Out of mutator-inventory scope; noted for completeness.

---

## E. Summary counts

| Surface | Mutators | `when_not_paused` | `only_owner` | Neither (intentional) | Missing expected gate |
|---|---:|---:|---:|---:|---:|
| `ControllerInterface` | 20 | 13 | 0 | 7 | **0** |
| `ControllerAdmin` | 29 | 0 | 28 | 1 (`accept_ownership`) | **0** |
| Views on either trait | 22 | 0 | 0 | 22 | n/a |

**Overall A001 verdict:** defended. No ControllerInterface mutator lacks an expected pause gate; no ControllerAdmin mutator lacks an expected owner gate.
