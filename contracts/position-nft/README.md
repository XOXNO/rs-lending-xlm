# Position NFT

Ownership record for lending accounts. Every controller account is exactly one
token in this collection, and the token id is the account id. The controller
stores no owner address for an account; it calls `owner_of(account_id)` on this
contract every time it needs to know who may act on a position. A lending
position is therefore an ordinary transferable non-fungible token (NFT), so
wallets, indexers, and marketplaces can read and move it with no
protocol-specific tooling.

| | |
| --- | --- |
| Base standard | OpenZeppelin `stellar-tokens` 0.7.1 (git rev `fbfde388`), non-fungible |
| Extension | `Enumerable` (`type ContractType = Enumerable;`) with sequential ids |
| Not used | `Consecutive`, `Burnable` |
| Interface | [`interfaces/position-nft`](../../interfaces/position-nft) |
| Deployed by | Controller, at salt `[1u8; 32]`, one-shot |

## Role in the protocol

The controller deploys this contract once through `deploy_position_nft` and
passes its own address as the constructor's `controller`. That address is the
only one allowed to mint, burn, and upgrade.

| Controller action | NFT call | Effect |
| --- | --- | --- |
| `create_account` | `mint(owner)` | The returned `u32` token id becomes the `u64` account id |
| account deletion (`remove_account_and_burn_nft`) | `burn(token_id)` | Runs on every account deletion, including liquidation cleanup and bad-debt socialization |
| any owner check (`try_account_owner`) | `owner_of(token_id)` | Live lookup; the owner is never cached in controller storage |
| `renew_account` | `renew(token_id)` | Lifts the token's `Owner` entry to the protocol's per-user window |
| `upgrade_position_nft` | `upgrade(hash)` | Owner-gated Wasm upgrade |

`account_id == token_id`. The controller widens `u32` to `u64` on mint and
narrows back with `u32::try_from` on every other call; an id above `u32::MAX`
can never have been minted, so it resolves to `AccountNotFound`. Account id `0`
is the controller's "create a new account" sentinel, so the constructor
consumes token id 0 and the first real position is id 1.

Transferring the token transfers the whole position. Nothing in the controller
changes on transfer: the next controller call resolves the new holder and
accepts it. Collateral and debt both move with the token.

## Entrypoints

Own entrypoints, defined in [`src/contract.rs`](src/contract.rs):

| Call | Signature | Caller | Does |
| --- | --- | --- | --- |
| `__constructor` | `fn __constructor(e: &Env, controller: Address, uri: String, name: String, symbol: String)` | Deployer, once | Stores `controller`, sets collection metadata, consumes token id 0 |
| `mint` | `fn mint(e: &Env, to: Address) -> u32` | `controller` only | Mints the next sequential id to `to`, returns it, renews instance TTL |
| `burn` | `fn burn(e: &Env, token_id: u32)` | `controller` only | Clears owner, balance, approval, and enumeration; emits `Burn`; renews instance TTL |
| `renew` | `fn renew(e: &Env, token_id: u32)` | Anyone | Extends `Owner(token_id)` to the per-user window and renews instance TTL |
| `upgrade` | `fn upgrade(e: &Env, new_wasm_hash: BytesN<32>)` | `controller` only | Renews instance TTL, replaces the contract Wasm |
| `token_uri` | `fn token_uri(e: &Env, token_id: u32) -> String` | Anyone | Overrides the standard default; returns `{base_uri}{token_id}?isStatic=true&chain=STELLAR` |

Inherited from the OpenZeppelin `NonFungibleToken` trait, exported unchanged:

| Call | Signature | Caller | Does |
| --- | --- | --- | --- |
| `balance` | `fn balance(e: &Env, account: Address) -> u32` | Anyone | Number of positions held by `account` |
| `owner_of` | `fn owner_of(e: &Env, token_id: u32) -> Address` | Anyone | Current holder; panics `NonExistentToken` if never minted or burned |
| `transfer` | `fn transfer(e: &Env, from: Address, to: Address, token_id: u32)` | `from` must authorize | Moves the position to `to` |
| `transfer_from` | `fn transfer_from(e: &Env, spender: Address, from: Address, to: Address, token_id: u32)` | `spender` must authorize and be approved for the token or an operator for `from` | Moves the position to `to` |
| `approve` | `fn approve(e: &Env, approver: Address, approved: Address, token_id: u32, live_until_ledger: u32)` | `approver` must authorize and be the owner or an operator | Grants `approved` the right to move that one position until `live_until_ledger` |
| `approve_for_all` | `fn approve_for_all(e: &Env, owner: Address, operator: Address, live_until_ledger: u32)` | `owner` must authorize | Makes `operator` able to move every position `owner` holds until `live_until_ledger`; `0` revokes |
| `get_approved` | `fn get_approved(e: &Env, token_id: u32) -> Option<Address>` | Anyone | Live per-token approval, if any |
| `is_approved_for_all` | `fn is_approved_for_all(e: &Env, owner: Address, operator: Address) -> bool` | Anyone | Whether `operator` may move all of `owner`'s positions |
| `name` | `fn name(e: &Env) -> String` | Anyone | Collection name from metadata |
| `symbol` | `fn symbol(e: &Env) -> String` | Anyone | Collection symbol from metadata |

Inherited from the `NonFungibleEnumerable` extension, exported unchanged:

| Call | Signature | Caller | Does |
| --- | --- | --- | --- |
| `total_supply` | `fn total_supply(e: &Env) -> u32` | Anyone | Number of live positions |
| `get_owner_token_id` | `fn get_owner_token_id(e: &Env, owner: Address, index: u32) -> u32` | Anyone | Walks one holder's positions; pair with `balance` |
| `get_token_id` | `fn get_token_id(e: &Env, index: u32) -> u32` | Anyone | Walks all live positions; pair with `total_supply` |

Errors are the stock `NonFungibleTokenError` codes 200–214.
[`interfaces/position-nft/src/lib.rs`](../../interfaces/position-nft/src/lib.rs)
declares only the subset the controller calls: `mint`, `burn`, `owner_of`,
`renew`, `upgrade`.

## Storage and TTL

| Key | Tier | Holds |
| --- | --- | --- |
| `DataKey::Controller` | instance | The only address allowed to mint, burn, and upgrade |
| `NFTStorageKey::Metadata` | instance | `base_uri`, `name`, `symbol` |
| `NFTSequentialStorageKey::TokenIdCounter` | instance | Next free token id |
| `NFTEnumerableStorageKey::TotalSupply` | instance | Live token count |
| `NFTStorageKey::Owner(token_id)` | persistent | The ownership record |
| `NFTStorageKey::Balance(address)` | persistent | Per-holder count |
| `NFTEnumerableStorageKey::OwnerTokens` / `OwnerTokensIndex` / `GlobalTokens` / `GlobalTokensIndex` | persistent | Enumeration lists |
| `NFTStorageKey::Approval(token_id)` | temporary | Per-token approval, expires at `live_until_ledger` |
| `NFTStorageKey::ApprovalForAll(owner, operator)` | temporary | Operator approval, expires at `live_until_ledger` |

`mint`, `burn`, `upgrade`, and `renew` all call `renew_instance`, which extends
instance storage using the protocol constants `TTL_THRESHOLD_INSTANCE` and
`TTL_BUMP_INSTANCE` (180-day bump). Mint and burn run on every controller
account creation and deletion, so instance storage stays alive on protocol
traffic alone.

Per-token `Owner` entries renew on two different schedules. Any read through
`owner_of` extends `Owner(token_id)` by the OpenZeppelin default of 30 days.
`renew(token_id)` extends it with `TTL_THRESHOLD_USER` and `TTL_BUMP_USER`, a
120-day bump matching the controller's own per-user entries. `renew` takes no
authorization from anyone: extending a lifetime moves no state, cannot shorten
a lifetime, and cannot move or approve a token.

