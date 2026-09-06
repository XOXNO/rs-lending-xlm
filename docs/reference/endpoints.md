# Endpoint reference

This reference lists the callable interfaces and authorization rules for the eight protocol contracts. For component responsibilities, start with [architecture](architecture.md). Use [errors](errors.md) to interpret failures and [events](events.md) to decode transaction output.

Soroban supplies `Env`, so signatures omit it. Constructors run at deployment; test, mock and Certora hooks are excluded. Signatures use Rust ABI types. Named contract structs encode as maps; tuple structs encode as vectors.

| Value | Unit |
| --- | --- |
| Amounts | Raw token units, using the token's decimals |
| USD values and health factors | WAD (`10^18`) |
| Shares, indexes and annual rates | RAY (`10^27`) |
| Basis points (BPS) | `10,000` represents 100% |

`HubAssetKey` is `{hub_id: u32, asset: Address}`. The same token in two hubs represents two markets.

“Owner” means the contract owner. “NFT owner/delegate” means the position's current NFT owner or an active registered manager delegated by that owner. Open views require no signature, but can renew storage time to live (TTL) and invoke external contracts.

## Controller

Every user mutation below requires the named caller, payer or liquidator to authorize. In the Pause column, “gated” requires an unpaused controller; “open” remains callable during global pause. Both can still enforce asset-level restrictions. Monetary calls reject execution during a flash callback; account renewal and delegate writes do not use that guard.

| Endpoint (parameters → return) | Additional authority | Pause | Effect |
| --- | --- | --- | --- |
| `supply(caller: Address, account_id: u64, spoke_id: u32, assets: Vec<(HubAssetKey, i128)>) -> u64` | Existing assets only for third parties | gated | Supply measured deposits; id 0 creates Normal account. |
| `borrow(caller: Address, account_id: u64, borrows: Vec<(HubAssetKey, i128)>, to: Option<Address>)` | NFT owner/delegate | gated | Debt booked to account; recipient defaults to caller. |
| `withdraw(caller: Address, account_id: u64, withdrawals: Vec<(HubAssetKey, i128)>, to: Option<Address>) -> Vec<(HubAssetKey, i128)>` | NFT owner/delegate | open | Zero means full withdrawal; returns resolved amounts. |
| `repay(caller: Address, account_id: u64, payments: Vec<(HubAssetKey, i128)>)` | None | open | Anyone can repay; excess returns to caller. |
| `liquidate(liquidator: Address, account_id: u64, debt_payments: Vec<(HubAssetKey, i128)>, seize_mode: SeizeMode) -> u64` | None; credit receiver owner/delegate | open | Pro-rata seizure; Transfer returns 0, Credit returns receiver id. |
| `clean_bad_debt(caller: Address, account_id: u64)` | None | open | Debt exceeds collateral and collateral <= $5; socialize and burn NFT. |
| `flash_loan(caller: Address, asset: HubAssetKey, amount: i128, receiver: Address, data: Bytes)` | None | gated | Wasm callback; pool pulls exact principal plus fee. |
| `flash_position(caller: Address, account_id: u64, spoke_id: u32, mode: PositionMode, debt: HubAssetKey, amount: i128, receiver: Address, data: Bytes, collaterals: Vec<(HubAssetKey, i128)>, refund_assets: Vec<Address>) -> u64` | NFT owner/delegate for existing id | gated | Mint zero-fee debt; callback returns declared collateral deltas. |
| `multiply(caller: Address, account_id: u64, spoke_id: u32, collateral: HubAssetKey, debt_to_flash_loan: i128, debt: HubAssetKey, mode: PositionMode, swap: Bytes, initial_payment: Option<(HubAssetKey, i128)>, convert_swap: Option<Bytes>) -> u64` | NFT owner/delegate for existing id | gated | Borrow, swap and supply; optional initial capital. |
| `swap_debt(caller: Address, account_id: u64, existing_debt: HubAssetKey, amount: i128, new_debt: HubAssetKey, swap: Bytes)` | NFT owner/delegate | gated | Borrow new debt before repaying existing debt with swap output; the borrow-first order can prevent refinancing at a cap. |
| `swap_collateral(caller: Address, account_id: u64, current: HubAssetKey, amount: i128, new: HubAssetKey, swap: Bytes)` | NFT owner/delegate | gated | Withdraw, convert and redeposit collateral. |
| `repay_debt_with_collateral(caller: Address, account_id: u64, collateral: HubAssetKey, collateral_amount: i128, debt: HubAssetKey, swap: Bytes, close_position: bool)` | NFT owner/delegate | gated | Direct same-market netting or swap; optional full close. |
| `migrate_from_blend(caller: Address, account_id: u64, spoke_id: u32, hub_id: u32, blend_pool: Address, collateral_assets: Vec<Address>, supply_assets: Vec<Address>, debt_caps: Vec<(Address, i128)>) -> u64` | NFT owner/delegate for existing id | gated | Migrate caller’s position from approved Blend pool. |
| `update_indexes(caller: Address, assets: Vec<HubAssetKey>)` | None | gated | Accrue specified markets. |
| `claim_revenue(caller: Address, assets: Vec<HubAssetKey>) -> Vec<i128>` | None | gated | Pay only configured accumulator; return controller receipts. |
| `update_account_threshold(caller: Address, has_risks: bool, account_ids: Vec<u64>)` | None | gated | Refresh LTV; optional risk refresh requires final HF >= 1.05. |
| `recapitalize(payer: Address, hub_asset: HubAssetKey, amount: i128) -> i128` | None | open | Measured backing injection; refund surplus; return amount applied. |
| `renew_account(caller: Address, account_id: u64)` | NFT owner | open | Renew account and NFT ownership storage. |
| `add_delegate(caller: Address, account_id: u64, delegate: Address)` | NFT owner | gated | Grant only to active approved manager; maximum 16. |
| `remove_delegate(caller: Address, account_id: u64, delegate: Address)` | NFT owner | open | Revoke grant. |

