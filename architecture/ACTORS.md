# Actors and Privileges

Every actor that interacts with `controller` or `pool`, with their trust level, roles, and entrypoints.

## Privilege Tiers

```
Owner (single Address, two-step transferable)
├── KEEPER role  (granted by Owner; multiple Addresses possible)
├── REVENUE role (granted by Owner)
└── ORACLE role  (granted by Owner)

Controller contract (pool admin, set at pool construction by router)

User (any Address)
├── Account owner (caller == AccountMeta.owner)
├── Liquidator (any user; pays debt + receives collateral + bonus)
└── Flash-loan caller (any user; signs the outer tx)

External contracts
├── Reflector CEX oracle
├── Reflector DEX oracle (optional)
├── Aggregator (swap router)
├── Accumulator (revenue sink)
└── Token SACs
```

## Summary

| Actor | Trust | Auth | Key entrypoints |
|---|---|---|---|
| Owner | Trusted (root) | `#[only_owner]` | `upgrade`, `pause`/`unpause`, `grant_role`/`revoke_role`, `transfer_ownership`/`accept_ownership`, `set_aggregator`, `set_accumulator`, `set_liquidity_pool_template`, `edit_asset_config`, `set_position_limits`, e-mode mgmt, `approve_token_wasm`/`revoke_token_wasm`, `create_liquidity_pool`, `upgrade_pool_params`, `upgrade_pool` |
| KEEPER | Semi-trusted | `#[only_role(caller, "KEEPER")]` | `update_indexes`, `keepalive_shared_state`, `keepalive_accounts`, `keepalive_pools`, `clean_bad_debt`, `update_account_threshold` |
| REVENUE | Semi-trusted | `#[only_role(caller, "REVENUE")]` | `claim_revenue`, `add_rewards` |
| ORACLE | Semi-trusted (high impact) | `#[only_role(caller, "ORACLE")]` | `configure_market_oracle`, `edit_oracle_tolerance`, `disable_token_oracle` |
| Controller (pool admin) | Trusted by pool | `verify_admin(&env)` | All pool-mutating endpoints |
| User / Account owner | Untrusted | `caller.require_auth()` + account-owner check | `supply`, `borrow`, `withdraw`, `repay`, strategy fns |
| Liquidator | Untrusted | `liquidator.require_auth()` | `liquidate` |
| Flash-loan caller / receiver | Untrusted | `caller.require_auth()`; receiver exports `execute_flash_loan` | `flash_loan` |
| Reflector CEX oracle | Trusted (validated) | external call | Spot + TWAP price; staleness + tolerance checked; decimals read on-chain |
| Reflector DEX oracle (optional) | Trusted (validated) | external call | DEX TWAP for `DualOracle` mode |
| Aggregator | Trusted (validated) | external call | Swap router; controller checks balance delta, aggregator enforces `amount_out_min` |
| Accumulator | Trusted (address only) | external call | Revenue sink; tokens forwarded blindly |
| Token SACs | Trusted | external call | `transfer`, `approve`, `balance`; transfer-or-panic |

## Owner

Single Address in `OwnableStorageKey::Owner`. Two-step transfer via `transfer_ownership` then `accept_ownership`; `live_until_ledger` sets TTL on the pending state. `upgrade` pauses before upgrading; `set_position_limits` clamps `[1, 32]`.

Trust: the single Owner anchors protocol trust. No on-chain timelock or multisig is enforced; operator key custody must provide both off-chain.

Operator policy notes:
- `disable_token_oracle` is a single-call kill switch: ORACLE-role compromise or operator error immediately freezes withdrawals for the affected market (liquidations still proceed via `allow_disabled_market_price`). Off-chain multisig MUST gate this endpoint.
- `set_position_limits` takes immediate effect with no two-step. Raising limits to 32/32 has gas implications for liquidations; off-chain change control SHOULD precede every change.
- `approve_token_wasm` is a creation-time gate. `revoke_token_wasm` does NOT stop existing pools from using a token at runtime. If a token's WASM becomes hostile, operator must `pause()` and migrate users to a new market.

## KEEPER role