If `Owner(token_id)` archives, `owner_of` no longer resolves until the entry is
restored. A caller that simulates the transaction and then submits it is not
blocked — the simulation returns the archived entry ids and the submitted
operation restores them in line, at the cost of restore rent. Liquidation
therefore still succeeds against a lapsed `Owner` entry. Only a caller that
hand-builds a footprint without simulating needs an explicit `RestoreFootprint`;
the permissionless `renew(token_id)` above avoids the situation entirely. This
asymmetry is recorded as INV-STOR-02 in
[`docs/reference/invariants.md`](../../docs/reference/invariants.md).

## Events

The contract publishes no bespoke events. Every event is a standard
OpenZeppelin token event.

| Event | Topics | Data | Emitted by |
| --- | --- | --- | --- |
| `Transfer` | `from: Address`, `to: Address` | `token_id: u32` | `transfer`, `transfer_from` |
| `Approve` | `approver: Address`, `token_id: u32` | `approved: Address`, `live_until_ledger: u32` | `approve` |
| `ApproveForAll` | `owner: Address` | `operator: Address`, `live_until_ledger: u32` | `approve_for_all` |
| `Mint` | `to: Address` | `token_id: u32` | `mint` |
| `Burn` | `from: Address` | `token_id: u32` | `burn` |

A burn emits `Burn` only. It does not emit a `Transfer` to a zero address,
because `burn` clears the owner through `Base::update`, which publishes
nothing, and then calls `emit_burn` directly.

## Security rules

**Mint and burn are controller-only.** Both call
`controller(e).require_auth()`, where `controller` is the address fixed at
construction. No other address can create or destroy a position.

**Burn does not require the holder's authorization.** The contract does not use
the OpenZeppelin `Burnable` extension, because `Base::burn` calls
`from.require_auth()`. The controller must be able to delete an account that
emptied through liquidation, where the holder never signed. `burn` therefore
reimplements `Enumerable::burn` without that check. The holder cannot burn
their own token either: no holder-facing burn entrypoint exists.

**Token ids are never reused.** Ids come from `increment_token_id`, a
monotonic instance counter. `burn` does not decrement it. A burned id can never
be minted again, so a deleted account id cannot be resurrected.

**A burned token is inert.** `burn` removes `Owner(token_id)`, so `owner_of`,
`transfer`, `transfer_from`, `renew`, and `token_uri` all panic with
`NonExistentToken`.

**Transfer cannot be used to escape debt.** The token carries the account, and
the account carries both collateral and debt. The controller keys every
solvency check on `account_id`, not on holder identity, so an underwater
position stays liquidatable after transfer and the new holder must repay before
withdrawing.

**Approval hands over the whole account.** `approve` and `approve_for_all` let
another address move the position, and moving the position moves the collateral
and the debt with it. The blast radius is the entire loan account. See the
Controller ↔ Position NFT section of
[`docs/explanation/threat-model.md`](../../docs/explanation/threat-model.md).

**Delegates lapse on transfer.** The controller stores a `DelegateGrant`
stamped with the `granted_by` address. `get_delegates` returns an empty list
unless `granted_by` equals the current NFT owner, so a transfer disables the
old holder's delegates immediately. `remove_delegate` deletes a stale grant
outright, so it cannot re-arm if the token later returns to the address that
granted it.

**Upgrade is governance-reachable only.** `upgrade` requires controller
authorization, and the only controller path is the owner-gated
`upgrade_position_nft`, which governance runs as a sensitive, timelocked
operation.

## Source

```text
contracts/position-nft/src/
  lib.rs        # crate root; exports PositionNft and PositionNftClient
  contract.rs   # constructor, mint, burn, renew, upgrade, token_uri override,
                # NonFungibleToken and NonFungibleEnumerable trait exports
  test.rs       # unit tests: id 0 reservation, auth gates, TTL windows, token_uri

interfaces/position-nft/src/
  lib.rs        # PositionNftInterface — the controller-facing client ABI
```

Controller side: [`external/position_nft.rs`](../controller/src/external/position_nft.rs)
(call wrappers and id widening), [`storage/account.rs`](../controller/src/storage/account.rs)
(owner resolution, delegate grants), [`markets.rs`](../controller/src/markets.rs)
(deploy and upgrade). Integration tests:
[`tests/test-harness/tests/controller/position_nft.rs`](../../tests/test-harness/tests/controller/position_nft.rs).