### Accounts and authorization

Account id `0` creates an account on `supply`, `multiply`, `flash_position`, `migrate_from_blend`, and liquidation with `Credit(0)`. An account's spoke binding is permanent. `multiply` and `flash_position` require Multiply, Long or Short mode, and an existing account must match the requested mode. Blend migration creates a Normal account; an existing destination need not be Normal.

Delegates belong to the granting owner. An NFT transfer disables that owner's grants; a transfer back can reactivate them unless another owner has rewritten the list. NFT ownership, including control of collateral and the debt obligation, transfers atomically.

Ordinary payment lists sum duplicate markets in first-appearance order and reject negative amounts. `supply`, `borrow` and `repay` reject zero. In `withdraw`, zero means the full balance and overrides positive amounts for the same market. Flash-position declaration lists reject duplicates.

### Risk checks and pause flags

After pool accounting, `borrow`, `withdraw` and the six account strategies require LTV-weighted collateral to cover debt and health factor (HF) to be at least 1. If debt remains, LTV-weighted collateral must also meet the minimum-borrow floor. Ordinary `supply` and `repay` skip these checks; repayment loads debt positions only. Liquidation uses separate admission and sizing rules. See [formulas](formulas.md) for the calculations.

| Listing flag | Restriction |
| --- | --- |
| `paused` | Blocks ordinary entry and exit, including liquidation debt repayment |
| `frozen` | Blocks entry |
| `no_seize` | Blocks collateral seizure |

Seizure checks only `no_seize`. Liquidators cannot choose a different collateral subset. Global pause leaves withdrawal, repayment, liquidation, bad-debt cleanup and recapitalization callable, subject to their other checks.

### Liquidation results

Credit liquidation transfers supply shares without moving collateral tokens. The receiver must differ from the target, share its spoke and use Normal mode. `Credit(0)` can create a receiver in a deprecated spoke. Position limits apply; collateral permissions and supply caps do not gate this credit.

Liquidation estimates report gross seizure before protocol fees. Transfer-mode seizure and fees use token units; Credit-mode seizure and fees use RAY shares. `refunds` use debt-token units. Execution can differ from an estimate because prices, indexes and measured token delivery can change.

### Strategy payments and callbacks

For `multiply`, an initial payment in collateral joins the supply; a payment in debt joins the main swap. Another payment asset requires `convert_swap`. Same-market `repay_debt_with_collateral` requires empty swap bytes. With `close_position`, any remaining debt rejects the call; otherwise, the remaining collateral is withdrawn to the caller.

