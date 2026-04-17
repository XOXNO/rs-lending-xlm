# Actors and Privileges

Every actor that interacts with `controller` or `pool`, what they do, and how the auth check enforces it.

## Privilege Tiers

```
Owner (single Address, two-step transferable)
├── KEEPER role  (granted by Owner; multiple Addresses possible)
├── REVENUE role (granted by Owner)
└── ORACLE role  (granted by Owner)

Controller contract (as pool admin, set at pool construction by router)

User (any Address)
├── Account owner (caller == AccountMeta.owner)
├── Liquidator (any user; pays debt + receives collateral + bonus)
└── Flash-loan caller (any user; signs the outer tx)

External contracts (called by controller)
├── Reflector CEX oracle
├── Reflector DEX oracle (optional)
├── Aggregator (swap router; per-network address)
├── Accumulator (revenue sink)
└── Token SACs (asset contracts)
```

## Owner

Single Address held in `OwnableStorageKey::Owner`. Two-step transfer via `transfer_ownership` → `accept_ownership`; `live_until_ledger` sets TTL on the pending state.

| Capability | Endpoint | Enforcement |
|---|---|---|
| Upgrade controller WASM | `upgrade(new_wasm_hash)` | `#[only_owner]`; pauses before upgrade |
| Pause / unpause | `pause()` / `unpause()` | `#[only_owner]` |
| Grant / revoke any role | `grant_role(account, role)` / `revoke_role(...)` | `#[only_owner]` |
| Transfer ownership | `transfer_ownership(new_owner, live_until_ledger)` | `#[only_owner]` |
| Set aggregator / accumulator / pool template | `set_aggregator` / `set_accumulator` / `set_liquidity_pool_template` | `#[only_owner]` |
| Edit asset config | `edit_asset_config(asset, cfg)` | `#[only_owner]` |
| Set position limits | `set_position_limits(limits)` | `#[only_owner]`; clamped `[1, 32]` |
| Manage e-mode categories | `add_e_mode_category` / `edit_e_mode_category` / `remove_e_mode_category` | `#[only_owner]` |
| Manage e-mode asset membership | `add_asset_to_e_mode_category` / `edit_asset_in_e_mode_category` / `remove_asset_from_e_mode` | `#[only_owner]` |
| Approve / revoke token wasm allowlist | `approve_token_wasm` / `revoke_token_wasm` | `#[only_owner]` |
| Create liquidity pool (deploy from template) | `create_liquidity_pool(asset, params, config)` | `#[only_owner]` |
| Upgrade pool params (rate model) | `upgrade_pool_params(asset, ...)` | `#[only_owner]` |
| Upgrade pool WASM | `upgrade_pool(asset, new_wasm_hash)` | `#[only_owner]` |

**Trust assumption**: the single Owner anchors protocol trust. Compromising the Owner key compromises everything. The contract enforces no timelock and no multisig — operator key custody must enforce both off-chain.

## KEEPER role

Owner grants this role; multiple Addresses may hold it. Time-sensitive maintenance requires it.

| Capability | Endpoint | Enforcement | Notes |
|---|---|---|---|
| Sync market indexes | `update_indexes(caller, assets)` | `#[only_role(caller, "KEEPER")]` | Bumps `pools_list` TTL; refuses during pause or flash loan. |
| Bump shared-state TTL | `keepalive_shared_state(caller, assets)` | `#[only_role(caller, "KEEPER")]` | Ignores `caller` param (`let _ = caller`). |
| Bump per-account TTL | `keepalive_accounts(caller, account_ids)` | `#[only_role(caller, "KEEPER")]` | Ignores `caller` param. |
| Bump pool TTL | `keepalive_pools(caller, assets)` | `#[only_role(caller, "KEEPER")]` | Ignores `caller` param. |
| Clean bad debt | `clean_bad_debt(caller, account_id)` | `#[only_role(caller, "KEEPER")]` | Refuses during pause or flash loan. **The only path that mutates `supply_index` downward outside in-liquidation socialization.** |
| Propagate threshold updates | `update_account_threshold(caller, asset, has_risks, account_ids)` | `#[only_role(caller, "KEEPER")]` | Reprices each account using tight oracle tolerance (`safe = false`). |