Time-sensitive maintenance. `update_indexes` bumps `pools_list` TTL and refuses during pause or flash loan. The `keepalive_*` endpoints ignore the `caller` param. `clean_bad_debt` refuses during pause or flash loan and is the only path that mutates `supply_index` downward outside in-liquidation socialization. `update_account_threshold` reprices each account with tight oracle tolerance (`safe = false`).

Threat surface: cannot steal funds directly, but can time bad-debt socialization and selectively propagate threshold updates to delay liquidations.

## REVENUE role

`claim_revenue` refuses during flash loan, sequences per-asset claims, and forwards tokens to `Accumulator`. `add_rewards` is caller-paid; the pool's supply index rises by the reward amount.

Threat surface: can starve the accumulator by never claiming, but cannot redirect funds since Owner fixes the accumulator address.

## ORACLE role

`configure_market_oracle` reads decimals on-chain and transitions market `PendingOracle` then `Active`. `edit_oracle_tolerance` is bounded by `MIN_FIRST_TOLERANCE` and `MAX_LAST_TOLERANCE`. `disable_token_oracle` transitions the market to `Disabled`.

Threat surface: can repoint a market to a controlled oracle or widen tolerance bands, enabling manipulated-price liquidations or borrows. After Owner, this role carries the highest impact.

## Controller (as pool admin)

Pool construction (`pool::__constructor`) sets the controller as `admin`. Every pool-mutating endpoint calls `verify_admin(&env)`.

Gated endpoints: `__constructor`, `supply`, `borrow`, `withdraw`, `repay`, `update_indexes`, `add_rewards`, `flash_loan_begin`, `flash_loan_end`, `create_strategy`, `seize_position`, `claim_revenue`, `update_params`, `upgrade`, `keepalive`.

Views (no auth): `capital_utilisation`, `reserves`, `deposit_rate`, `borrow_rate`, `protocol_revenue`, `supplied_amount`, `borrowed_amount`, `delta_time`, `get_sync_data`.

Trust: pools hold no protocol-level risk knowledge and trust the controller. Compromising the controller compromises every pool.

## User (Account Owner)

The first `supply` call creates the user and allocates a fresh `account_id` from `AccountNonce`. `validation::require_account_owner(env, account, caller)` asserts `account.owner == caller` AND calls `caller.require_auth()`.

| Capability | Endpoint | Notes |
|---|---|---|
| Open / add to position | `supply(caller, account_id, e_mode, assets)` | First call creates the account. |
| Borrow | `borrow(caller, account_id, borrows)` | Validates LTV, HF, cap, silo, e-mode, isolation. |
| Repay (own or others' debt) | `repay(caller, account_id, payments)` | Does NOT enforce account-owner check on the target; anyone can repay anyone. |
| Withdraw | `withdraw(caller, account_id, withdrawals)` | Recomputes HF when borrows remain. |
| Liquidate | `liquidate(liquidator, account_id, debt_payments)` | Caller does not own the account. |
| Initiate flash loan | `flash_loan(caller, asset, amount, receiver, data)` | `receiver` must implement `execute_flash_loan`. |
| Strategy flows | `multiply` / `swap_debt` / `swap_collateral` / `repay_debt_with_collateral` | Routes through the aggregator. |

## Liquidator

A subset of "User"; no special role. Any Address that signs `liquidate` and holds the debt assets qualifies. Chooses which debt assets to repay; receives `seized_collaterals` (base + bonus) while the pool keeps `protocol_fees`. Cannot choose collateral to seize; the protocol seizes proportionally across every collateral the account holds.

## Flash-Loan Caller and Receiver

`caller` signs the outer tx. `receiver` must export `execute_flash_loan(initiator, asset, amount, fee, data)`.

- Pool transfers `amount` to `receiver` in `flash_loan_begin` via host-mediated SAC transfer.
- Controller invokes the receiver's `execute_flash_loan` callback via `env.invoke_contract`.
- Pool calls `tok.transfer(&receiver, &pool_addr, &(amount + fee))` in `flash_loan_end`. Soroban SAC `transfer(from, to, amount)` requires `from.require_auth()` internally.
- The receiver MUST pre-authorize this transfer in its callback (typically via `env.authorize_as_current_contract(...)`). Otherwise the transfer panics with auth-denied.