`flash_position` requires a deployed Wasm receiver other than the controller or pool. Its collateral declarations must meet all of these conditions:

- The list is nonempty and does not exceed the maximum supply-position count.
- Markets and underlying tokens are unique.
- All minimum amounts are nonnegative, with at least one positive minimum.
- Measured controller receipts from the callback meet every minimum.

Pool supply measures receipts again. Caps are checked when the callback's pool changes merge into account accounting. Supply and the declared debt position must remain open after finalization.

Refund assets must be unique, listed in the debt hub and account spoke, disjoint from collateral declarations, and bounded by the maximum supply-position count. Refund eligibility requires an active spoke and an existing listing; it does not check collateralizable, borrowable, paused or frozen flags. Only positive balance changes above pre-callback balances return to the caller. The debt token can be a refund asset, but refunding it does not repay the minted debt.

Undeclared callback assets receive neither credit nor refunds. There is no controller sweep endpoint. Refunds produce token transfer events, without a dedicated controller refund event.

### Revenue and recapitalization

`claim_revenue` forwards the controller's measured receipts to the configured accumulator. A taxed onward transfer can deliver less to the accumulator. Pool revenue measures outstanding claims, not cumulative earnings. Recapitalization applies no more than the market's backing shortfall and refunds excess.

### Controller views

| View | Result / semantics |
| --- | --- |
| `is_liquidatable(account_id: u64) -> bool` | HF < 1 WAD |
| `get_health_factor(account_id: u64) -> i128` | HF WAD; i128::MAX for missing/debt-free account |
| `get_total_collateral_usd(account_id: u64) -> i128` | Unweighted USD WAD |
| `get_total_borrow_usd(account_id: u64) -> i128` | Debt USD WAD |
| `get_collateral_amount(account_id: u64, hub_asset: HubAssetKey) -> i128` | Underlying token units; zero without position |
| `get_borrow_amount(account_id: u64, hub_asset: HubAssetKey) -> i128` | Underlying token units; zero without position |
| `get_account_positions(account_id: u64) -> ( Map<HubAssetKey, AccountPositionRaw>, Map<HubAssetKey, DebtPositionRaw>, )` | Supply/debt maps; empty if missing |
| `get_account_attributes(account_id: u64) -> AccountAttributes` | Stored spoke and mode |
| `account_exists(account_id: u64) -> bool` | Metadata existence |
| `get_liquidation_estimate(account_id: u64, debt_payments: Vec<(HubAssetKey, i128)>, seize_mode: SeizeMode) -> LiquidationEstimate` | Shared liquidation plan; max 256 payment inputs |
| `get_liquidation_collateral(account_id: u64) -> i128` | Threshold-weighted USD WAD; not a seizure ceiling |
| `get_ltv_collateral_usd(account_id: u64) -> i128` | LTV-weighted USD WAD |
| `get_pool_address() -> Address` | Configured pool |
| `get_market_index(hub_asset: HubAssetKey) -> MarketIndexRaw` | Simulated accrued RAY indexes |
| `get_market_indexes_detailed(hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexView>` | Accrued indexes and oracle status; at most 256 inputs |
| `get_spoke(spoke_id: u32) -> SpokeConfig` | Spoke config |
| `get_spoke_asset(spoke_id: u32, hub_asset: HubAssetKey) -> SpokeAssetConfig` | Listed risk config; fails if missing |
| `get_spoke_usage(spoke_id: u32, hub_asset: HubAssetKey) -> SpokeUsageRaw` | RAY shares; default zero if absent |
| `price_aggregator() -> Address` | Configured price aggregator |
| `get_min_borrow_collateral_usd() -> i128` | LTV-weighted collateral floor WAD |
| `is_blend_pool_approved(pool: Address) -> bool` | Migration allowlist |

### Controller administration

Constructor `(admin: Address)` initializes the controller and starts it paused. Administration requires the contract owner, normally governance, except that `accept_ownership` authenticates the pending owner. `get_app_version` is open. The controller exports only the ownership methods listed here.

