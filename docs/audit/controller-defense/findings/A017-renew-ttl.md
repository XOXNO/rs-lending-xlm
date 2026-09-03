# A017 — renew_account TTL-only mutation defense
- Agent: A017
- Theme: T1
- Severity: info
- Status: defended
- Paths: `contracts/controller/src/lib.rs:405-409`, `contracts/controller/src/account.rs:223-237`, `contracts/controller/src/account.rs:145-152`, `contracts/controller/src/storage/account.rs:257-270`, `contracts/controller/src/storage/protocol.rs:155-161`, `contracts/controller/src/external/position_nft.rs:30-36`, `contracts/position-nft/src/contract.rs:103-123`, `contracts/position-nft/src/contract.rs:35-47`, `common/src/ttl.rs:11-15`, `scripts/permissionless_entrypoints.txt:55`
- Defense: Owner-only auth (`require_auth` + `require_account_owner`); mutations are exclusively `extend_ttl` on controller instance, live account keys, and NFT Owner/Balance; no position/meta/delegate/spoke/pool writes; intentionally pause-open
- Gap: none for the scoped claim (TTL-only + owner gate + no accounting side effects). Residual operational note: NFT enumeration keys stay on OZ defaults (INV-STOR-02d / F-11), outside this entrypoint's remit
- Impact: Blast radius of a successful call is rent/TTL only — cannot mint/burn shares, move tokens, alter debt, change spoke usage, or reassign ownership. Failed non-owner calls revert entirely (no committed TTL bumps)
- Evidence: INV-STOR-01, INV-STOR-02b, INV-AUTH-02, INV-HALT-01; permissionless surface line for `controller::renew_account`; harness + unit tests listed below
- Opinion: The path is tightly defended for its threat (stranded account state / asymmetric NFT Owner TTL). Owner gate is stricter than owner-or-delegate, matching INV-AUTH-02's "delegates cannot renew their own authority" spirit for storage lifetime. Accounting invariance is structural (no `set_*` / Cache / pool FFI), not merely tested.

## Call graph

```
Controller::renew_account (lib.rs; no #[when_not_paused], no #[only_owner])
  └─ account::renew_account
       ├─ storage::renew_controller_instance  → instance.extend_ttl only
       ├─ caller.require_auth()
       ├─ require_account_owner               → get_account_meta + NFT owner_of == caller
       ├─ storage::renew_user_account         → extend_ttl on existing AccountMeta /
       │                                         SupplyPositions / BorrowPositions / Delegates
       └─ nft_renew_call → PositionNft::renew → extend_ttl Owner + Balance + instance
```

No `Cache::new`, no `positions::*`, no pool FFI, no events, no spoke-usage touch.

## Owner gate

1. `caller.require_auth()` binds the transaction to the claimed caller.
2. `require_account_owner` loads meta (missing → `AccountNotInMarket` #13) and resolves ownership via `storage::account_owner` → NFT `owner_of` (fail-closed). Caller must equal current NFT owner — **delegates are rejected** (`require_account_owner_rejects_active_delegate`).
3. After NFT transfer, the new holder can renew; the previous owner cannot (`renew_account_follows_current_owner`). Gate tracks live NFT ownership, not stale bookkeeping.

Declared in `scripts/permissionless_entrypoints.txt` as `caller-auth` with `require_account_owner` — not genuinely third-party permissionless.

## TTL-only mutation (no accounting side effects)

| Step | Mechanism | Value write? |
|---|---|---|
| Controller instance | `common::ttl::renew_instance` → `instance.extend_ttl` | No |
| Account keys | `renew_user_key` → `persistent.extend_ttl` iff `has(key)` | No |
| NFT | `extend_user_persistent_ttl` on `Owner(token_id)` + `Balance(owner)`; `renew_instance` | No |

Absent from this path: `set_account_meta`, `set_supply_positions`, `set_debt_positions`, `add_delegate` / `remove_delegate`, `SpokeUsageContext`, pool deposit/withdraw/borrow/repay, token transfers, event publish, flash guard.

Harness confirmation: `test_renew_account_owner_succeeds` renews then asserts unchanged supply balance and account id.

## Pause / halt posture

No `#[when_not_paused]` — intentional under INV-HALT-01 so owners can keep account + NFT Owner legs alive during global pause (threat-model "Availability trade-offs"; endpoints.md pause column `open`).

## Residual notes (not gaps in scope)

- Owner-only (not owner-or-delegate): by design; prevents a delegate from extending lifetime unilaterally.
- Complementary permissionless `position-nft::renew` exists for keepers (INV-STOR-02b); controller path remains owner-gated rent for the paired controller keys + NFT lift in one tx.
- Enumeration NFT keys are not extended here (documented INV-STOR-02d); does not create accounting mutation risk on this entrypoint.

## Tests / verification anchors

| Check | Location |
|---|---|
| Non-owner rejected | `tests/test-harness/tests/controller/account.rs` (`test_renew_account_requires_owner`) |
| Owner succeeds; balances unchanged | same file (`test_renew_account_owner_succeeds`) |
| Follows post-transfer NFT owner | `tests/test-harness/tests/controller/position_nft.rs` (`renew_account_follows_current_owner`) |
| Lifts Owner TTL to protocol window | `tests/test-harness/tests/controller/position_nft_ttl_and_ownership_reads.rs` (`renew_account_and_permissionless_renew_close_the_ttl_gap`) |
| Missing account panics #13 | `contracts/controller/tests/entrypoints.rs` (`renew_account_missing_account_panics`) |
| Owner-only helper rejects delegate | `contracts/controller/tests/helpers/account.rs` (`require_account_owner_rejects_active_delegate`) |
| Co-renews all live account sibling keys | `contracts/controller/tests/storage/account.rs` (`renew_user_account_co_renews_all_live_siblings`) |
| Instance TTL re-extend | `contracts/controller/tests/storage/protocol.rs` (`renew_controller_instance_re_extends_instance_ttl`) |
