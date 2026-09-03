# A004 — Account creation / NFT mint defense on `account_id=0`

- Agent: A004
- Theme: T1 (defensive protections inventory — auth / ownership coupling)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/account.rs:26-114` (`create_account`, `create_account_with`, `load_or_create_account`)
  - `contracts/controller/src/account.rs:161-176` (`remove_account_and_burn_nft`, `cleanup_account_if_empty`)
  - `contracts/controller/src/external/position_nft.rs:4-36` (`nft_mint_call`, `nft_burn_call`, `nft_try_owner_of_call`)
  - `contracts/controller/src/positions/supply.rs:39-102` (`process_supply`, `require_third_party_existing_supply`)
  - `contracts/controller/src/strategies/multiply.rs:38-149` (`process_multiply` → `prepare_multiply_account`)
  - `contracts/controller/src/strategies/flash_position.rs:42-104` (`process_flash_position`)
  - `contracts/controller/src/strategies/migrate_blend.rs:40-76` (`process_migrate_blend`)
  - `contracts/controller/src/positions/liquidation/mod.rs:170-216` (`resolve_seize_receiver` / `Credit(0)`)
  - `contracts/controller/src/storage/account.rs:28-56` (`try_account_owner`, `set_account_meta` — no owner write)
  - `contracts/controller/src/lib.rs:95-107`, `195-223`, `231-259`, `348-372` (entrypoints)
  - `contracts/position-nft/src/contract.rs:55-79` (constructor consumes id 0; `mint` controller-gated)
  - `scripts/permissionless_entrypoints.txt:47-48,52,63,97`
- Defense: `account_id == 0` always mints the position NFT **to the authorizing caller**; controller stores no owner; NFT `mint` is controller-auth only; third-party supply cannot open foreign slots
- Gap: none on foreign-owned creation. Residual (accepted): self-owned account/id spam via repeated `account_id=0`; post-mint NFT transfer can move ownership without recipient auth (standard NFT, documented)
- Impact: a stranger **cannot** create an account owned by a victim. Blast radius of create-spam is finite `u32` id space + rent for the spammer’s own NFTs, not foreign fund control
- Evidence: INV-AUTH-03, INV-STOR-03; tests `supply_mints_nft_with_token_id_equal_account_id`, `emptying_account_burns_nft_and_resupply_mints_fresh_id`, `account_creation_before_nft_deploy_fails_closed`, `poc_single_actor_spams_unbounded_dust_accounts`; Certora `supply_new_slot_requires_owner_or_delegate`
- Opinion: ownership on create is tightly coupled to `caller.require_auth()`. The create sentinel cannot be redirected to a foreign address through any public path audited here.

## 1. Question under audit

When a caller passes `account_id = 0` on supply or strategy entrypoints:

1. Who becomes the account owner?
2. How is that ownership coupled to position-NFT mint?
3. Can a stranger create an account whose owner is someone else (foreign-owned account creation)?

## 2. Create sentinel and NFT id domain

| Fact | Source |
|---|---|
| `account_id == 0` means “create”, never a live account | `load_or_create_account` branch; NFT constructor consumes token id 0 |
| First real account id is `1` | `position-nft` `__constructor` calls `sequential::increment_token_id(e, 1)` |
| `account_id == token_id` (widened `u32` → `u64`) | `nft_mint_call`; architecture / position-nft README |
| Ids never reuse after burn | sequential mint; harness `emptying_account_burns_nft_and_resupply_mints_fresh_id` |
| Ids above `u32::MAX` cannot have been minted | `nft_burn_call` / `nft_try_owner_of_call` narrow with `u32::try_from` → fail closed |

So the create path never collides with an existing account id: `0` is reserved and burned at NFT construction time.

## 3. Core coupling: create → mint(caller) → meta only

### 3.1 `load_or_create_account`

```97:99:contracts/controller/src/account.rs
    if account_id == 0 {
        return create_account(env, caller, spoke_id, mode, cache);
    }
