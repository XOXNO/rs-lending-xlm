# Governance

Timelocked admin of the lending controller and price-aggregator. Role gates,
delays, and Recovery reset are documented on the rustdoc entrypoints.

| | |
| --- | --- |
| Owner | OZ `Ownable` (two-step) |
| Roles | `PROPOSER`, `EXECUTOR`, `CANCELLER`, `GUARDIAN`, `ORACLE` |
| Interface | `interfaces/governance` |

Pending ops only keep `OperationLedger` storage; execute and cancel remove it.
`salt` uniquifies re-proposes; `predecessor` is always `0`.

## Entrypoints

| Call | Role |
| --- | --- |
| `propose` | `PROPOSER` — schedule `AdminOperation` |
| `execute` / `execute_self` | `EXECUTOR` optional — run ready op |
| `cancel` | `CANCELLER` — veto pending (not Recovery) |
| `pause` / `set_spoke_asset_flags` / `create_hub` / `add_spoke` | `GUARDIAN` — immediate |
| `set_sanity_band` | `ORACLE` — immediate |
| `revoke_role_immediate` | Owner — strip `GUARDIAN`/`ORACLE` |
| `propose_canceller_reset` / `execute_canceller_reset` | Owner / open — Recovery reset |
| `deploy_controller` / `deploy_price_aggregator` | Owner — one-shot |
| `accept_ownership` | Pending owner |
| Views (`get_*`, `hash_operation`, `has_role`, `resolve_*`, addresses) | Public |

## Halt controls (global + per listing)

| Control | Immediate (GUARDIAN) | Recovery / clear |
| --- | --- | --- |
| Global controller pause | `pause` | Timelocked `AdminOperation::Unpause` |
| Per-spoke-asset `paused` / `frozen` | `set_spoke_asset_flags` (**ratchet**: may only tighten; clearing reverts `SpokeAssetFlagRelaxation`) | Timelocked `AdminOperation::EditAssetInSpoke` with the desired flags |

`EditAssetInSpoke` rewrites the full listing (risk params, caps, **and** halt
flags). Clearing flags there is intentional and delayed — not a bypass of the
immediate-path ratchet. Always pass the intended `paused`/`frozen` on every
edit so a risk-param change does not accidentally re-open a halted listing.

## Entrypoints

Signatures are copied from `contracts/governance/src/`. The `Env` argument is
dropped by the generated client, so a client call takes one fewer argument than
the signature shows.

| Entrypoint | Signature | Notes | What it does |
| --- | --- | --- | --- |
| `deploy_controller` | `fn deploy_controller(env: Env, wasm_hash: BytesN<32>) -> Address` | owner-only | Deploys the controller contract from `wasm_hash` and records its address. |
| `controller` | `fn controller(env: Env) -> Address` | — | Returns the deployed controller's address. |
| `deploy_price_aggregator` | `fn deploy_price_aggregator(env: Env, wasm_hash: BytesN<32>) -> Address` | owner-only | Deploys the price aggregator contract from `wasm_hash`, records its address, and registers it with the controller if one is deployed. |
| `price_aggregator` | `fn price_aggregator(env: Env) -> Address` | — | Returns the deployed price aggregator's address. |
| `execute` | `fn execute( env: Env, executor: Option<Address>, target: Address, function: Symbol, args: Vec<Val>, predecessor: BytesN<32>, salt: BytesN<32>, ) -> Val` | — | Executes a ready, non-expired scheduled op against `target` (not this contract). |
| `cancel` | `fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>)` | — | Cancels a pending operation. |
| `get_min_delay` | `fn get_min_delay(env: Env) -> u32` | — | Returns the timelock's configured minimum delay, in ledgers. |
| `get_operation_state` | `fn get_operation_state(env: Env, operation_id: BytesN<32>) -> OperationState` | — | Returns the current state of the operation identified by `operation_id`. |
| `get_operation_ledger` | `fn get_operation_ledger(env: Env, operation_id: BytesN<32>) -> u32` | — | Returns the ledger at which the operation becomes ready (delay elapsed). |
| `hash_operation` | `fn hash_operation( env: Env, target: Address, function: Symbol, args: Vec<Val>, predecessor: BytesN<32>, salt: BytesN<32>, ) -> BytesN<32>` | — | Computes the operation id for the given target, function, arguments, predecessor, and salt. |
| `resolve_oracle_tolerance` | `fn resolve_oracle_tolerance(env: Env, tolerance: u32) -> OracleTolerance` | — | Validates `tolerance` and returns the resolved oracle tolerance bounds. |
| `resolve_asset_oracle` | `fn resolve_asset_oracle(env: Env, key: PriceKey, oracle: AssetOracle) -> AssetOracle` | — | Resolves `oracle` for `key`, filling in `asset_decimals` from the token contract for a `PriceKey::Token` key or `0` for `PriceKey::Ref`. |
| `propose` | `fn propose(env: Env, proposer: Address, op: AdminOperation, salt: BytesN<32>) -> BytesN<32>` | — | Schedules `op` for later execution and returns its operation id. |
| `pause` | `fn pause(env: Env, caller: Address)` | — | Pauses the controller. |
| `set_spoke_asset_flags` | `fn set_spoke_asset_flags( env: Env, caller: Address, spoke_id: u32, hub_asset: HubAssetKey, paused: bool, frozen: bool, no_seize: bool, )` | — | Sets the paused, frozen, and no-seize flags for `hub_asset` in spoke `spoke_id`. |
| `set_sanity_band` | `fn set_sanity_band(env: Env, caller: Address, key: PriceKey, min_wad: i128, max_wad: i128)` | — | Sets the sanity-check price band for `key` on the price aggregator. |
| `create_hub` | `fn create_hub(env: Env, caller: Address) -> u32` | — | Creates a new hub on the controller and returns its id. |
| `add_spoke` | `fn add_spoke(env: Env, caller: Address) -> u32` | — | Creates a new spoke on the controller and returns its id. |
| `revoke_role_immediate` | `fn revoke_role_immediate(env: Env, account: Address, role: Symbol)` | owner-only | Revokes `role` from `account` without going through the timelock. |
| `execute_self` | `fn execute_self(env: Env, executor: Option<Address>, op: AdminOperation, salt: BytesN<32>)` | — | Executes a ready, non-expired scheduled admin operation that targets this contract itself. |
| `propose_canceller_reset` | `fn propose_canceller_reset( env: Env, new_cancellers: Vec<Address>, salt: BytesN<32>, ) -> BytesN<32>` | owner-only | Schedules a reset of the canceller role to `new_cancellers` and returns its operation id. |
| `execute_canceller_reset` | `fn execute_canceller_reset( env: Env, executor: Option<Address>, new_cancellers: Vec<Address>, salt: BytesN<32>, )` | — | Executes a ready, non-expired scheduled reset of the canceller role to `new_cancellers`. |
| `accept_ownership` | `fn accept_ownership(env: Env)` | — | Completes a pending ownership transfer to the caller. |
| `has_role` | `fn has_role(env: Env, account: Address, role: Symbol) -> bool` | — | Returns whether `account` currently holds `role`. |
| `__constructor` | `pub fn __constructor(env: Env, admin: Address, min_delay: u32)` | — | Initializes the governance contract: sets `admin` as both owner and access-control admin, grants it every default operational role, and sets the timelock minimum delay to `min_delay`. |

Error codes: [`../../docs/reference/errors.md`](../../docs/reference/errors.md).
Events: [`../../docs/reference/events.md`](../../docs/reference/events.md).
