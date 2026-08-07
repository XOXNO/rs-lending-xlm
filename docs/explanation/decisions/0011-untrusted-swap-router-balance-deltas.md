# 0011. The swap router is untrusted: routes computed off-chain, verified on-chain by balance deltas

Status: Accepted

## Context

Strategy flows (`multiply`, `swap_debt`, `swap_collateral`,
`repay_debt_with_collateral`) need token swaps, and good execution requires
multi-venue, multi-hop routes that only an off-chain solver can compute. Putting
route knowledge on-chain means whitelisting venues, pools, and path shapes —
governance surface and code that grows with every DEX integration. But accepting
an opaque route means the router and every venue beneath it are attacker-shaped:
they may lie about outputs, spend more input than authorized, refund garbage, or
re-enter the controller mid-swap. The controller must therefore extract a
correct swap result from a callee it does not trust, while granting that callee
the narrowest possible spending power.

## Decision

The route is an opaque payload: `common/src/types/aggregator.rs::StrategySwap`
is a bare `Bytes` alias, and the only pre-flight check is non-empty bytes plus a
positive amount (`contracts/controller/src/strategies/swap/route.rs::validate_strategy_swap`).
Verification is entirely by measured balance deltas, orchestrated in
`contracts/controller/src/strategies/swap/mod.rs::swap_tokens`:

1. `contracts/controller/src/strategies/swap/balances.rs::snapshot_swap_balances`
   records the controller's own `token_in`/`token_out` balances;
2. `contracts/controller/src/strategies/swap/auth.rs::pre_authorize_router_pull`
   mints exactly one scoped `authorize_as_current_contract` entry permitting
   `transfer(controller, router, amount_in)` — nothing else, no sub-invocations;
3. `contracts/controller/src/strategies/swap/route.rs::call_router_with_reentrancy_guard`
   invokes the router inside the flash guard and discards its returned amount;
4. `contracts/controller/src/strategies/swap/balances.rs::settle_router_input`
   rejects any net `token_in` gain or overspend (`RouterOverspend`) and refunds
   the unspent remainder;
5. `contracts/controller/src/strategies/swap/balances.rs::verify_router_output`
   requires a strictly positive measured `token_out` delta (`NoSwapOutput`) and
   returns that measurement as the swap result.

The router applies the same philosophy inward:
`contracts/swap-aggregator/src/venues/mod.rs::dispatch_hop` credits each venue's
measured output delta, ignores the adapter's reported amount, and requires the
measured input spend to equal `amount_in` exactly;
`contracts/swap-aggregator/src/lib.rs::execute_strategy` enforces the payload's
`total_min_out` against the vault's measured balance and sweeps residue into
admin revenue only up to `residual_allowance`, panicking on anything larger.
Economic route quality is enforced solely by `total_min_out` plus the
controller's post-strategy health-factor gates
(`contracts/controller/src/risk/validation.rs::require_post_pool_risk_gates`).
Adversarial router behavior is pinned by
`tests/test-harness/src/mock_aggregator.rs::BadAggregator`.

## Alternatives

**On-chain route validation and venue whitelisting.** The controller would parse
the route and check every hop against a governed registry of venues and pools.
This buys nothing the balance deltas do not already guarantee — a whitelisted
venue can still misreport — while adding a large decoding surface, a governance
process per DEX listing, and a standing incentive to ship router upgrades. The
measured-delta design makes the router swappable without touching the
controller's trust argument.

**Trusting the router's returned amount.** Using the return value (or the venue
adapters' reported outputs) as the credited result is one lie away from minting
phantom collateral. The implementation discards the return value entirely; only
tokens that verifiably arrived at the controller count, and the same rule holds
one level down where `dispatch_hop` measures each venue.

**Blanket token approval to the router.** A standing allowance would save one
auth entry per call but converts any router compromise into unlimited drainage
of controller-held balances. The per-call scoped authorization caps the blast
radius of a malicious route at exactly `amount_in` of one token, and
`settle_router_input` claws back even that if it goes unspent.

## Consequences

The controller's safety argument is independent of router code: a router that
misbehaves can only waste the pre-authorized `amount_in` or fail the
transaction, never inflate credited output — the STRAT and ACCT domains of
../../reference/invariants.md rest on the measured deltas, and the RISK domain's
post-strategy gates bound the economic damage of a bad-but-successful route.
Reentrancy through the router is covered by the shared flash guard (see ADR
0010 and ../threat-model.md).

What it makes hard: the protocol cannot police price quality on-chain beyond
`total_min_out`; a user (or compromised off-chain quoting service) that signs a
generous `total_min_out` accepts that slippage. The `StrategyPayload` wire
format is quoted by off-chain infrastructure and pinned by test, so it evolves
only with coordinated releases.

What must stay true: the router's return value stays decorative; every
authorization stays single-call, exact-amount, with empty sub-invocations; and
the flash guard keeps wrapping the router call so no position verb is reachable
mid-swap. The `BadAggregator` suite is the regression fence — new router
behaviors get an adversarial mode there first.
