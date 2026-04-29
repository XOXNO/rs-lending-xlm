# Entrypoint Auth / Invariant / Pool-Call Matrix

Every public function on `controller` and `pool` with its auth gate, runtime invariants (with file:line), and downstream pool calls. Role model: `Owner` controls lifecycle and config; `KEEPER` handles bad-debt cleanup; `REVENUE` claims protocol revenue; `ORACLE` manages oracle routes. Global guards: `P` = `require_not_paused`, `F` = `require_not_flash_loaning`.

## Legend

- **Auth**: `Owner` = `#[only_owner]`; `Role(X)` = `#[only_role(caller, "X")]`; `Caller` = `caller.require_auth()`; `AcctOwner` = caller-equals-owner plus require_auth; `Liquidator` = `liquidator.require_auth()`; `Admin` = `verify_admin` (caller == controller); `View` = read-only.
- **Reentry**: `P` = `require_not_paused`; `F` = `require_not_flash_loaning`.

## Controller Lifecycle

| Fn | Auth | Reentry | Notes |
|---|---|---|---|
| `__constructor(admin)` | one-shot | — | Sets owner, admin, KEEPER+REVENUE+ORACLE roles, position limits 10/10 (`lib.rs:117-119`) |
| `upgrade(new_wasm_hash)` | Owner (`lib.rs:128`) | pauses (`lib.rs:130`) | Non-zero hash; `pausable::pause` then `upgradeable::upgrade` |
| `pause()` | Owner (`lib.rs:138`) | — | `pausable::pause` |
| `unpause()` | Owner (`lib.rs:143`) | — | `pausable::unpause` |

## User Position Operations

### `supply(caller, account_id, e_mode, assets)` — `lib.rs:152` → `positions/supply.rs:16`

| # | Check | File:line | Error |
|---|---|---|---|
| 1 | `caller.require_auth()` | supply.rs:23 | host auth |
| 2 | `require_not_paused` | supply.rs:24 | PausableError |
| 3 | `require_not_flash_loaning` | supply.rs:25 | `FlashLoanError::FlashLoanOngoing` |
| 4 | Account-owner check (skipped on first-supply when `account_id==0`) | supply.rs:38 | `GenericError::AccountNotInMarket` |
| 5 | E-mode category not deprecated (per batch) | emode.rs:65-67 | `EModeError::EModeCategoryDeprecated` |
| 6 | `validate_bulk_position_limits` (per batch, atomic) | validation.rs:70 | `CollateralError::PositionLimitExceeded` |
| 7 | Asset supported (per asset) | validation.rs:74 | `GenericError::AssetNotSupported` |
| 8 | Amount > 0 (per asset) | validation.rs:75 | `GenericError::AmountMustBePositive` |
| 9 | E-mode asset membership / `EModeWithIsolated` (per asset) | emode.rs:80-82 | `EModeError` / `CollateralError::NotCollateral` |
| 10 | `is_collateralizable` via `can_supply()` | supply.rs:84-85 | `CollateralError::NotCollateral` |
| 11 | Isolation: `MixIsolatedCollateral` etc. | emode.rs:88, 110-124 | `EModeError::MixIsolatedCollateral` |
| 12 | `supply_cap` (when cap > 0) | supply.rs:91 | `CollateralError::SupplyCapReached` |
| 13 | Token transfer caller→pool, balance-delta > 0 | supply.rs:210-212 | `GenericError::AmountMustBePositive` |
| 14 | `pool.supply(position, price_wad, amount)` | supply.rs:218 | — |

Notes: `token_approved` enforced at `create_liquidity_pool`, not at supply. Price from `cache.cached_price(asset)` (`allow_unsafe = true`; supply decreases risk).

### `borrow(caller, account_id, borrows)` — `lib.rs:166` → `positions/borrow.rs:97`

