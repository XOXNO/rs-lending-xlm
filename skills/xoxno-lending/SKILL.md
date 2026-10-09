---
name: xoxno-lending
description: "Use for XOXNO Lending tasks on Stellar: choose the contract, SDK, API, liquidation, data, or swap guidance; resolve network addresses and ids; or interpret hubs, spokes, accounts, position NFTs, health factors, WAD, RAY, and BPS."
user-invocable: true
argument-hint: "[XOXNO Lending task]"
---

# XOXNO Lending

Start here. Select one `xoxno-*` task skill. Load the companion reference
for that task. Reserve `evals/` for skill testing.

## Route the task

| You are… | Load |
|---|---|
| Writing a Soroban contract that supplies, borrows, holds a position, or receives a flash loan | [../xoxno-lending-contracts/SKILL.md](../xoxno-lending-contracts/SKILL.md) |
| Building a wallet, web app, React Native app, backend, or bot in TypeScript | [../xoxno-lending-sdk/SKILL.md](../xoxno-lending-sdk/SKILL.md) |
| Swapping tokens, or embedding a swap inside a lending action (leverage, collateral/debt swap, repay with collateral) | [../xoxno-swap-aggregator/SKILL.md](../xoxno-swap-aggregator/SKILL.md) |
| Building a liquidation bot, keeper, or risk monitor | [../xoxno-lending-liquidations/SKILL.md](../xoxno-lending-liquidations/SKILL.md) |
| Indexing events, building analytics, or calling the REST API from any language | [../xoxno-lending-data/SKILL.md](../xoxno-lending-data/SKILL.md) |
| Debugging a failed simulation or transaction, or setting up testnet | [../xoxno-lending-troubleshooting/SKILL.md](../xoxno-lending-troubleshooting/SKILL.md) |

Selection is complete when one row covers the primary task. For a composed flow,
load the lending skill first and the swap skill only for its quote or payload.

## Load shared references on demand

| Task | File |
|------|------|
| Contract addresses, RPC and API endpoints, hub ids, spoke ids, listed markets and their token contracts | [addresses.md](addresses.md) (generated from `configs/`) |
| Portfolio balances, prices, APYs, and missing values | [SDK reads](../xoxno-lending-sdk/reads.md#units-and-missing-data) |
| Full repay / withdraw sizing and share rounding | [math.md](math.md#shares-and-token-amounts) |
| Health factor, LTV weights | [math.md](math.md#health-factor-and-ltv-weighting) |
| Borrow curve, deposit rate, APR vs APY | [math.md](math.md#borrow-rate-curve) |

Reference selection is complete when every interpreted value has an explicit
unit and every market identifier includes both `hub_id` and asset address.

## Integrator read path

For ordinary wallet rendering, start with the deployed v1 API. It returns
one position per indexed owned NFT and asset/spoke arrays with prices, APYs,
labels, logos, and capacities. No client-side token joins or RAY conversion
are needed. Types and schemas belong to the SDK read entry.

Use [SDK reads](../xoxno-lending-sdk/reads.md) or
[HTTP reference](../xoxno-lending-data/api.md#integrator-v1-arrays).
Supply remains a synchronous SDK builder followed by explicit RPC preparation;
there is no supply HTTP endpoint. A displayed position is an indexed estimate. Verify current NFT ownership
and simulate the action before signing.

## Shared model

- The **Controller is the only user-facing lending contract**. Integrations use
  it for accounts and lending actions, then discover the pool with
  `get_pool_address()`. The swap aggregator is separately user-facing and
  directly callable for standalone swaps.
- A market is `HubAssetKey { hub_id, asset }`; the same token in two hubs is two
  isolated markets. Carry both fields through reads, quotes, and transactions.
- A spoke is an account's lifetime risk profile and listing set. Read its
  current configuration before opening a position.
- A position belongs to a `u64` account id. Its position NFT
  (`token_id == account_id`) is the live ownership authority.
- Token transfers and caps use asset base units. USD values and health
  factors use WAD (`1e18`). Shares, indexes, and annual rates use RAY (`1e27`).
  Risk ratios and fees use BPS (`10_000 = 100%`).

## Execute safely

1. Resolve the network and contract addresses from [addresses.md](addresses.md) or live configuration.
2. Select the exact spoke, hub, and token contract.
3. Load the task skill and its required reference.
4. Check units and rounding in [math.md](math.md) when arithmetic affects the action.
5. Build the transaction with integer arithmetic.
6. Verify the required authorization.
7. Simulate the exact transaction before requesting a signature.
8. Apply the task skill's completion checks.

For generic Stellar storage, auth, and contract testing, use the `stellar-dev`
`smart-contracts` skill. For wallet signing and submission, use its `dapp`
skill; for RPC or Horizon access, use its `data` skill.