| Endpoint |
| --- |
| `set_swap_aggregator(addr: Address)` |
| `set_price_aggregator(addr: Address)` |
| `set_accumulator(addr: Address)` |
| `set_position_limits(limits: PositionLimits)` |
| `set_min_borrow_collateral_usd(floor_wad: i128)` |
| `set_position_manager(manager: Address, is_active: bool)` |
| `approve_blend_pool(pool: Address)` |
| `revoke_blend_pool(pool: Address)` |
| `create_hub() -> u32` |
| `add_spoke() -> u32` |
| `remove_spoke(id: u32)` |
| `set_spoke_liquidation_curve(id: u32, target_hf_wad: i128, hf_for_max_bonus_wad: i128, liquidation_bonus_factor_bps: u32)` |
| `add_asset_to_spoke(input: SpokeAssetArgs)` |
| `edit_asset_in_spoke(input: SpokeAssetArgs)` |
| `set_spoke_asset_flags(spoke_id: u32, hub_asset: HubAssetKey, paused: bool, frozen: bool, no_seize: bool)` |
| `remove_asset_from_spoke(hub_asset: HubAssetKey, spoke_id: u32)` |
| `deploy_pool(wasm_hash: BytesN<32>) -> Address` |
| `deploy_position_nft(wasm_hash: BytesN<32>, uri: String, name: String, symbol: String) -> Address` |
| `create_liquidity_pool(hub_id: u32, asset: Address, params: MarketParamsRaw) -> Address` |
| `upgrade_liquidity_pool_params(hub_asset: HubAssetKey, params: InterestRateModel)` |
| `upgrade_pool(new_wasm_hash: BytesN<32>)` |
| `upgrade_position_nft(new_wasm_hash: BytesN<32>)` |
| `force_socialize_bad_debt(account_id: u64)` |
| `pause()` |
| `unpause()` |
| `upgrade(new_wasm_hash: BytesN<32>)` |
| `migrate(new_version: u32)` |
| `get_app_version() -> u32` |
| `transfer_ownership(new_owner: Address, live_until_ledger: u32)` |
| `accept_ownership()` |

`force_socialize_bad_debt` requires debt greater than unweighted collateral and bypasses the $5 collateral cap. Follow the [force-socialize runbook](runbooks/force-socialize-bad-debt.md) before scheduling this irreversible operation.

`set_spoke_asset_flags` can set flags but cannot clear them; timelocked listing edits can clear them. Upgrade pauses the controller before replacing its code. Migration requires a strictly increasing version.

## Pool

Constructor `(admin: Address)` sets its owner; normal deployment passes the controller. Every mutator requires that owner; end users enter through controller. No ownership-transfer endpoint is exported.

| Mutator |
| --- |
| `create_market(hub_id: u32, params: MarketParamsRaw)` |
| `update_params(hub_asset: HubAssetKey, model: InterestRateModel)` |
| `update_indexes(hub_assets: Vec<HubAssetKey>)` |
| `supply(entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation>` |
| `borrow(receiver: Address, entries: Vec<PoolBorrowEntry>) -> Vec<PoolPositionMutation>` |
| `withdraw(receiver: Address, is_liquidation: bool, entries: Vec<PoolWithdrawEntry>) -> Vec<PoolPositionMutation>` |
| `repay(payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation>` |
| `net_settle(entry: PoolNetSettleEntry) -> PoolNetSettleResult` |
| `seize_positions(entries: Vec<PoolSeizeEntry>)` |
| `flash_loan(hub_asset: HubAssetKey, initiator: Address, receiver: Address, amount: i128, data: Bytes) -> i128` |
| `create_strategy(receiver: Address, action: PoolAction, charge_fee: bool) -> PoolStrategyMutation` |
| `recapitalize(hub_asset: HubAssetKey, payer: Address, amount: i128) -> PoolAmountMutation` |
| `claim_revenue(hub_asset: HubAssetKey) -> PoolAmountMutation` |
| `upgrade(new_wasm_hash: BytesN<32>)` |