| # | Check | File:line | Error |
|---|---|---|---|
| 1 | `caller.require_auth()` | borrow.rs:98 | host auth |
| 2 | `require_not_paused` | borrow.rs:99 | PausableError |
| 3 | `require_not_flash_loaning` | borrow.rs:100 | `FlashLoanError::FlashLoanOngoing` |
| 4 | `account.owner == caller` | borrow.rs:104 | `GenericError::AccountNotInMarket` |
| 5 | `validate_bulk_position_limits` (atomic) | borrow.rs:109 | `CollateralError::PositionLimitExceeded` |
| 6 | LTV-collateral computed once per batch | borrow.rs:114-115 | feeds per-asset checks |
| 7 | Asset supported | borrow.rs:387 | `GenericError::AssetNotSupported` |
| 8 | Amount > 0 | borrow.rs:388 | `GenericError::AmountMustBePositive` |
| 9 | E-mode validation | borrow.rs:396 | `EModeError` |
| 10 | `is_borrowable` | borrow.rs:400 | `CollateralError::AssetNotBorrowable` |
| 11 | `isolation_borrow_enabled` for isolated accounts | borrow.rs:351 | `EModeError::NotBorrowableIsolation` |
| 12 | Silo: single borrow when siloed | borrow.rs:355-356 | `CollateralError::NotBorrowableSiloed` |
| 13 | Silo: existing borrows must match new asset | borrow.rs:361-367 | `CollateralError::NotBorrowableSiloed` |
| 14 | LTV: post-borrow debt ≤ LTV-collateral | borrow.rs:404-411 (`validate_ltv_collateral` 311-338) | `CollateralError::InsufficientCollateral` |
| 15 | `borrow_cap` (when cap > 0) | borrow.rs:412-419 (`validate_borrow_cap` 273-290) | `CollateralError::BorrowCapReached` |
| 16 | Isolated-debt ceiling | borrow.rs:421 (`handle_isolated_debt` 204-242, check at 228) | `EModeError::DebtCeilingReached` |

Note: No post-batch HF recompute. LTV-pre-borrow bounds risk; `validate_asset_config` enforces LTV ≤ liquidation threshold, so LTV-pass implies HF > 1. Confirm under e-mode overrides and same-tx compounding.

### `withdraw(caller, account_id, withdrawals)` — `lib.rs:174` → `positions/withdraw.rs`

| # | Check | File:line | Error |
|---|---|---|---|
| 1 | `caller.require_auth()` | withdraw.rs:17 | host auth |
| 2 | `require_not_paused` | withdraw.rs:18 | PausableError |
| 3 | `require_not_flash_loaning` | withdraw.rs:19 | `FlashLoanError::FlashLoanOngoing` |
| 4 | `account.owner == caller` | withdraw.rs:24-26 | `GenericError::AccountNotInMarket` |
| 5 | `amount == 0` sentinel → `i128::MAX` (per asset) | withdraw.rs:84 | triggers pool full-withdraw |
| 6 | Position must exist | withdraw.rs:78-81 | `CollateralError::CollateralPositionNotFound` |
| 7 | `pool.withdraw(caller, amount, position, ...)` | withdraw.rs:133-140 | — |
| 8 | Pool dust-lock: partial with residual 0 escalates to full | pool/lib.rs:186-197 | no panic |
| 9 | Pool reserve check `has_reserves(net_transfer)` | pool/lib.rs:209-212 | `CollateralError::InsufficientLiquidity` |
| 10 | Post-batch HF ≥ WAD when borrows remain | withdraw.rs:44-52 | `CollateralError::InsufficientCollateral` |

Doc drift: `INVARIANTS.md §A` and `ARCHITECTURE.md §Withdraw` claim no `amount == 0` sentinel; code has one at withdraw.rs:84. Update upstream docs.

### `repay(caller, account_id, payments)` — `lib.rs:182` → `positions/repay.rs`