**Threat surface**: a malicious KEEPER cannot steal funds directly, but can:
- Trigger bad-debt socialization at a chosen ledger, timing the supply-index drop.
- Select which accounts to update, delaying liquidations through selective threshold propagation.

## REVENUE role

Owner grants this role. Manages protocol-owned revenue and reward distribution.

| Capability | Endpoint | Enforcement | Notes |
|---|---|---|---|
| Claim revenue from pools to accumulator | `claim_revenue(caller, assets)` | `#[only_role(caller, "REVENUE")]` | Refuses during flash loan; sequences per-asset claims; forwards tokens to `Accumulator`. |
| Add rewards to a pool | `add_rewards(caller, rewards)` | `#[only_role(caller, "REVENUE")]` | Caller pays; the pool's supply index rises by the reward amount. |

**Threat surface**: a malicious REVENUE holder can starve the accumulator by never claiming, but cannot redirect funds — Owner fixes the accumulator address.

## ORACLE role

Owner grants this role. Manages per-market oracle wiring.

| Capability | Endpoint | Enforcement | Notes |
|---|---|---|---|
| Configure market oracle | `configure_market_oracle(caller, asset, cfg)` | `#[only_role(caller, "ORACLE")]` | Reads decimals on-chain; transitions market `PendingOracle` → `Active`. |
| Edit tolerance bands | `edit_oracle_tolerance(caller, asset, first, last)` | `#[only_role(caller, "ORACLE")]` | Constants `MIN_FIRST_TOLERANCE` and `MAX_LAST_TOLERANCE` bound the values. |
| Disable a token's oracle | `disable_token_oracle(caller, asset)` | `#[only_role(caller, "ORACLE")]` | Transitions market to `Disabled`. |

**Threat surface**: a malicious ORACLE holder can repoint a market to a controlled oracle contract or widen tolerance bands, enabling manipulated-price liquidations or borrows. **After Owner, this role carries the highest impact.**

**Operator policy notes (audit-prep)**:
- `disable_token_oracle` is a **single-call kill switch**. ORACLE-role compromise or operator error immediately freezes withdrawals for the affected market (only liquidations proceed via `allow_disabled_market_price`). The contract enforces no two-step. Off-chain operator multisig MUST gate this endpoint. Finding M-01.
- `set_position_limits` (Owner-only) takes immediate effect with no two-step or rate limit. Raising the limit to 32/32 has gas-footprint implications for liquidations (THREAT_MODEL §3.3). Off-chain change control SHOULD precede every change. Finding M-11.
- `approve_token_wasm` is a **creation-time gate**. `revoke_token_wasm` does NOT stop existing pools from using the token at runtime — Soroban exposes no `code_hash(addr)`. If a token's WASM goes hostile after listing, operator must `pause()` and migrate users to a new market backed by a different token contract. Finding M-12.

## Controller (as pool admin)

Pool construction (`pool::__constructor`) sets the controller as `admin`. Every pool-mutating endpoint calls `verify_admin(&env)`, which asserts the caller matches the controller's address.

Pool endpoints gated by `verify_admin`:

`__constructor`, `supply`, `borrow`, `withdraw`, `repay`, `update_indexes`, `add_rewards`, `flash_loan_begin`, `flash_loan_end`, `create_strategy`, `seize_position`, `claim_revenue`, `update_params`, `upgrade`, `keepalive`.

Pool views (no auth): `capital_utilisation`, `reserves`, `deposit_rate`, `borrow_rate`, `protocol_revenue`, `supplied_amount`, `borrowed_amount`, `delta_time`, `get_sync_data`.

**Trust assumption**: pools hold no protocol-level risk knowledge; they trust the controller's instructions. Compromising the controller compromises every pool.

## User (Account Owner)

The first `supply` call creates the user, allocating a fresh `account_id` from `AccountNonce`. The Address in `AccountMeta.owner` identifies them.