| Open view | Units / behavior |
| --- | --- |
| `get_utilisation(hub_asset: HubAssetKey) -> i128` | RAY ratio at stored indexes |
| `get_reserves(hub_asset: HubAssetKey) -> i128` | Booked cash, token units |
| `get_deposit_rate(hub_asset: HubAssetKey) -> i128` | Annual RAY APR at stored utilization |
| `get_borrow_rate(hub_asset: HubAssetKey) -> i128` | Annual RAY APR at stored utilization |
| `get_revenue(hub_asset: HubAssetKey) -> i128` | Outstanding revenue in token units at stored index; no accrual |
| `get_supplied_amount(hub_asset: HubAssetKey) -> i128` | Token units at stored index |
| `get_borrowed_amount(hub_asset: HubAssetKey) -> i128` | Token units at stored index |
| `get_delta_time(hub_asset: HubAssetKey) -> u64` | Milliseconds since accrual |
| `get_sync_data(hub_asset: HubAssetKey) -> PoolSyncData` | Stored params/state |
| `get_bulk_indexes(hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexRaw>` | Simulated accrued indexes, RAY |

## Governance

Constructor `(admin: Address, min_delay: u32)` initializes the owner, access-control admin, five operational roles and a nonzero minimum delay. Scheduled operations use the interface's `AdminOperation` variants.

`execute` must match a scheduled operation hash and predecessor and run within its execution window. With `executor = None`, anyone can execute a ready operation. With `Some(address)`, that address must authorize and hold EXECUTOR_ROLE.

| Endpoint | Authority / effect |
| --- | --- |
| `deploy_controller(wasm_hash: BytesN<32>) -> Address` | Owner; one-time deployment |
| `controller() -> Address` | Open view / resolver |
| `deploy_price_aggregator(wasm_hash: BytesN<32>) -> Address` | Owner; one-time deployment |
| `price_aggregator() -> Address` | Open view / resolver |
| `execute(executor: Option<Address>, target: Address, function: Symbol, args: Vec<Val>, predecessor: BytesN<32>, salt: BytesN<32>) -> Val` | Ready scheduled operation; optional executor |
| `cancel(canceller: Address, operation_id: BytesN<32>)` | CANCELLER_ROLE; recovery ops cannot be cancelled; target cannot cancel own revocation |
| `get_min_delay() -> u32` | Open view / resolver |
| `get_operation_state(operation_id: BytesN<32>) -> OperationState` | Open view / resolver |
| `get_operation_ledger(operation_id: BytesN<32>) -> u32` | Open view / resolver |
| `hash_operation(target: Address, function: Symbol, args: Vec<Val>, predecessor: BytesN<32>, salt: BytesN<32>) -> BytesN<32>` | Open view / resolver |
| `resolve_oracle_tolerance(tolerance: u32) -> OracleTolerance` | Open view / resolver |
| `resolve_asset_oracle(key: PriceKey, oracle: AssetOracle) -> AssetOracle` | Open view / resolver |
| `propose(proposer: Address, op: AdminOperation, salt: BytesN<32>) -> BytesN<32>` | PROPOSER_ROLE; ownership transfers additionally require proposer == current owner |
| `pause(caller: Address)` | GUARDIAN_ROLE; immediate |
| `set_spoke_asset_flags(caller: Address, spoke_id: u32, hub_asset: HubAssetKey, paused: bool, frozen: bool, no_seize: bool)` | GUARDIAN_ROLE; immediate tightening only |
| `set_sanity_band(caller: Address, key: PriceKey, min_wad: i128, max_wad: i128)` | ORACLE_ROLE; immediate tightening only |
| `create_hub(caller: Address) -> u32` | GUARDIAN_ROLE; immediate |
| `add_spoke(caller: Address) -> u32` | GUARDIAN_ROLE; immediate |
| `revoke_role_immediate(account: Address, role: Symbol)` | Owner; only guardian/oracle roles |
| `execute_self(executor: Option<Address>, op: AdminOperation, salt: BytesN<32>)` | Ready scheduled self-operation; optional executor |
| `propose_canceller_reset(new_cancellers: Vec<Address>, salt: BytesN<32>) -> BytesN<32>` | Owner; schedule uncancellable recovery |
| `execute_canceller_reset(executor: Option<Address>, new_cancellers: Vec<Address>, salt: BytesN<32>)` | Ready recovery; optional executor |
| `accept_ownership()` | Pending owner; synchronizes access-control admin and roles |
| `has_role(account: Address, role: Symbol) -> bool` | Open view / resolver |