| # | Check | File:line | Error |
|---|---|---|---|
| 1 | `caller.require_auth()` | repay.rs:17 | host auth |
| 2 | `require_not_paused` | repay.rs:18 | PausableError |
| 3 | `require_not_flash_loaning` | repay.rs:19 | `FlashLoanError::FlashLoanOngoing` |
| 4 | No account-owner check (anyone may repay) | repay.rs:20 | by design |
| 5 | Amount > 0 | repay.rs:50 | `GenericError::AmountMustBePositive` |
| 6 | Borrow position exists | repay.rs:53-56 | `CollateralError::DebtPositionNotFound` |
| 7 | Token transfer caller→pool, delta > 0 | repay.rs:62-71 | `GenericError::AmountMustBePositive` |
| 8 | Pool call; overpayment refund to `caller` | pool/lib.rs:251-268 | refunds repayer |
| 9 | Isolated-debt decrement, sub-$1 dust erasure | utils.rs:85-88 (from repay.rs:131-141) | `cache.flush_isolated_debts()` at repay.rs:39 |

### `liquidate(liquidator, account_id, debt_payments)` — `lib.rs:190` → `positions/liquidation.rs`

| # | Check | File:line | Error |
|---|---|---|---|
| 1 | `liquidator.require_auth()` | liquidation.rs:28 | host auth |
| 2 | `require_not_paused` | liquidation.rs:29 | PausableError |
| 3 | `require_not_flash_loaning` | liquidation.rs:30 | `FlashLoanError::FlashLoanOngoing` |
| 4 | HF must start `< WAD` | liquidation.rs:158-160 | `CollateralError::HealthFactorTooHigh` (101) |
| 5 | Three-tier HF cascade 1.02 → 1.01 → `d_max = total_coll / (1+base_bonus)` | helpers/mod.rs:216, 231, 261, 284 | — |
| 6 | Fallback regression guard `base_new_hf < WAD::ONE && < hf` | helpers/mod.rs:295 | — |
| 7 | Per-asset split: `base = capped/(1+bonus)`, `bonus = capped-base`, `protocol_fee = bonus*liquidation_fees_bps` | liquidation.rs:361-366 | conservation by construction |
| 8 | Payments loop → `pool.repay` | liquidation.rs:57-89 | — |
| 9 | Seizures loop → `pool.seize_position` | liquidation.rs:92-122 | — |
| 10 | Bad-debt trigger: `debt_usd > coll_usd && coll_usd ≤ 5*WAD` | liquidation.rs:127, 429-430 | `apply_bad_debt_to_supply_index` |
| 11 | Supply-index floor `10^18 raw` | pool/interest.rs:14, 131-135 | clamped |

`clean_bad_debt(caller, account_id)` (Role(KEEPER), `lib.rs:347`) → `clean_bad_debt_standalone` (liquidation.rs:442-471) shares `execute_bad_debt_cleanup` at line 463.

### `flash_loan(caller, asset, amount, receiver, data)` — `lib.rs:203` → `flash_loan.rs:9`

| # | Check | File:line | Error |
|---|---|---|---|
| 1 | `caller.require_auth()` | flash_loan.rs:18 | host auth |
| 2 | `require_not_paused` | flash_loan.rs:21 | PausableError |
| 3 | `require_not_flash_loaning` | flash_loan.rs:22 | `FlashLoanError::FlashLoanOngoing` |
| 4 | Amount > 0 | flash_loan.rs:25 | `GenericError::AmountMustBePositive` |
| 5 | Market active | flash_loan.rs:26 | `GenericError::PairNotActive` |
| 6 | `is_flashloanable` | flash_loan.rs:32-34 | `FlashLoanError::FlashloanNotEnabled` |
| 7 | Fee `Bps::from_raw(flashloan_fee_bps).apply_to(amount)` (half-up) | flash_loan.rs:37, common/fp.rs:189-191 | — |
| 8 | `set_flash_loan_ongoing(true)` | flash_loan.rs:43 | — |
| 9 | `pool.flash_loan_begin(asset, amount, receiver)` | flash_loan.rs:46-47 | pool→receiver |
| 10 | Receiver callback `execute_flash_loan(initiator, asset, amount, fee, data)` | flash_loan.rs:51-55 | `env.invoke_contract::<()>(...)` |
| 11 | `pool.flash_loan_end(asset, amount, fee, receiver)` | flash_loan.rs:58 | `tok.transfer(receiver→pool, amount+fee)` at pool/lib.rs:353 |
| 12 | Clear guard | flash_loan.rs:61 | — |