```

There is **no** separate `owner` / `to` argument on this helper. The create branch hard-wires `owner = caller`.

### 3.2 `create_account` / `create_account_with`

Sequence (ActiveOnly for user paths):

1. Reject `spoke_id < 1` (`SpokeNotFound`).
2. Require spoke active (`cache.active_spoke`) — except liquidation `Credit(0)`, which uses `AllowDeprecated`.
3. `nft = storage::get_position_nft(env)` — unset NFT → `PositionNftNotSet` (#53), fail closed.
4. `account_id = nft_mint_call(env, &nft, owner)` → NFT `mint(to)` with `to = owner = caller`.
5. Persist **only** `AccountMeta { spoke_id, mode }` via `set_account_meta`.
6. Return in-memory `Account { owner, spoke_id, mode, empty maps }`.

Critical storage fact: the controller **never writes an owner address**. Live ownership is always `position_nft.owner_of(account_id)` via `try_account_owner` / `account_owner` (`storage/account.rs`). The `Account.owner` field on create is ephemeral scaffolding for the rest of the same invocation; subsequent loads re-resolve from the NFT.

### 3.3 FFI: `nft_mint_call`

```7:8:contracts/controller/src/external/position_nft.rs
pub(crate) fn nft_mint_call(env: &Env, nft: &Address, to: &Address) -> u64 {
    u64::from(PositionNftClient::new(env, nft).mint(to))
}
```

NFT side (`contracts/position-nft/src/contract.rs:72-79`):

- `controller(e).require_auth()` — only the construction-time controller may mint.
- Sequential mint to `to`.
- Extends per-user TTL on the new `Owner` / `Balance` entries.

A third party invoking `position-nft::mint` directly fails auth (documented in `scripts/permissionless_entrypoints.txt:97` and NFT unit tests `mint_requires_controller_auth`).

### 3.4 Burn pairing (INV-STOR-03)

Every deletion goes through `remove_account_and_burn_nft`: remove controller keys, then `nft_burn_call`. Empty cleanup and bad-debt socialization both use that path. Meta-without-NFT and NFT-without-meta are not reachable through public create/delete flows (harness asserts both sides die together).

## 4. Call sites that create on `account_id == 0`

| Entrypoint | Guard | Mode on create | Owner on create | Auth before create |
|---|---|---|---|---|
| `supply` | `AccountGuard::Supply` | forced `PositionMode::Normal` | `caller` | `require_authorized_caller` (`caller.require_auth` + not flash-loaning) |
| `multiply` | `AccountGuard::Multiply` | caller-chosen (`Multiply`/`Long`/`Short`, validated) | `caller` | same |
| `flash_position` | `AccountGuard::Multiply` | caller-chosen mode | `caller` | same |
| `migrate_from_blend` | `AccountGuard::Migrate` | `PositionMode::Normal` | `caller` | same |
| `liquidate` / `SeizeMode::Credit(0)` | N/A (direct `create_account_with`) | `Normal` | `liquidator` | liquidator auth on liquidate path |

Non-creating strategies (`swap_debt`, `swap_collateral`, `repay_debt_with_collateral`) load existing accounts only — no `load_or_create_account`, no mint.

### 4.1 Supply path detail

`process_supply`:

1. `require_authorized_caller(caller)`.
2. Aggregate positive payments.
3. `load_or_create_account(..., AccountGuard::Supply)`.
4. `require_third_party_existing_supply` — **skipped when the input `account_id` was 0**, because the caller is now the owner (`supply.rs:80-101`).
5. Deposit / finalize; return fresh id.

Comments and `docs/reference/endpoints.md` match the code: “New accounts skip the check because the caller becomes the owner.”

For **existing** accounts (`account_id != 0`), Supply guard only enforces spoke match — not owner. Third parties may top up **existing** supply slots only; opening a new hub-asset slot as a non-owner/non-delegate panics `NotAuthorized`. That is INV-AUTH-03 (foreign risk / slot consumption), not create-path ownership.

### 4.2 Strategy path detail

Multiply / flash_position / migrate:

- `account_id == 0` → create owned by caller, with the path’s mode / spoke.
- `account_id != 0` → `require_owner_or_delegate` (Migrate and Multiply guards). Strangers cannot open leverage or migration into someone else’s existing account.

So strategy create is the same ownership rule as supply create: **self-owned only**.

### 4.3 Liquidation `Credit(0)` (adjacent create)

Not in the A004 supply/strategy scope, but it is the only other create site: mint goes to `liquidator`, not the liquidated user. Still not foreign-owned relative to the authorizing party.

## 5. Can a stranger create foreign-owned accounts?

### 5.1 Direct answer: **No**

To mint an account owned by `Victim`:

- Public create paths always pass `owner = caller` into `create_account`.
- `caller` must `require_auth()` before create on supply/strategies.
- Therefore the NFT `to` address is always an address that authorized this invocation.
- There is no ABI field to specify a different mint recipient.

Attack attempts and why they fail:

| Attempt | Result |
|---|---|
| Mallory calls `supply(caller=Mallory, account_id=0, ...)` | Account owned by Mallory |
| Mallory calls `supply(caller=Alice, account_id=0, ...)` without Alice auth | Reverts at `require_auth` |
| Mallory calls `supply(caller=Alice, account_id=0, ...)` with Alice auth | Alice consented; Alice owns the NFT |
| Mallory calls `position_nft.mint(Alice)` | Reverts — not controller |
| Mallory supplies to Alice’s **existing** id with a **new** hub asset | `NotAuthorized` (INV-AUTH-03 / Certora rule) |
| Mallory supplies to Alice’s existing id on an **existing** supply asset | Allowed top-up; does **not** mint; does **not** change owner |

### 5.2 What is *not* “foreign create” but looks adjacent

1. **Third-party top-up** — adds collateral to an existing foreign account without minting. Owner unchanged. Constrained to existing supply keys.
2. **Post-mint NFT transfer** — Mallory creates (self-owned), then `transfer`/`transfer_from` to Alice. Recipient need not authorize (OZ NFT). Alice receives ownership of Mallory’s position (and any debt). This is **transfer semantics**, not create-time foreign mint. Threat model documents it explicitly; controller does not gate transfer.
3. **Approve / approve_for_all phishing** — hands transfer right over a live account. Ownership change, not create.
4. **Account/id spam** — Mallory repeatedly calls with `account_id=0` and dust deposits, consuming sequential `u32` ids. Accounts are owned by Mallory. Threat model residual: exhaustion of the id space; harness `poc_single_actor_spams_unbounded_dust_accounts` documents unbounded creation cost to the spammer, not foreign ownership takeover.

## 6. Auth ordering (create window)

Supply / multiply / flash_position / migrate all call `require_authorized_caller` **before** `load_or_create_account`. Order:

1. Caller authorization + flash-loan guard.
2. (Strategies) request validation.
3. Create → NFT mint under controller auth (Soroban auth context includes the controller contract as the mint caller).
4. Position mutations financed by the same authorizing caller (measured transfers from `caller`, or strategy debt booked to the new self-owned account).

There is no window where an account meta exists without an NFT owner, or an NFT is minted to an address that did not authorize the enclosing controller call on these paths.

## 7. Spoke / mode binding on create

- Spoke is bound once at create (`INV-AUTH-06`); `AccountGuard::Supply` / Migrate / Multiply re-check spoke match on load.
- Supply create always `PositionMode::Normal`.
- Multiply/flash create bind the caller-chosen strategy mode; on later calls Multiply guard asserts `account.mode == mode`.
- Deprecated spokes: user create uses `ActiveOnly` (rejected). Liquidation credit create may use `AllowDeprecated` — still owned by liquidator.

None of these allow binding a foreign user’s identity into a new account.

## 8. Invariants and documentation cross-check

| Artifact | Claim | Code match |
|---|---|---|
| INV-AUTH-03 | Permissionless actions do not create foreign risk; third-party supply only tops up existing slots | `require_third_party_existing_supply`; create skips because caller is owner |
| INV-STOR-03 | Account ↔ NFT paired create/destroy | `create_account` mint + `remove_account_and_burn_nft` |
| `permissionless_entrypoints.txt` supply line | `account_id 0` creates account owned by caller | Exact |
| `endpoints.md` supply | mint NFT to `caller`, no separate create entrypoint | Exact |
| `architecture.md` / NFT README | controller stores no owner; `owner_of` is authority | Exact |
| Threat model NFT section | transfer ungated; approve is full account handover | Out of create scope; residual |

## 9. Test / formal evidence map

| Evidence | What it shows |
|---|---|
| `tests/.../position_nft.rs::supply_mints_nft_with_token_id_equal_account_id` | First supply with create sentinel → NFT owner == supplier; id ≥ 1 |
| `emptying_account_burns_nft_and_resupply_mints_fresh_id` | Burn + remint new id; meta and NFT die together |
| `unknown_and_unmintable_account_ids_are_account_not_found` | Non-zero unknown / `> u32::MAX` fail closed (not create) |
| `contracts/controller/tests/helpers/account.rs::account_creation_before_nft_deploy_fails_closed` | No NFT address → `#53`, no orphan meta |
| `tests/.../supply.rs::test_third_party_supply_to_existing_account_succeeds` | Top-up allowed without ownership change |
| Certora `supply_new_slot_requires_owner_or_delegate` | Non-owner cannot open a new supply slot on existing account |
| `poc_single_actor_spams_unbounded_dust_accounts` | Create spam possible but self-owned |
| NFT contract tests `mint_requires_controller_auth` | External mint blocked |

## 10. Residual risks (not foreign-create bugs)

| Residual | Severity for A004 scope | Notes |
|---|---|---|
| Sequential `u32` id exhaustion via self-owned dust creates | Accepted / documented | Threat model; exporter should publish next id |
| NFT transfer / approval moves whole position without controller gate | Accepted / documented | Not a create-path ownership redirect |
| Liquidation `Credit(0)` creates liquidator-owned account even on deprecated spoke | By design | Owner still = authorizing liquidator |

## 11. Verdict

**Defended.** On every supply and strategy path that honors `account_id = 0`, the authorizing `caller` becomes the sole owner via an immediate controller-gated NFT mint to that same address. A stranger cannot mint an account into a victim’s name. Related INV-AUTH-03 defenses correctly treat create as self-ownership and separately constrain third-party mutation of existing accounts.

No remediation required for foreign-owned account creation. Peer agents covering third-party supply slots (A012) and NFT lifecycle storage (A031) should treat this finding as agreeing on create-time ownership = caller.
