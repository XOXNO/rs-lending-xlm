# Position NFT

Ownership record for lending accounts. Each controller account is one token,
and the token id is the account id. The controller stores no owner; it calls
`owner_of(account_id)` each time it checks who may act. A position is therefore
a plain transferable NFT that wallets, indexers and marketplaces can read and
move.

| | |
| --- | --- |
| Base standard | OpenZeppelin `stellar-tokens` 0.7.1 (git rev `59b98f8e`), non-fungible |
| Extension | `Enumerable` (`type ContractType = Enumerable;`) with sequential ids |
| Not used | `Consecutive`, `Burnable` |
| Client | [`interfaces/position-nft`](../../interfaces/position-nft) |
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
| `renew_account` | `renew(token_id)` | Lifts the token's `Owner` entry and its holder's `Balance` entry to the protocol's per-user window |
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

Own entrypoints are in [`src/contract.rs`](src/contract.rs); the generated
client drops the `Env` argument.

| Entrypoint | Caller | Does |
| --- | --- | --- |
| `__constructor(controller, uri, name, symbol)` | deployer, once | Stores `controller`, sets metadata, consumes token id 0 |
| `mint(to) -> u32` | controller | Mints the next id; extends `Owner` and `Balance` to the per-user window |
| `burn(token_id)` | controller | Removes owner, approval and enumeration entries; decrements `Balance` and total supply; emits `Burn` |
| `renew(token_id)` | anyone | Extends `Owner(token_id)` and the holder's `Balance` to the per-user window |
| `upgrade(new_wasm_hash)` | controller | Replaces the Wasm |
| `token_uri(token_id) -> String` | anyone | `{base_uri}{token_id}?isStatic=true&chain=STELLAR` |

`mint`, `burn`, `renew` and `upgrade` also renew the instance TTL.

Inherited unchanged from OpenZeppelin `NonFungibleToken` and
`NonFungibleEnumerable`:

| Entrypoint | Caller | Does |
| --- | --- | --- |
| `transfer(from, to, token_id)` | `from` | Moves the position |
| `transfer_from(spender, from, to, token_id)` | `spender`: `from`, approved for the token, or an operator for `from` | Moves the position |
| `approve(approver, approved, token_id, live_until_ledger)` | `approver`: owner or operator | Lets `approved` move this position until `live_until_ledger` |
| `approve_for_all(owner, operator, live_until_ledger)` | `owner` | Lets `operator` move all of `owner`'s positions; `0` revokes |
| `owner_of(token_id)` | anyone | Current holder; panics `NonExistentToken` if never minted or burned |
| `balance(account)` | anyone | Positions held by `account` |
| `get_approved(token_id)`, `is_approved_for_all(owner, operator)` | anyone | Live approvals |
| `name()`, `symbol()` | anyone | Collection metadata |
| `total_supply()` | anyone | Live positions |
| `get_owner_token_id(owner, index)` | anyone | Walks one holder's positions; pair with `balance` |
| `get_token_id(index)` | anyone | Walks all positions; pair with `total_supply` |

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

Per-token `Owner` entries renew on two schedules. Any read through `owner_of`
extends `Owner(token_id)` to the OpenZeppelin default of 30 days. `mint` and
`renew(token_id)` extend `Owner(token_id)` and the holder's `Balance` entry with
`TTL_THRESHOLD_USER` and `TTL_BUMP_USER`, a 120-day bump matching the
controller's own per-user entries. Neither call extends the enumeration
entries. `renew` requires no authorization: extending a lifetime moves no
state, cannot shorten a lifetime, and cannot move or approve a token.

If `Owner(token_id)` archives, `owner_of` no longer resolves until the entry is
restored. A caller that simulates the transaction and then submits it is not
blocked — the simulation returns the archived entry ids and the submitted
operation restores them in line, at the cost of restore rent. Liquidation
therefore still succeeds against a lapsed `Owner` entry. Only a caller that
hand-builds a footprint without simulating needs an explicit `RestoreFootprint`.
A call to the permissionless `renew(token_id)` before expiry prevents the
archival. This asymmetry is recorded as INV-STOR-02 in
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

## Security properties

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
`transfer`, `approve`, `renew`, and `token_uri` panic with `NonExistentToken`.
`transfer_from` also fails: with error 202 (insufficient approval) when
`spender` is neither `from` nor an operator for `from`, else with
`NonExistentToken`.

**Transfer cannot be used to escape debt.** The token carries the account, and
the account carries both collateral and debt. The controller keys every
solvency check on `account_id`, not on holder identity, so an underwater
position stays liquidatable after transfer, and the new holder can withdraw only
what leaves the debt covered by LTV-weighted collateral.

**Approval hands over the whole account.** `approve` and `approve_for_all` let
another address move the position, and moving the position moves the collateral
and the debt with it. See
[Account authority](../../docs/explanation/threat-model.md#account-authority)
in the threat model.

**Delegates lapse on transfer.** The controller stores a `DelegateGrant`
stamped with the `granted_by` address. `get_delegates` returns an empty list
unless `granted_by` equals the current NFT owner, so a transfer disables the
old holder's delegates immediately. The stale grant stays in storage and
re-arms if the token returns to `granted_by`. The next holder's `add_delegate`
overwrites it and `remove_delegate` deletes it.

**Upgrade is governance-reachable only.** `upgrade` requires controller
authorization, and the only controller path is the owner-gated
`upgrade_position_nft`, which governance runs as a sensitive, timelocked
operation.

## Layout

```text
contracts/position-nft/src/
  lib.rs        # crate root; exports PositionNft and PositionNftClient
  contract.rs   # constructor, mint, burn, renew, upgrade, token_uri,
                # NonFungibleToken and NonFungibleEnumerable exports
  test.rs       # unit tests: id 0 reservation, auth gates, TTL windows, token_uri
```

Controller side: [`external/position_nft.rs`](../controller/src/external/position_nft.rs)
(call wrappers, id widening), [`storage/account.rs`](../controller/src/storage/account.rs)
(owner resolution, delegate grants), [`markets.rs`](../controller/src/markets.rs)
(deploy, upgrade).

## References

- Invariants (INV-STOR-02): [`docs/reference/invariants.md`](../../docs/reference/invariants.md)
- Threat model, account authority: [`docs/explanation/threat-model.md`](../../docs/explanation/threat-model.md#account-authority)
- Integration tests: [`tests/test-harness/tests/controller/position_nft.rs`](../../tests/test-harness/tests/controller/position_nft.rs)