Governance exports no generic `grant_role`, `revoke_role`, `renounce_ownership`, `get_owner`, `schedule`, or `update_delay` endpoint. Role/owner/delay/upgrade changes route through its explicit AdminOperation handlers.

## Position NFT

Account ids are NFT token ids: the NFT uses `u32`, and the controller exposes them as `u64`. Constructor `(controller: Address, uri: String, name: String, symbol: String)` reserves id 0 and stores the controller and metadata. The exported methods include the dependency defaults for NonFungibleToken and NonFungibleEnumerable.

| Endpoint | Authority / effect |
| --- | --- |
| `mint(to: Address) -> u32` | Controller; mint sequential id and renew owner/balance TTL |
| `burn(token_id: u32)` | Controller; remove ownership/enumeration and emit inherited Burn without holder auth |
| `renew(token_id: u32)` | Permissionless; existence check, owner/balance and instance TTL renewal |
| `upgrade(new_wasm_hash: BytesN<32>)` | Controller |
| `balance(account: Address) -> u32` | Open view |
| `owner_of(token_id: u32) -> Address` | Open view |
| `transfer(from: Address, to: Address, token_id: u32)` | from signature and token ownership |
| `transfer_from(spender: Address, from: Address, to: Address, token_id: u32)` | spender signature and approval |
| `approve(approver: Address, approved: Address, token_id: u32, live_until_ledger: u32)` | approver signature and ownership/operator authority |
| `approve_for_all(owner: Address, operator: Address, live_until_ledger: u32)` | owner signature |
| `get_approved(token_id: u32) -> Option<Address>` | Open view |
| `is_approved_for_all(owner: Address, operator: Address) -> bool` | Open view |
| `name() -> String` | Open view |
| `symbol() -> String` | Open view |
| `token_uri(token_id: u32) -> String` | Open view |
| `total_supply() -> u32` | Open view |
| `get_owner_token_id(owner: Address, index: u32) -> u32` | Open view |
| `get_token_id(index: u32) -> u32` | Open view |

`token_uri` reads stored base URI, appends token id and `?isStatic=true&chain=STELLAR`; it requires an existing token. No generic metadata setter, ownership setter or Burnable burn_from is exported.

## Swap aggregator

Constructor `(admin: Address)` sets the router owner. The sender authorizes `execute_strategy`. Its validated XDR can invoke an LP burn, packed swap program and LP mint. The router applies fees, checks its output balance against the route minimum, then transfers and returns the payout amount.

Recipient receipt is not remeasured, so taxed payout tokens can deliver less than the return value. Controller strategies measure positive swap receipts but do not independently enforce the route minimum. Fees require an active nonzero referral. The whitelist selects which side pays the fee; it does not restrict permitted tokens.

Lending governance has no router-upgrade operation. The router owner controls upgrades and administration.

| Endpoint | Authority |
| --- | --- |
| `execute_strategy(sender: Address, total_in: i128, swap_xdr: Bytes) -> i128` | Sender signature |
| `set_static_fee(fee_bps: u32)` | Owner |
| `add_to_whitelist(token: Address)` | Owner |
| `remove_from_whitelist(token: Address)` | Owner |
| `upgrade(new_wasm_hash: BytesN<32>)` | Owner |
| `add_referral(owner: Address, fee_bps: u32) -> u64` | Owner |
| `set_referral_fee(id: u64, fee_bps: u32)` | Owner |
| `set_referral_active(id: u64, active: bool)` | Owner |
| `set_referral_owner(id: u64, new_owner: Address)` | Owner |
| `claim_admin_fees(recipient: Address, tokens: Vec<Address>)` | Owner |
| `claim_referral_fees(id: u64, tokens: Vec<Address>)` | Permissionless; fixed referral-owner recipient |
| `sweep_balance(recipient: Address, tokens: Vec<Address>)` | Owner |
| `admin() -> Address` | Open view |
| `static_fee_bps() -> u32` | Open view |
| `referral(id: u64) -> Option<ReferralConfig>` | Open view |
| `referral_counter() -> u64` | Open view |
| `is_whitelisted(token: Address) -> bool` | Open view |
| `whitelisted_tokens() -> Vec<Address>` | Open view |
| `admin_fee_balance(token: Address) -> i128` | Open view |
| `referral_fee_balance(id: u64, token: Address) -> i128` | Open view |