Soroban auth: pool/lib.rs:353 `tok.transfer(&receiver, &env.current_contract_address(), &total)` triggers `receiver.require_auth()` inside the SAC. The receiver MUST call `env.authorize_as_current_contract(...)` (or equivalent) in its callback. Not an allowance pattern. Panic in steps 8-12 reverts the tx including the guard write.

## Strategy Operations — `controller/src/strategy.rs`

| Fn | Auth | Reentry guard | Notable per-fn check |
|---|---|---|---|
| `multiply` | Caller (strategy.rs:53) | strategy.rs:55 | mode ∈ {Multiply, Long, Short} (rejects Normal at :63); Long/Short with third-token initial_payment requires convert_steps (:94-96) |
| `swap_debt` | Caller | strategy.rs:228 | — |
| `swap_collateral` | Caller | strategy.rs:345 | rejects isolated accounts at :359 (`FlashLoanError::SwapCollateralNoIso`) |
| `repay_debt_with_collateral` | Caller | strategy.rs:518 | `close_position == true` requires zero remaining borrows; panics `CannotCloseWithRemainingDebt` at :638-641 |

Aggregator interaction (`swap_tokens`): snapshots `balance_in_before`/`balance_out_before`, brackets the aggregator call with `set_flash_loan_ongoing(true/false)`, verifies spend ≤ `amount_in`, zeros residual allowance, refunds unspent input, and rejects `received < amount_out_min`. A callback into any mutating controller endpoint trips the shared reentry guard.

## Keeper / Revenue / Oracle Roles

All gates are `#[only_role(caller, "KEEPER" / "REVENUE" / "ORACLE")]` and verified. Unchanged from prior version (see git history).

## Configuration Endpoints

All `#[only_owner]` per `lib.rs:451-627`. Field-level validation per `CONFIG_INVARIANTS.md`.

## Pool Public ABI (`pool/src/lib.rs`)

Every mutating pool fn calls `verify_admin(&env)` → `ownable::enforce_owner_auth(env)` (pool/lib.rs:37-39). Construction sets admin to the controller.

| Fn | File:line | Notable |
|---|---|---|
| `__constructor(admin, params)` | 77 | Sets owner; inits indexes to RAY; `last_timestamp = ledger.timestamp() * 1000` |
| `supply(position, price_wad, amount)` | 100, verify_admin 106 | `global_sync`; `scaled = calculate_scaled_supply(amount)`; `supplied_ray += scaled` |
| `borrow(caller, amount, position, price_wad)` | 128, verify_admin 135 | `global_sync`; panics unless `has_reserves(amount)`; transfers pool→caller |
| `withdraw(caller, amount, position, ...)` | 165, verify_admin 174 | Full vs partial; dust-lock 186-197; reserve check 209-212 |
| `repay(caller, amount, position, price_wad)` | 237, verify_admin 244 | `global_sync`; refunds excess to `caller` at 251-268 (line 267) |
| `update_indexes(price_wad)` | 285, verify_admin 286 | `global_sync` |
| `add_rewards(price_wad, amount)` | 301, verify_admin 302 | Bumps supply index |
| `flash_loan_begin(asset, amount, receiver)` | 317, verify_admin 318 | pool→receiver transfer |
| `flash_loan_end(asset, amount, fee, receiver)` | 340, verify_admin 341 | `tok.transfer(receiver→pool, amount+fee)` at 353 |
| `create_strategy(...)` | 377, verify_admin 385 | Per-strategy accounting |
| `seize_position(...)` | 424, verify_admin 429 | base+bonus+fee split; may call `apply_bad_debt_to_supply_index` |
| `claim_revenue(caller, price_wad)` | 453, verify_admin 454 | Proportional burn; transfer = `min(reserves, treasury_actual)` |
| `update_params(params)` | 509, verify_admin 520 | Controller pre-validates |
| `upgrade(new_wasm_hash)` | 590, verify_admin 595 | Non-zero hash |
| `keepalive()` | 594, verify_admin 595 | Bumps Instance TTL |