Before mutating an account, `validation::require_account_owner(env, account, caller)` asserts `account.owner == caller` AND calls `caller.require_auth()`.

| Capability | Endpoint | Auth | Notes |
|---|---|---|---|
| Open / add to position | `supply(caller, account_id, e_mode, assets)` | `caller.require_auth()` | The first call creates the account; later calls add or top up. |
| Borrow | `borrow(caller, account_id, borrows)` | account-owner check | Validates LTV, HF, cap, silo, e-mode, and isolation. |
| Repay (own or others' debt) | `repay(caller, account_id, payments)` | repay does **NOT** enforce the account-owner check on the *target* account; anyone can repay anyone | Decrements isolated debt by the amount actually applied. |
| Withdraw | `withdraw(caller, account_id, withdrawals)` | account-owner check | Recomputes HF when borrows remain. |
| Liquidate (someone else's account) | `liquidate(liquidator, account_id, debt_payments)` | `liquidator.require_auth()` | The liquidator does not own the account; pays debt and receives collateral. |
| Initiate flash loan | `flash_loan(caller, asset, amount, receiver, data)` | `caller.require_auth()` | `receiver` must implement the `execute_flash_loan` callback. |
| Multiply / swap_debt / swap_collateral / repay_debt_with_collateral | (strategy fns) | `caller.require_auth()` (strategy.rs:53) | Routes through the aggregator. |

**Open question**: confirm that `repay` intentionally lets anyone repay anyone — many protocols allow this, but document the intent.

## Liquidator

A subset of "User"; no special role. Any Address that signs a `liquidate` tx and holds the debt assets qualifies.

Privileges:
- Choose which debt assets to repay (a subset of the account's debts).
- Receive `seized_collaterals` (base + bonus); pool keeps `protocol_fees`.
- Cannot choose collateral assets to seize — the protocol seizes proportionally across every collateral asset the account holds.

## Flash-Loan Caller and Receiver

`caller` signs the outer tx. `receiver` names a contract address that must export `execute_flash_loan(initiator, asset, amount, fee, data)`.

Mechanic (verified pool/lib.rs:353):
- Pool transfers `amount` to `receiver` in `flash_loan_begin` via host-mediated SAC transfer.
- Controller invokes the receiver's `execute_flash_loan` callback through `env.invoke_contract`.
- Pool calls `tok.transfer(&receiver, &pool_addr, &(amount + fee))` in `flash_loan_end` (pool/lib.rs:353). This is plain `transfer`, not ERC-20 `transfer_from`/`approve`. Soroban SAC `transfer(from, to, amount)` requires `from.require_auth()` internally.
- **The receiver MUST pre-authorize this transfer in its `execute_flash_loan` callback** — typically via `env.authorize_as_current_contract(...)` for the upcoming SAC `transfer` invocation. Otherwise line 353 panics with auth-denied.

This follows the Soroban-native pattern, not the EVM allowance model.

## External Contracts

| Address | Role | Trust assumption |
|---|---|---|
| Reflector CEX oracle (`cex_oracle` per market) | Spot + TWAP price source | Trusted to return `(price, timestamp)`; protocol validates staleness + tolerance bands; **reads decimals on-chain** |
| Reflector DEX oracle (`dex_oracle` per market, optional) | DEX TWAP for `DualOracle` mode | Same as above; optional |
| Aggregator (`Aggregator` storage key) | Swap router for strategy flows | Controller checks its OWN `balance` before and after the call (verified strategy.rs:456-457, 481, 496); enforces spend ≤ `amount_in`. **The AGGREGATOR enforces the slippage minimum (`amount_out_min`) at the DEX level; the controller does not re-verify it** (strategy.rs:472) — the operator-set aggregator must honor it. |
| Accumulator (`Accumulator` storage key) | Revenue sink | Pool forwards tokens blindly; trust limited to "address exists" |
| Token SAC (per asset) | ERC-20 equivalent on Stellar | Trusted to honor `transfer` / `approve` / `balance`; pool relies on transfer-or-panic semantics |