`sweep_balance` excludes reserved admin and referral fee balances.

### Router and XOXNO oracle ownership

Both contracts export these Ownable methods:

| Endpoint | Authority |
| --- | --- |
| `get_owner() -> Option<Address>` | Open |
| `transfer_ownership(new_owner: Address, live_until_ledger: u32)` | Owner |
| `accept_ownership()` | Pending owner |
| `renounce_ownership()` | Owner; no pending transfer |

A zero ownership-transfer deadline cancels the matching pending transfer. NFT approvals use zero expiry to revoke approval.

## Price aggregator

Constructor `(owner: Address)` sets owner and emits OwnershipTransferCompleted.

| Endpoint | Authority / semantics |
| --- | --- |
| `get_owner() -> Option<Address>` | Open view |
| `prices(keys: Vec<PriceKey>) -> Map<PriceKey, PriceFeedRaw>` | Open; unusable price fails |
| `quotes(keys: Vec<PriceKey>) -> Map<PriceKey, PriceStatus>` | Open; unusable price status returned |
| `price_spread(key: PriceKey) -> (i128, i128)` | Open; unusable price fails |
| `oracle(key: PriceKey) -> Option<AssetOracle>` | Open view |
| `set_oracle(key: PriceKey, oracle: AssetOracle)` | Owner |
| `set_sanity_band(key: PriceKey, min_wad: i128, max_wad: i128)` | Owner |
| `set_tolerance(key: PriceKey, tolerance: OracleTolerance)` | Owner |
| `upgrade(new_wasm_hash: BytesN<32>)` | Owner |

`quotes` handles individual price-resolution failures as status; it is not a guarantee against host, storage, budget or cross-contract traps. `prices` fails on invalid status. `set_sanity_band` tightens only. Price aggregation has no ownership-transfer ABI.

## XOXNO oracle

Constructor `(admin: Address, signers: Vec<Address>, threshold: u32, resolution: u32) -> Result<(), Error>` initializes the owner and signing quorum. Each submission authenticates one registered signer. A quorum of fresh stored submissions within the allowed timestamp cluster produces the lower-median aggregate. A successful submission need not produce an aggregate.

Prices use 8 decimals. Package and aggregate-write timestamps use milliseconds; freshness and resolution parameters and Reflector timestamps use seconds.

| Endpoint | Authority / behavior |
| --- | --- |
| `add_signer(signer: Address) -> Result<(), Error>` | Owner |
| `remove_signer(signer: Address) -> Result<(), Error>` | Owner |
| `set_threshold(threshold: u32) -> Result<(), Error>` | Owner |
| `set_max_stale_seconds(seconds: u64) -> Result<(), Error>` | Owner |
| `set_max_submission_age_seconds(seconds: u64) -> Result<(), Error>` | Owner |
| `set_max_relative_skew_seconds(seconds: u64) -> Result<(), Error>` | Owner |
| `recompute_feeds(feed_ids: Vec<String>) -> Result<(), Error>` | Owner |
| `register_feed(feed_id: String) -> Result<(), Error>` | Owner |
| `add_feed(feed_id: String, asset: ReflectorAsset) -> Result<(), Error>` | Owner |
| `remove_feed(asset: ReflectorAsset) -> Result<(), Error>` | Owner |
| `set_resolution(resolution: u32) -> Result<(), Error>` | Owner |
| `purge_feed(feed_id: String) -> Result<(), Error>` | Owner |
| `read_price_data_for_feed(feed_id: String) -> Result<RedStonePriceData, Error>` | Open |
| `read_price_data(feed_ids: Vec<String>) -> Result<Vec<RedStonePriceData>, Error>` | Open |
| `read_price_history(feed_id: String, limit: u32) -> Result<Vec<RedStonePriceData>, Error>` | Open |
| `max_submission_age_seconds() -> u64` | Open |
| `max_stale_seconds() -> u64` | Open |
| `max_relative_skew_seconds() -> u64` | Open |
| `base() -> ReflectorAsset` | Open |
| `decimals() -> u32` | Open |
| `resolution() -> u32` | Open |
| `assets() -> Vec<ReflectorAsset>` | Open |
| `feeds() -> Vec<String>` | Open |
| `lastprice(asset: ReflectorAsset) -> Option<ReflectorPriceData>` | Open |
| `price(asset: ReflectorAsset, timestamp: u64) -> Option<ReflectorPriceData>` | Open |
| `prices(asset: ReflectorAsset, records: u32) -> Option<Vec<ReflectorPriceData>>` | Open |
| `submit_price(signer: Address, feed_id: String, price: i128, package_timestamp: u64) -> Result<(), Error>` | Registered signer signature |
| `submit_prices(signer: Address, feed_ids: Vec<String>, prices: Vec<i128>, package_timestamp: u64) -> Result<(), Error>` | Registered signer signature |
| `upgrade(new_wasm_hash: BytesN<32>)` | Owner |

