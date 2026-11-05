# Morpho curator roles, and what they teach the curator registry

Date: 2026-11-05

Scope: Morpho Blue core, MetaMorpho V1 and V1.1, Morpho Vaults V2, the Public
Allocator, the role holders of eight large vaults, and a comparison with the
uncommitted curator registry on branch `feat/curator-registry`
(`contracts/curator/src/*.rs` plus `git diff v1.1.0 -- contracts/governance/src interfaces/governance/src`).

Evidence labels:

- **Code**: read in the Morpho Solidity at the pinned commit below.
- **On-chain**: read by JSON-RPC `eth_call()` (Ethereum near block 26349679, Base
  at the same time) or from the Morpho API at `https://api.morpho.org/graphql`
  on 2026-11-05.
- **Docs**: docs.morpho.org only. Not checked against code unless stated.
- **Inferred** / **Unverified**: as defined in the repository instructions.

Source pins (all links below use these commits):

| Repository | Commit |
|---|---|
| morpho-org/morpho-blue | `b4af81dcd9e4e2f2d937cc57ab7f654c18f7c73c` |
| morpho-org/metamorpho (V1) | `a99b95b2f1c7bd2bbac6757ba61e7083e3797659` |
| morpho-org/metamorpho-v1.1 | `b6bb715896da481f6c49f2d276a0ed7dce5794a0` |
| morpho-org/vault-v2 | `48da66bb64af98320c34483fcc42177645e44338` |
| morpho-org/public-allocator | `aa324d81c2be9b25260ad865c18248e4099d274e` |

## Key findings

1. Morpho never lets a curator halt liquidations or withdrawals. Morpho Blue has
   no pause and no per-market flag; a market is immutable once created
   (Code). Vault curators control only where vault money goes, never the
   market rules. Our `FLAGS` role can halt seizure (`no_seize`) and, through
   `paused`, block repayment and user exit in a spoke, instantly.
2. Every Morpho design is asymmetric: risk-increasing changes wait for a
   timelock, risk-reducing changes are instant. V1: cap raise timelocked, cap
   cut instant. V2: every curator setter is timelockable except the two cap
   decreases (Code).
3. Morpho's timelock floor is real time, not a token delay. V1 enforces 1 day to
   2 weeks after setup (Code). Observed production vaults use 3 days and
   7 days for cap raises (On-chain). Our listing proposals wait 12 ledgers
   (about one minute) on mainnet, which gives suppliers no exit window.
4. A separate veto role exists in every Morpho vault: the V1 guardian and the
   V2 sentinels revoke pending changes and (V2) cut caps and pull funds to
   idle. They cannot add risk (Code). In production these are Safes or Aragon
   DAOs separate from the curator Safe (On-chain).
5. V2 lets the curator pick a timelock per function selector, makes a timelock
   decrease wait for the old timelock, and can abdicate a selector forever
   (Code). Large vaults abdicate the exit gates (On-chain). The API shows an
   abdicated selector as timelock `0`, so the `abdicated(bytes4)` view must be
   read too.
6. In practice: owner and curator are multisigs (3/7 to 5/10); allocators are
   bots (EOAs, EIP-7702 delegated EOAs, or bot contracts) plus the Public
   Allocator; one large V2 vault uses 1-of-1 Safes for owner and curator
   (On-chain).
7. Our registry has the right raw pieces (lower caps instantly, queue raises,
   cap grants) but enforces the asymmetry only inside the curator contract.
   Governance `edit_asset_in_spoke` accepts any value from any `CURATOR_ROLE`
   holder. Morpho enforces the asymmetry in the contract that holds the state.

## 1. Morpho Blue core

