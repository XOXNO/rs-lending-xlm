---
name: xoxno-lending
description: "Use for XOXNO Lending tasks on Stellar: choose the contract, SDK, API, liquidation, data, or swap guidance; resolve network addresses and ids; or interpret hubs, spokes, accounts, position NFTs, health factors, WAD, RAY, and BPS."
user-invocable: true
argument-hint: "[XOXNO Lending task]"
---

# XOXNO Lending

Start here: select one `xoxno-*` task skill and load its companion reference.
`evals/` is for skill testing only.

## Route the task

| You are… | Load |
|---|---|
| Writing a Soroban contract that supplies, borrows, holds a position, or takes a flash loan | [../xoxno-lending-contracts/SKILL.md](../xoxno-lending-contracts/SKILL.md) |
| Building a wallet, web app, React Native app, backend, or bot in TypeScript | [../xoxno-lending-sdk/SKILL.md](../xoxno-lending-sdk/SKILL.md) |
| Swapping tokens, or embedding a swap inside a lending action (leverage, collateral/debt swap, repay with collateral) | [../xoxno-swap-aggregator/SKILL.md](../xoxno-swap-aggregator/SKILL.md) |
| Building a liquidation bot, keeper, or risk monitor | [../xoxno-lending-liquidations/SKILL.md](../xoxno-lending-liquidations/SKILL.md) |
| Indexing events, building analytics, or calling the REST API from any language | [../xoxno-lending-data/SKILL.md](../xoxno-lending-data/SKILL.md) |
| Debugging a failed simulation or transaction, or setting up testnet | [../xoxno-lending-troubleshooting/SKILL.md](../xoxno-lending-troubleshooting/SKILL.md) |

One row should cover the primary task. For a composed flow, load the lending
skill first and the swap skill only for its quote or payload.

## Load shared references on demand

| Task | File |
|------|------|
| Contract addresses, RPC/API endpoints, hub and spoke ids, markets and token contracts | [addresses.md](addresses.md) (generated from `configs/`) |
| Portfolio balances, prices, APYs, and missing values | [SDK reads](../xoxno-lending-sdk/reads.md#units-and-missing-data) |
| Full repay / withdraw sizing and share rounding | [math.md](math.md#shares-and-token-amounts) |
| Health factor, LTV weights | [math.md](math.md#health-factor-and-ltv-weighting) |
| Borrow curve, deposit rate, APR vs APY | [math.md](math.md#borrow-rate-curve) |

Every interpreted value needs an explicit unit, and every market identifier
both `hub_id` and asset address.

## Integrator read path

For wallet rendering, start with the deployed v1 API: one position per
indexed owned NFT, plus asset/spoke arrays with prices, APYs, labels, logos,
and capacities, with no client-side token joins or RAY conversion. Types and
schemas: [SDK reads](../xoxno-lending-sdk/reads.md); HTTP:
[integrator v1 arrays](../xoxno-lending-data/api.md#integrator-v1-arrays).
A displayed position is an indexed estimate; verify current NFT ownership and
simulate before signing. Supply has no HTTP endpoint: use the synchronous SDK
builder, then explicit RPC preparation.

## Shared model

- The **Controller is the only user-facing lending contract**: use it for
  accounts and lending actions, and discover the pool with
  `get_pool_address()`. The swap aggregator is separately callable for
  standalone swaps.
- A market is `HubAssetKey { hub_id, asset }`; the same token in two hubs is two
  isolated markets. Carry both fields through reads, quotes, and transactions.
- A spoke is an account's lifetime risk profile and listing set; read its
  current configuration before opening a position.
- A position belongs to a `u64` account id. Its position NFT
  (`token_id == account_id`) is the live ownership authority.
- Units: token transfers and caps in asset base units; USD values and health
  factors in WAD (`1e18`); shares, indexes, and annual rates in RAY (`1e27`);
  risk ratios and fees in BPS (`10_000 = 100%`). Full table:
  [math.md#units](math.md#units).

## Execute safely

1. Resolve network and contract addresses from [addresses.md](addresses.md) or live configuration.
2. Select the exact spoke, hub, and token contract.
3. Load the task skill and its reference.
4. Check units and rounding in [math.md](math.md) when arithmetic matters.
5. Build the transaction with integer arithmetic.
6. Verify the required authorization.
7. Simulate the exact transaction before requesting a signature.
8. Apply the task skill's completion checks.

For generic Stellar storage, auth, and contract testing, use the `stellar-dev`
`smart-contracts` skill; for wallet signing and submission, its `dapp` skill;
for RPC or Horizon access, its `data` skill.

## Verified against

These pins apply to every `xoxno-*` skill ("the pinned SDK" links here).

- SDK: `@xoxno/sdk-js` `1.0.228`; SDK claims, export names, and code samples
  target this release.
- Off-chain services: the XOXNO public API (`api.xoxno.com`) and the swap quote
  server (`stellar-swap.xoxno.com`), as used by that SDK release.
- Contract crate: `xoxno-contract-sdk` at the version named in
  [xoxno-lending-contracts](../xoxno-lending-contracts/SKILL.md).
- Contracts: the `main` branch of
  [rs-lending-xlm](https://github.com/XOXNO/rs-lending-xlm).

Re-check the affected skills after a release of any of these.
