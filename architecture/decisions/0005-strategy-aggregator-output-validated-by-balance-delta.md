# ADR 0005: Validate Strategy Aggregator Output by Balance Delta

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team

## Context

Strategies (`multiply`, `swap_collateral`, `swap_debt`,
`repay_debt_with_collateral`) route through an aggregator with an opaque swap
payload. The router may pull too much input, return no/wrong output, or
misreport amounts.

Do not trust router bookkeeping. Verify controller token balances.

## Decision

**Pre-call**

- `validate_strategy_swap` requires `amount_in > 0` and non-empty swap bytes
  (`GenericError` on failure).  
- Strategy entrypoints pass measured `amount_in` and strategy-selected
  `token_in` / `token_out` into `swap_tokens`. The opaque payload is not decoded.  
- Same-asset path (`swap_tokens_or_passthrough`): empty swap, no router.  

The controller does not decode routes, hops, or venues. There is no controller
min-out check; slippage lives in the aggregator payload.

**Auth and call**

- Pre-authorize exactly one `transfer(controller → router, amount_in)`.  
- Controller is `sender`; measured `amount_in` is `total_in`.  
- Router return value is discarded; balance deltas are truth.  

**Post-call**

- `verify_router_input_spend` — no overspend (`StrategyError::RouterOverspend`).  
- `refund_router_underspend` — unspent input to `refund_to`.  
- `verify_router_output` — output delta strictly positive (`NoSwapOutput`).  

**Reentrancy:** only the router hop runs under `FlashLoanOngoing` (shared with
flash loans). Strategy and position entrypoints call `require_not_flash_loaning`,
so a swap cannot start while a flash window is open.

## Alternatives considered

- Trust router-reported amounts  
- Per-hop on-chain validation  
- Off-chain quote as `amount_in` (use measured delta instead; slippage is in aggregator payload)  

## Consequences

**Positive:** router is a black box; strategy risk matches supply/borrow model;
reentry from router callback blocked.

**Costs:** several token balance reads per swap; integrators build
aggregator-valid route bytes; only non-zero output is enforced on-controller.

## References

- `contracts/controller/src/strategies/swap/{route,balances,auth}.rs`  
- `contracts/controller/src/storage/instance.rs`  
- `contracts/controller/src/risk/validation.rs`  
- `common/src/errors.rs`, `common/src/types/aggregator.rs`  
- `contracts/swap-aggregator/src/vault.rs`  
- [SCF_BUILD_ARCHITECTURE.md](../../SCF_BUILD_ARCHITECTURE.md) §12  
