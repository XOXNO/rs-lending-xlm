# Controller

The only user-facing lending contract: holds account state, applies risk
rules and drives the token-holding pool, which users never call directly. The
swap aggregator stays callable alone for plain swaps.

| Calls | Why |
| --- | --- |
| pool | Token custody, per-market share accounting; owned by the controller ([README](../pool/README.md)) |
| price-aggregator | USD prices. A failed read reverts every state-changing call; `get_market_indexes_detailed` reports `valid: false` instead |
| swap-aggregator | Swap routes for strategy entrypoints; untrusted |
| position-nft | Account ownership; `account_id` = NFT `token_id` ([README](../position-nft/README.md)) |

| | |
| --- | --- |
| Owner | Governance, after deployment |
| Client | [`interfaces/controller`](../../interfaces/controller); drops `Env` |

## Authorization

Signatures, caller authority and pause behaviour:
[endpoint reference](../../docs/reference/endpoints.md#controller). Spoke-asset
flags (`paused`, `frozen`, `no_seize`):
[risk checks and pause flags](../../docs/reference/endpoints.md#risk-checks-and-pause-flags).

**Owner** (`#[only_owner]`) is governance; most owner calls pass its timelock.
Through governance, a GUARDIAN calls `pause` and `set_spoke_asset_flags`
without delay, and `create_hub` / `add_spoke` require `CURATOR_ROLE`
([`timelock/immediate.rs`](../governance/src/timelock/immediate.rs)). Flag
relaxation uses the timelocked `relax_spoke_asset_flags`
([governance README](../governance/README.md)).

**Global pause** (`#[when_not_paused]`) leaves exit and cleanup paths open,
plus `renew_account` and `remove_delegate`.

## Entrypoint notes

- `migrate_from_blend` borrows each `debt_caps` amount, repays Blend, repays
  the unused borrow, then sweeps Blend collateral and supply into the pool.
- `flash_position` forwards measured debt tokens to `receiver`, calls
  `execute_flash_position`, then deposits the measured balance increase of
  `collaterals`; the account must end solvent.
- A minimum-borrow floor (`set_min_borrow_collateral_usd`) of 0 disables the
  check.
- `__constructor(admin)` sets both position limits to `POSITION_LIMIT_MAX`, the
  floor to `DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD`, the version to
  `INITIAL_APP_VERSION`. **It starts paused.**
- `add_spoke` uses the default liquidation curve; `remove_spoke` deprecates a
  spoke, blocking new positions; `remove_asset_from_spoke` needs zero usage.
- `add_asset_to_spoke` / `edit_asset_in_spoke` validate against pool decimals.
  `upgrade_liquidity_pool_params` accrues before replacing the rate model.
- `upgrade` **pauses first**; call `unpause` after.

## Storage and TTL

All persistent keys are variants of `ControllerKey` in
[`common/src/types/controller.rs`](../../common/src/types/controller.rs).

| Keys | Tier | TTL window |
| --- | --- | --- |
| `Pool`, `SwapAggregator`, `PriceAggregator`, `PositionNft`, `Accumulator`, `PositionLimits`, `MinBorrowCollateralUsd`, `AppVersion`, `LastHubId`, `LastSpokeId` | instance | `TTL_THRESHOLD_INSTANCE` / `TTL_BUMP_INSTANCE`, renewed by state-changing calls |
| `Hub`, `Spoke`, `SpokeAsset`, `SpokeUsage`, `SpokeFlagsEpoch`, `PositionManager`, `BlendPoolAllowed` | persistent | `TTL_THRESHOLD_SHARED` / `TTL_BUMP_SHARED`, on read and write |
| `AccountMeta`, `SupplyPositions`, `BorrowPositions`, `Delegates` | persistent | `TTL_THRESHOLD_USER` / `TTL_BUMP_USER`, on read and write; `renew_account` extends them on demand |
| `FlashLoanOngoing` | temporary | Set only for the duration of a guarded external call |

The account owner lives in the position NFT, not here. See
[`../position-nft/README.md`](../position-nft/README.md#storage-and-ttl).

## Events

See [controller events](../../docs/reference/events.md#controller-events) for
fields and scales.

## Layout

```text
contracts/controller/src/
  lib.rs          # entrypoints and owner gates
  positions/      # supply, borrow, repay, withdraw, liquidation
  strategies/     # flash loan, flash position, multiply, swaps, Blend migration
  risk/           # health factor, LTV, risk parameters, validation
  config/         # hubs, spokes, listings, registry settings
  storage/        # typed accessors; the only place ControllerKey is read or written
  external/       # pool, position NFT, price aggregator and Blend calls
  events/         # account, market and configuration events
  account.rs, payments.rs, markets.rs, governance.rs, views.rs, context.rs
```

Internal module paths are load-bearing: the Certora spec and unit tests call
items such as `crate::positions::supply::process_supply` by exact path.

## References

- Errors: [`docs/reference/errors.md`](../../docs/reference/errors.md)
- Formulas: [`docs/reference/formulas.md`](../../docs/reference/formulas.md)
- Integration model: [`skills/xoxno-lending/SKILL.md`](../../skills/xoxno-lending/SKILL.md)
- Formal proofs: [`certora/controller/spec`](../../certora/controller/spec/README.md)