- **Permissionless market creation (Code).** `createMarket()` has no caller
  check. It requires only that the IRM and LLTV are enabled and the market id is
  new ([Morpho.sol#L150-L166](https://github.com/morpho-org/morpho-blue/blob/b4af81dcd9e4e2f2d937cc57ab7f654c18f7c73c/src/Morpho.sol#L150-L166)).
  The market id hashes loan token, collateral token, oracle, IRM, and LLTV, so
  the creator picks the oracle freely; governance never whitelists oracles.
- **What the owner controls (Code).** `onlyOwner` gates `setOwner()`,
  `enableIrm()`, `enableLltv()` (LLTV below 1e18), `setFee()` per market (at most
  `ConstantsLib.MAX_FEE` = 25 % of interest), and `setFeeRecipient()`
  ([Morpho.sol#L95-L145](https://github.com/morpho-org/morpho-blue/blob/b4af81dcd9e4e2f2d937cc57ab7f654c18f7c73c/src/Morpho.sol#L95-L145),
  [ConstantsLib.sol#L5](https://github.com/morpho-org/morpho-blue/blob/b4af81dcd9e4e2f2d937cc57ab7f654c18f7c73c/src/libraries/ConstantsLib.sol#L5)).
  Enabling is one-way: there is no disable function for IRMs or LLTVs.
- **Immutability (Code + Docs).** A market's parameters never change after
  creation; the docs describe markets as
  immutable after deployment ([docs: Morpho Markets](https://docs.morpho.org/learn/concepts/blue)).
- **Liquidation cannot be halted (Code).** `liquidate()` checks only that the
  market exists and the borrower is unhealthy at the oracle price
  ([Morpho.sol#L347-L415](https://github.com/morpho-org/morpho-blue/blob/b4af81dcd9e4e2f2d937cc57ab7f654c18f7c73c/src/Morpho.sol#L347-L415)).
  The contract has no pause or freeze function. The incentive is a fixed
  function of LLTV, capped at 1.15
  ([ConstantsLib.sol#L11-L14](https://github.com/morpho-org/morpho-blue/blob/b4af81dcd9e4e2f2d937cc57ab7f654c18f7c73c/src/libraries/ConstantsLib.sol#L11-L14)).
  Bad debt is socialised to the market's suppliers in the same call.
- **Who is owner (On-chain vs Docs).** `owner()` of
  [0xBBBB...FFCb](https://etherscan.io/address/0xBBBBBbbBBb9cC5e90e3b3Af64bdAF62C37EEFFCb)
  is the Safe
  [0xcBa2...9AFa](https://etherscan.io/address/0xcBa28b38103307Ec8dA98377ffF9816C164f9AFa).
  The docs call it a 5/9 multisig
  ([docs: organization](https://docs.morpho.org/learn/governance/organization));
  on chain `getThreshold()` returns 6 with 10 owners. `feeRecipient()` is the
  zero address (fee switch off). Governance scope per the docs: treasury, fee
  switch, LLTV and IRM whitelisting
  ([docs: governance](https://docs.morpho.org/learn/governance)).

## 2. MetaMorpho V1 and V1.1

### Roles and exact powers (Code, V1 [MetaMorpho.sol](https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/MetaMorpho.sol))

The modifiers nest: the owner passes every role check, the curator passes the
allocator check
([#L139-L170](https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/MetaMorpho.sol#L139-L170)).
Ownership is two-step (OpenZeppelin `Ownable2Step.sol`).

| Role (count) | Function | Timelock |
|---|---|---|
| Owner (1) | `setCurator()`, `setIsAllocator()`, `setSkimRecipient()`, `setFeeRecipient()` | Instant |
| Owner | `setFee()` (at most 50 %) | Instant ([#L229-L241](https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/MetaMorpho.sol#L229-L241)) |
| Owner | `submitTimelock()` raise | Instant |
| Owner | `submitTimelock()` lower | Waits the current timelock ([#L213-L226](https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/MetaMorpho.sol#L213-L226)) |
| Owner | `submitGuardian()` | Instant if no guardian, else timelocked ([#L257-L268](https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/MetaMorpho.sol#L257-L268)) |
| Curator (1) | `submitCap()` lower | Instant |
| Curator | `submitCap()` raise, including enabling a new market | Timelocked ([#L273-L289](https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/MetaMorpho.sol#L273-L289)) |
| Curator | `submitMarketRemoval()` (cap must be 0) | Timelocked ([#L292-L303](https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/MetaMorpho.sol#L292-L303)) |
| Curator or guardian | `revokePendingCap()`, `revokePendingMarketRemoval()` | Instant ([#L434-L446](https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/MetaMorpho.sol#L434-L446)) |
| Guardian (1) | `revokePendingTimelock()`, `revokePendingGuardian()` | Instant ([#L420-L431](https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/MetaMorpho.sol#L420-L431)) |
| Allocators (many) | `setSupplyQueue()`, `updateWithdrawQueue()`, `reallocate()` within caps | Instant ([#L308-L415](https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/MetaMorpho.sol#L308-L415)) |
| Anyone | `acceptCap()`, `acceptTimelock()`, `acceptGuardian()` after the delay | ([#L460-L475](https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/MetaMorpho.sol#L460-L475)) |

Rules worth copying:

- **Bounds (Code).** V1 timelock is 1 day to 2 weeks, enforced at construction
  too ([ConstantsLib.sol#L10-L19](https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/libraries/ConstantsLib.sol#L10-L19),
  [#L130](https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/MetaMorpho.sol#L130)).
- **A pending raise must be revoked before any new cap (Code).** While a raise
  is pending, `submitCap()` reverts for every new value, a cut included, so the
  curator or guardian calls `revokePendingCap()` first and then cuts.
- **Removal needs cap 0, then the timelock, then zero supply or an elapsed
  removal timer** before `updateWithdrawQueue()` can drop the market
  ([#L323-L363](https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/MetaMorpho.sol#L323-L363)).
- **Guardian cannot revoke fees (Code).** `setFee()` is instant in V1, so the
  guardian has no fee veto at all.

### V1.1 changes (Code + README)

The [V1.1 README](https://github.com/morpho-org/metamorpho-v1.1/blob/b6bb715896da481f6c49f2d276a0ed7dce5794a0/README.md)
lists four changes, confirmed in
[MetaMorphoV1_1.sol](https://github.com/morpho-org/metamorpho-v1.1/blob/b6bb715896da481f6c49f2d276a0ed7dce5794a0/src/MetaMorphoV1_1.sol):

1. Bad debt is not realized: `lostAssets` holds the missing amount so
   `totalAssets()` does not fall, and the docs suggest covering it by supplying
   on behalf of `address(1)`
   ([#L109](https://github.com/morpho-org/metamorpho-v1.1/blob/b6bb715896da481f6c49f2d276a0ed7dce5794a0/src/MetaMorphoV1_1.sol#L109),
   [IMetaMorphoV1_1.sol#L82-L85](https://github.com/morpho-org/metamorpho-v1.1/blob/b6bb715896da481f6c49f2d276a0ed7dce5794a0/src/interfaces/IMetaMorphoV1_1.sol#L82-L85)).
   V1 lets share price fall with Morpho Blue's bad-debt write-down.
2. Initial timelock may be 0; later values must still be 1 day to 2 weeks
   (`ConstantsLib.POST_INITIALIZATION_MIN_TIMELOCK`,
   [#L137](https://github.com/morpho-org/metamorpho-v1.1/blob/b6bb715896da481f6c49f2d276a0ed7dce5794a0/src/MetaMorphoV1_1.sol#L137),
   [#L741-L744](https://github.com/morpho-org/metamorpho-v1.1/blob/b6bb715896da481f6c49f2d276a0ed7dce5794a0/src/MetaMorphoV1_1.sol#L741-L744)).
3. Owner can rename the vault (`setName()`, `setSymbol()`).
4. `reallocate()` always reverts for a market that is not enabled; V1 checked a
   zero cap on the supply leg instead.

## 3. Morpho Vaults V2

All references are to
[VaultV2.sol](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol).

### Roles (Code)

| Role | Powers | Gate |
|---|---|---|
| Owner (1) | `setOwner()`, `setCurator()`, `setIsSentinel()`, name, symbol. Inherits nothing else. | `msg.sender == owner` ([#L273-L301](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol#L273-L301)) |
| Curator (1) | `submit()` any setter call data; `revoke()`; instant `decreaseAbsoluteCap()` / `decreaseRelativeCap()` | [#L306-L337](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol#L306-L337), [#L493-L521](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol#L493-L521) |
| Sentinels (many) | `revoke()`, both cap decreases, `deallocate()` | same lines plus [#L561-L563](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol#L561-L563) |
| Allocators (many) | `allocate()` within caps, `deallocate()`, `setLiquidityAdapterAndData()`, `setMaxRate()` | [#L533-L606](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol#L533-L606) |
| Anyone | Execute a submitted call after its delay; `forceDeallocate()` | [#L319-L327](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol#L319-L327), [#L789-L800](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol#L789-L800) |

The contract states the design rule in its header: the owner has no
action that directly hurts depositors, and the curator has none outside a
timelock
([#L135-L140](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol#L135-L140)).

### Timelocked setters (Code)

Each of these starts with the internal `timelocked()` check, so it runs only
from call data the curator submitted, after `timelock[selector]` seconds, by
anyone: `setIsAllocator()`, the four gate setters, `setAdapterRegistry()`,
`addAdapter()`, `removeAdapter()`, `increaseTimelock()`, `decreaseTimelock()`,
`abdicate()`, both fee setters, both fee-recipient setters,
`increaseAbsoluteCap()`, `increaseRelativeCap()`, `setForceDeallocatePenalty()`
([#L340-L530](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol#L340-L530)).

- **Per-selector timelock with no floor.** Defaults are 0 at creation
  ([#L148](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol#L148)).
  The contract has no minimum; protection depends on what the curator sets.
- **Decrease waits for the old value.** `submit()` charges a
  `decreaseTimelock()` call the current timelock of the target selector, not its
  own ([#L306-L317](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol#L306-L317)).
  An increase uses the timelock of `increaseTimelock()` itself, and a pending
  call keeps its original `executableAt`
  ([#L414-L431](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol#L414-L431)).
- **Abdication.** `abdicate()` sets a flag; `timelocked()` then reverts for
  that selector forever, whatever is pending
  ([#L434-L438](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol#L434-L438)).
- **Caps per id.** An adapter returns ids for each allocation (market,
  collateral, adapter). Allocation requires a non-zero absolute cap and stays
  under the absolute cap and under a relative cap of `firstTotalAssets`
  ([#L538-L559](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol#L538-L559)).
  Caps are checked only when allocation grows; interest can exceed them.
- **Gates.** Four optional contracts gate receiving shares, sending shares,
  withdrawing assets, and depositing assets. The header warns that three of
  them can lock users out of exit; the deposit gate cannot block funds
  ([#L113-L129](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol#L113-L129)).
- **Adapter registry.** If set, every current and future adapter must be in it;
  the guarantee is permanent only if the registry is add-only and the setter is
  abdicated ([#L71-L76](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol#L71-L76),
  [docs: adapter registry](https://docs.morpho.org/curate/concepts/adapter-registry)).
- **forceDeallocate.** Anyone can pull assets from an adapter back to idle for
  a user, paying a per-adapter penalty of at most 2 %
  (`ConstantsLib.MAX_FORCE_DEALLOCATE_PENALTY` in
  [ConstantsLib.sol#L9-L12](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/libraries/ConstantsLib.sol#L9-L12)).
  This is the in-kind exit that makes the timelock meaningful when markets are
  illiquid ([README](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/README.md)).
- **Fees.** Performance fee at most 50 % of interest, management fee at most
  5 % a year; both timelockable (same constants file).

### Instant versus timelocked summary

| Instant | Timelocked (curator-chosen delay, may be 0) |
|---|---|
| Cap decreases (curator, sentinel) | Cap increases |
| `revoke()` pending (curator, sentinel) | Adapters, adapter registry |
| `deallocate()` (allocator, sentinel) | Allocators, gates, fees, penalty |
| `allocate()` within caps (allocator) | Timelock changes, abdication |
| Role setting by owner (curator, sentinels) | |

Morpho's emergency page states the rule: functions that reduce risk are never
timelocked, and only risk-increasing functions are ([docs: emergency V2](https://docs.morpho.org/curate/tutorials-v2/emergency)).
Note that the owner can swap the curator instantly; the curator's pending
calls survive the swap.

## 4. Who holds the roles in practice

All rows are **On-chain**: role addresses from the Morpho API, address types
from `eth_getCode`, Safe `getThreshold()` / `getOwners()`, and EIP-1967 slots.
"7702 EOA" means the code is an EIP-7702 delegation stub (`0xef0100...`).

### MetaMorpho V1 vaults (Ethereum, factory 0xA9c3...1101)

| Vault | Owner | Curator | Guardian | Allocators | Timelock |
|---|---|---|---|---|---|
| [Steakhouse USDC](https://etherscan.io/address/0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB) | Safe 5/10 | Safe 3/9 | Aragon OSx DAO proxy (protocol 1.3.0) | 7: Safe 1/7, two 7702 EOAs, an EIP-2535 diamond, an EOA, an unidentified proxy, Public Allocator | 7 days |
| [Gauntlet WETH Prime](https://etherscan.io/address/0x2371e134e3455e0593363cBF89d3b6cf53740618) | Safe 4/7 | Safe 3/7 | Safe 3/7 | EOA, Public Allocator | 7 days |
| [MEV Capital wETH](https://etherscan.io/address/0x9a8bC3B04b7f3D87cfC09ba407dCED575f2d61D8) | Safe 2/5 | none (owner acts) | Aragon OSx DAO proxy | 3 EOAs | 3 days |
| [Re7 WETH](https://etherscan.io/address/0x78Fc2c2eD1A4cDb5402365934aE5648aDAd094d0) | Safe 2/4 | none (owner acts) | none | 2 EOAs | 3 days |

The guardian proxies
([Steakhouse](https://etherscan.io/address/0xaa0500198B4425DfC4E272FbE42C8E64E21fc03d),
[MEV Capital](https://etherscan.io/address/0x777498D046d593F747Fd5884A73B3b9964479711))
both point to implementation 0x52af...2d3a and answer `protocolVersion()` with
`[1, 3, 0]` (Aragon OSx DAO). Their voting setup is **Unverified**. The docs
list Aragon DAO as a guardian option
([docs: roles](https://docs.morpho.org/curate/concepts/roles)).

### Vaults V2 (largest by TVL on Ethereum and Base)

| Vault | Owner | Curator | Sentinels | Allocators |
|---|---|---|---|---|
| [Grove x Steakhouse USDC](https://basescan.org/address/0xbeef0786756810478b88982DE00F3CD7fdB8e7c7) (Base, about $538M) | Governance executor contract (queue/execute, `delay()` = 0, grace 7 days, guardian role) | Safe 2/2 | Safe 2/3 | 6: bot contracts, a 7702 EOA, Safe 1/7, Blue Public Allocator |
| [Steakhouse Prime USDC](https://etherscan.io/address/0xbeef088055857739C12CD3765F20b7679Def0f51) (about $149M) | "Supervisor" contract, `TIMELOCK()` = 14 days, owned by the Safe 5/10 | Safe 3/9 | the Supervisor | 7: bot contracts, 7702 EOA, Safe 1/7, Blue Public Allocator |
| [Gauntlet USDC Prime](https://etherscan.io/address/0x8c106EEDAd96553e64287A5A6839c3Cc78afA3D0) (about $101M) | Safe 4/7 | Safe 3/7 | EOA, Safe 3/7 | curator Safe, EOA |
| [Sentora RLUSD Main](https://etherscan.io/address/0x6dC58a0FdfC8D694e571DC59B9A52EEEa780E6bf) (about $430M) | Safe 1/1 | Safe 1/1 | 7702 EOA, EOA, curator Safe | 7702 EOA, curator Safe, EOA |

Selected V2 timelocks (seconds; API `timelocks` field):

| Vault | Cap raise | Add adapter | Fees | Allocator add | Exit gates |
|---|---|---|---|---|---|
| Grove x Steakhouse | 604800 (7 d) | 604800 | 259200 (3 d) | 0 | 0, but abdicated |
| Steakhouse Prime USDC | 604800 | 604800 | 0 | 0 | 604800, `setReceiveAssetsGate` abdicated |
| Gauntlet USDC Prime | 604800 | 604800 | 259200 | 604800 | 604800 |
| Sentora RLUSD Main | 259200 | 259200 | 0 | 0 | 0 (abdication not checked) |

Observations:

- The API reports an abdicated selector as duration 0. For Grove x Steakhouse,
  `abdicated(bytes4)` returns true for both share gates and the receive-assets
  gate. A 0 in the API is not proof of an instant power.
- The API reports `decreaseTimelock` as 0 for every vault. That raw slot is
  unused: the effective delay is the target selector's timelock (Code, §3).
- `setForceDeallocatePenalty` at 0 (not abdicated) on Grove x Steakhouse lets
  the curator raise the exit penalty to 2 % instantly.
- Typical delays: 3 days and 7 days. Steakhouse adds a 14-day owner-level
  delay through its Supervisor contract.

### Public Allocator (Code)

- **V1** ([PublicAllocator.sol](https://github.com/morpho-org/public-allocator/blob/aa324d81c2be9b25260ad865c18248e4099d274e/src/PublicAllocator.sol)):
  the vault owner adds it as an allocator. Anyone calls `reallocateTo()`
  (paying an ETH fee if set) to move liquidity from enabled markets into one
  enabled market. Each market has `maxIn` / `maxOut` flow caps set by the vault
  owner or a per-vault admin; a withdrawal lowers the source's `maxOut` and
  raises its `maxIn`, the supply market does the reverse
  ([#L86-L158](https://github.com/morpho-org/public-allocator/blob/aa324d81c2be9b25260ad865c18248e4099d274e/src/PublicAllocator.sol#L86-L158)).
  Vault supply caps still apply because the call goes through `reallocate()`.
- **V2** (`BluePublicAllocator.sol` in
  [vault-v2 periphery](https://github.com/morpho-org/vault-v2/tree/48da66bb64af98320c34483fcc42177645e44338/src/periphery/blue-public-allocator)):
  configured by vault allocators (`isAllocator` check), with active adapters,
  pull-from-market and pull-from-idle switches, its own absolute ceiling per
  market, and a penalty paid to the vault. Vault caps still bind. Addresses per
  chain: [docs: addresses](https://docs.morpho.org/developers/contracts/addresses).

### Docs and code disagree

| Topic | Docs | Code or chain |
|---|---|---|
| Morpho governance Safe | 5/9 ([organization](https://docs.morpho.org/learn/governance/organization)) | 6 of 10 on chain |
| V1 fee | Anyone accepts a new fee after the timelock; the guardian cannot revoke a pending fee ([roles](https://docs.morpho.org/curate/concepts/roles)) | `setFee()` is instant; no pending fee exists |
| V1 guardian set | Setting the guardian is timelocked | Instant when no guardian is set |
| V1 cap accept | Accepted by the owner or the curator | `acceptCap()` is callable by anyone |
| Timelock page | V1 submitter is the owner ([timelock](https://docs.morpho.org/curate/concepts/timelock)) | V1 cap and removal submits are curator (or owner) |
| Abdication | Sets the timelock to infinity | Separate `abdicated` flag; timelock unchanged |
| V2 gates | `setSharesGate` | Two setters: receive and send shares |
| V1 flow caps | [README](https://github.com/morpho-org/public-allocator/blob/aa324d81c2be9b25260ad865c18248e4099d274e/README.md): supply market lowers `maxOut`, raises `maxIn` | Code lowers `maxIn`, raises `maxOut` (L157-L158) |

## 5. Comparison with our curator registry

Our model (from `contracts/curator/src/ops.rs`, `auth.rs`, `lib.rs`, and the
governance diff): one curator contract holds governance `CURATOR_ROLE` and
`PROPOSER`. The protocol owner opens registrars (`ops::register_registrar`).
Each registrar has an admin and members with role bits MARKET, SPOKE, FLAGS.
Registrars own hubs and spokes. Instant: create hub/spoke/market,
`ops::lower_caps` (own spoke, strictly lower), `ops::tighten_flags` (own spoke;
the controller rejects relaxation). Timelocked through `ops::propose`: new
listing, listing edit, rate change, flag relaxation. `ops::require_grant`
checks a foreign hub's `curator::CapGrant` at propose time only. Delay tiers: Standard
and Sensitive are both 12 ledgers on mainnet
([threat model](../explanation/threat-model.md),
[networks.json](../../configs/networks.json)).

### Mapping

| Morpho concept | Our equivalent | Gap or difference |
|---|---|---|
| Blue owner whitelists IRM/LLTV, sets fee | Governance owner; `validate_spoke_asset` bounds | Similar. A new listing also needs a configured asset oracle (`require_asset_oracle`), which the owner sets; Blue lets the creator pick any oracle |
| Immutable market, no pause | Hubs, spokes, listings are editable; `paused` / `frozen` / `no_seize` flags | We can halt exit and liquidation; Morpho cannot |
| Vault owner (sets roles) | Protocol owner opens registrars; registrar admin sets member bits | Our registrar admin is instant and unbounded, like V1/V2 owner. No two-step admin transfer (`ops::set_admin`) |
| Curator | Registrar MARKET + SPOKE bits | Matches, but our curator also edits LTV, threshold, bonus of live positions; Morpho curators never touch market risk parameters |
| Allocator | None (users choose spokes) | Not needed: we are not a vault |
| Guardian / sentinel veto | Governance `CANCELLER` can cancel any pending op | Exists, but it is protocol-level, and 12 ledgers is too short to use it |
| Sentinel instant cap cut | `ops::lower_caps` (own spoke only) | Hub owner cannot cut a grantee's spoke caps for its own hub asset |
| Cap raise timelocked | Edit via `ops::propose` | Matches in the curator, but governance `edit_asset_in_spoke` is immediate for any `CURATOR_ROLE` holder with no direction check |
| Timelock floor 1 day (V1), curator-chosen (V2) | 12 ledgers (~1 minute), global | No supplier exit window |
| Decrease waits old timelock | `UpdateGovDelay` is Standard tier | A delay cut waits only the current minimum |
| Abdication | None | No way to commit permanently (for example, never raise a bonus) |
| A pending cap blocks any new cap for that market | Salted proposals; any number can be pending | Multiple pending edits for one listing can race |
| Gates, adapter registry | Spoke membership, governance oracle config | Not needed |
| forceDeallocate exit | User withdraw, unless `paused` | A registrar can block exit instantly |
| Public Allocator flow caps | `curator::CapGrant` (per hub asset, per grantee spoke) | Grant is checked once at propose; Morpho caps are checked on every allocation |

### Ranked design lessons (all are recommendations, not facts)

1. **Do not let a registrar halt liquidation or exit for debt it does not own.**
   Morpho markets cannot pause liquidation at all (§1), and V1 docs state the
   curator cannot pause withdrawals
   ([roles](https://docs.morpho.org/curate/concepts/roles)). In our controller,
   `no_seize` blocks seizure and `paused` blocks repayment and user exit
   (`contracts/controller/src/positions/mod.rs` `FreezePolicy`;
   `contracts/controller/src/positions/liquidation/plan.rs` line 13).
   Recommendation: keep `no_seize` (and `paused` on a debt leg) with the
   GUARDIAN only, or require that the registrar owns the hub of the listed
   asset before `ops::tighten_flags` may set it. **Addresses open issue 1**
   (grantee sets `no_seize` and blocks liquidation of the hub's debt).
   `paused` on a borrowed foreign-hub asset has the same effect on the repay
   leg (Inferred from the `plan.rs` comment; not tested here).
2. **Enforce the asymmetry where the state lives, not in the caller.** Morpho
   puts the rule (lower is instant, raise is timelocked) inside the vault that holds the
   caps. Recommendation: split governance `edit_asset_in_spoke` so the
   immediate path accepts only cap decreases and flag tightening (copy the
   current listing, as `ops::lower_caps` does), and route any raise of cap,
   LTV, threshold, bonus, or a new `can_borrow` / `can_collateral` through
   `propose`. **Addresses open issue 4** (any `CURATOR_ROLE` holder raises
   caps or LTV instantly). It also protects against a bad curator upgrade.
3. **Give suppliers a real exit window.** Morpho's floor is 1 day in V1;
   production vaults use 3 to 7 days for cap raises (§4). Recommendation: put
   registrar proposals in their own tier with a floor in ledgers that is a real
   time span (for example 1 to 3 days), independent of the protocol minimum,
   and make a reduction of that floor wait for the current floor.
4. **Re-check grants at execution time, and let the grantor revoke.** The
   Public Allocator re-checks flow caps on every call. Recommendation:
   governance execution of a registrar-proposed edit should call back into the
   curator (or the curator should keep the grant reference in the op) so a
   lowered or deleted grant fails the pending op; give the hub owner a
   `revoke_grant()` plus an instant cut of the grantee listing down to the new
   grant. **Addresses open issues 2 and 3** (grants checked only at propose;
   hub owner cannot lower a grantee's caps).
5. **Add a registrar-scoped veto (sentinel) distinct from the proposer.**
   Morpho separates curator (proposes) from guardian or sentinels (revoke, cut,
   never add). Recommendation: a registrar role bit, or the hub owner for
   grantee listings, that can cancel that registrar's pending ops through the
   curator, in addition to the protocol `CANCELLER`. Hold it in a different
   multisig from the admin.
6. **Separate who sets roles from who curates.** V2's owner cannot curate; V1's
   owner can do everything. Our registrar admin holds every bit
   (`auth::require_role`). Recommendation: keep admin able to set roles, but
   consider a two-step admin transfer and an event per role change, so a
   compromised admin key is visible before it acts.
7. **Offer abdication for registrar commitments.** V2 large vaults abdicate exit
   gates (§4). Recommendation: let a registrar irreversibly disable classes of
   change for its spoke (for example, raising bonus, enabling borrow of a
   foreign hub asset, granting caps). Cheap to implement as a stored bit
   checked in `ops::propose` and `ops::set_cap_grant`.
8. **One pending change per listing.** V1 refuses a second pending cap for a
   market. Recommendation: key registrar proposals by (spoke, hub asset) and
   reject a new one while one is pending, to remove ordering races between
   salted proposals.
9. **Monitor effective delays, not raw slots.** The API shows abdicated
   selectors and the `decreaseTimelock` slot as 0 (§4). Recommendation: our
   exporter and keeper should read effective delays and flags from contract
   views, not from config.
10. **Remove a compromised curator contract quickly.** `revoke_role_immediate`
    accepts only GUARDIAN and ORACLE, so removing `CURATOR_ROLE` from the
    curator contract needs a Sensitive-tier `RevokeGovRole`. Morpho's owner
    replaces a curator instantly. Recommendation: allow the owner to revoke
    `CURATOR_ROLE` immediately (revocation only reduces power).

### Open questions not resolved here

- Whether governance should expose a hub-owner-scoped cancel, or whether the
  curator contract should hold `CANCELLER` for its own proposals.
- The right registrar delay floor for mainnet; Morpho data supports 3 to 7 days
  for risk increases, but our users exit through spokes, not vault shares.

## Sources

Code (pinned commits above):

- https://github.com/morpho-org/morpho-blue/blob/b4af81dcd9e4e2f2d937cc57ab7f654c18f7c73c/src/Morpho.sol
- https://github.com/morpho-org/morpho-blue/blob/b4af81dcd9e4e2f2d937cc57ab7f654c18f7c73c/src/libraries/ConstantsLib.sol
- https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/MetaMorpho.sol
- https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/libraries/ConstantsLib.sol
- https://github.com/morpho-org/metamorpho-v1.1/blob/b6bb715896da481f6c49f2d276a0ed7dce5794a0/README.md
- https://github.com/morpho-org/metamorpho-v1.1/blob/b6bb715896da481f6c49f2d276a0ed7dce5794a0/src/MetaMorphoV1_1.sol
- https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol
- https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/README.md
- https://github.com/morpho-org/vault-v2/tree/48da66bb64af98320c34483fcc42177645e44338/src/periphery
- https://github.com/morpho-org/public-allocator/blob/aa324d81c2be9b25260ad865c18248e4099d274e/src/PublicAllocator.sol

Docs (fetched 2026-11-05):

- https://docs.morpho.org/learn/concepts/blue
- https://docs.morpho.org/learn/governance
- https://docs.morpho.org/learn/governance/organization
- https://docs.morpho.org/curate/concepts/roles
- https://docs.morpho.org/curate/concepts/timelock
- https://docs.morpho.org/curate/concepts/adapter-registry
- https://docs.morpho.org/curate/tutorials-v2/emergency
- https://docs.morpho.org/learn/concepts/public-allocator
- https://docs.morpho.org/developers/contracts/addresses

On-chain (2026-11-05):

- Morpho API `https://api.morpho.org/graphql`: `vaultByAddress` (V1 vaults) and
  `vaultV2s` ordered by TVL, fields owner, curator, guardian, sentinels,
  allocators, timelocks.
- JSON-RPC `eth_getCode`, Safe `getThreshold()` / `getOwners()`, EIP-1967
  implementation slot, `abdicated(bytes4)`, gate getters, Morpho `owner()` and
  `feeRecipient()`; explorer links inline in §1 and §4.

Repository (this branch, uncommitted at time of writing):

- `contracts/curator/src/{lib,ops,auth,api,storage}.rs`
- `git diff v1.1.0 -- contracts/governance/src interfaces/governance/src`
- [docs/explanation/threat-model.md](../explanation/threat-model.md),
  [configs/networks.json](../../configs/networks.json)