The [four ownership methods](#router-and-xoxno-oracle-ownership) are also exported here. The configured Ownable owner administers this oracle; lending governance has no XOXNO-oracle scheduling variants.

Threshold, submission-age and relative-skew setters do not recompute aggregates. The owner can call `recompute_feeds` in bounded batches. `remove_signer` deletes that signer's submissions and recomputes affected feeds. When quorum is absent, ordinary submissions retain the previous aggregate, while owner recomputation or signer removal clears it.

Ordinary reads check aggregate freshness without reevaluating quorum. Package timestamps permit 60 seconds of future skew after flooring milliseconds to seconds. Equal timestamps from the same signer are accepted.

## DeFindex strategy

Constructor takes `asset: Address` and `init_args: Vec<Val> = [controller: Address, hub_id: u32, spoke_id: u32]`. It resolves the pool and checks the market index. The adapter owns one controller account per vault and exports no upgrade or administration endpoint.

| Endpoint | Authority / behavior |
| --- | --- |
| `asset() -> Result<Address, DeFindexStrategyError>` | Open; configured token |
| `deposit(amount: i128, from: Address) -> Result<i128, DeFindexStrategyError>` | from signature; positive measured supply, return resulting collateral balance |
| `harvest(from: Address, data: Option<Bytes>) -> Result<(), DeFindexStrategyError>` | from signature; emit price-per-share, move no funds |
| `balance(from: Address) -> Result<i128, DeFindexStrategyError>` | Open; vault collateral balance, zero if no live account |
| `withdraw(amount: i128, from: Address, to: Address) -> Result<i128, DeFindexStrategyError>` | from signature; positive amount <= balance, pay to, return remaining balance |

## Source map

- [Controller ABI](../../interfaces/controller/src/lib.rs), [administration](../../interfaces/controller/src/admin.rs), [implementation](../../contracts/controller/src/lib.rs), [account auth](../../contracts/controller/src/account.rs), [risk gates](../../contracts/controller/src/risk/validation.rs).
- [Pool ABI](../../interfaces/pool/src/lib.rs), [implementation](../../contracts/pool/src/lib.rs); [governance ABI](../../interfaces/governance/src/lib.rs), [implementation](../../contracts/governance/src/api.rs), [immediate authority](../../contracts/governance/src/timelock/immediate.rs).
- [NFT implementation](../../contracts/position-nft/src/contract.rs); [router ABI](../../interfaces/swap-aggregator/src/lib.rs), [implementation](../../contracts/swap-aggregator/src/lib.rs); [price ABI](../../interfaces/price-aggregator/src/lib.rs).
- [XOXNO oracle](../../contracts/xoxno-oracle/src/lib.rs), [admin](../../contracts/xoxno-oracle/src/admin.rs), [reads](../../contracts/xoxno-oracle/src/reads.rs), [submit](../../contracts/xoxno-oracle/src/submit.rs); [adapter](../../contracts/defindex-strategy/src/lib.rs).
- Generated NFT/default ownership semantics use OpenZeppelin stellar-contracts revision `fbfde388e1b72afa93d6b1c922067879b20e81db`, pinned in [Cargo.toml](../../Cargo.toml).
