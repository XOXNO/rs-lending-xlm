# 0002. One central pool, markets isolated by `HubAssetKey`

Status: Accepted

## Context

A lending protocol needs many markets — one per listed asset, and potentially
several risk-segmented listings of the same asset — while keeping custody,
accounting, and upgrades manageable. Two forces pull in opposite directions:
isolation (a fault or bad-debt event in one market must not contaminate
another) and consolidation (liquidity, upgrade surface, and operational
overhead all favor fewer contracts). Storage keys are the cheapest isolation
mechanism Soroban offers; contract instances are the most expensive. The
protocol also segments markets by hub: the same token may need to exist as two
independent markets with different interest and bad-debt histories.

## Decision

There is exactly one pool contract holding all liquidity, and every market is
keyed by the composite `common/src/types/pool.rs::HubAssetKey`
(`{ hub_id: u32, asset: Address }`). The pool's entire persistent state is two
key families — `common/src/types/pool.rs::PoolKey::Params(HubAssetKey)` and
`PoolKey::State(HubAssetKey)` — and nothing else: no per-account storage
exists in the pool. Account state lives entirely in the controller.

Market creation derives the key and rejects duplicates
(`contracts/pool/src/ops/market.rs::create`, failing with
`GenericError::AssetAlreadySupported`). The same token listed under two hub
ids is two fully isolated markets with separate indexes, cash, revenue, and
bad-debt socialization, pinned by
`contracts/pool/tests/flows.rs::test_two_market_isolation`.

Market identity carries `hub_id` everywhere it travels: controller-side cap
usage rows are keyed by the
`common/src/types/controller.rs::ControllerKey::SpokeUsage(u32, HubAssetKey)`
variant, so no controller-side structure ever collapses two hub listings of
one asset into a single row.

## Alternatives

**One pool contract instance per market (factory pattern).** Each market
would be its own deployed contract, isolating balances at the contract level.
This buys isolation the key already provides, while multiplying the upgrade
surface by the number of markets, fragmenting custody across N addresses, and
turning every cross-market operation (liquidation touching several assets,
protocol-wide accounting) into a fan-out of cross-contract calls with N
authorization edges. The single-pool design keeps one custody address, one
WASM to upgrade, and one owner edge to audit.

**Keying markets by asset address alone.** A plain `Address` key is simpler,
but it forecloses hub-level segmentation: the same token could never carry two
independent interest and bad-debt histories. Retrofitting a composite key
later would migrate every persisted market row; paying the `u32` up front
costs almost nothing.

**Per-account state in the pool.** The pool could track each account's scaled
shares itself. That would smear account logic across the custody boundary,
require the pool to know account identity, and duplicate state the controller
already owns. Keeping the pool aggregate-only means its accounting invariants
close over two key families and nothing else.

## Consequences

Isolation is structural, not procedural: a bad-debt write-down, index
movement, or revenue change in one `HubAssetKey` cannot touch another,
because no code path addresses more than one market per key — see
../../reference/invariants.md §INV-ACCT and §INV-IDX. Storage stays flat and
enumerable (§INV-STOR), and adding a market is a key insertion, not a
deployment. Consolidated liquidity means a single custody address to defend
(see ../threat-model.md).

What this makes hard: `hub_id` is load-bearing in every event payload,
indexer row, SDK call, and Certora pool spec — dropping or renumbering it is
a breaking change for all of them, and any change to the key layout migrates
the pool's entire persistent state. What must stay true: the pool must never
grow per-account keys, and market identity must never be reduced to the asset
address alone.
