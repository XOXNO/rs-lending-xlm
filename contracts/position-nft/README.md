# Position NFT

The position NFT is the ownership record for lending accounts. Each controller
account has one token, and the token id equals the account id. The controller
stores no owner. It calls `owner_of(account_id)` on every authority check. A
position is a plain transferable NFT that wallets, indexers, and marketplaces
can read and move.

| | |
| --- | --- |
| Base standard | OpenZeppelin `stellar-tokens` 0.8.0, non-fungible |
| Extension | `Enumerable` (`type ContractType = Enumerable;`) with sequential ids |
| Not used | `Consecutive`, `Burnable` |
| Client | [`interfaces/position-nft`](../../interfaces/position-nft) |
| Deployed by | Controller, at salt `[1u8; 32]`, one-shot |

## Role in the protocol

The controller deploys the position NFT once through `deploy_position_nft`. It
passes its own address as `controller`. That address is the only one allowed to
mint, burn, and upgrade.

| Controller action | NFT call | Effect |
| --- | --- | --- |
| `create_account` | `mint(owner)` | Returned `u32` token id becomes the `u64` account id |
| account deletion (`remove_account_and_burn_nft`) | `burn(token_id)` | Every deletion, including liquidation cleanup and bad-debt socialization |
| any owner check (`try_account_owner`) | `owner_of(token_id)` | Live lookup. The owner is never cached |
| `renew_account` | `renew(token_id)` | Lifts `Owner` and holder's `Balance` to the per-user window |
| `upgrade_position_nft` | `upgrade(hash)` | Owner-gated Wasm upgrade |

The controller widens `u32` to `u64` on mint and narrows with
`u32::try_from` elsewhere. An id above `u32::MAX` was never minted and resolves
to `AccountNotFound`. Account id `0` means "create a new account", so the
constructor consumes token id 0 and the first position is id 1.

A transfer of the token moves the whole position, including collateral and
debt. The controller changes nothing on transfer. Its next call resolves the
new holder.

## Entrypoints

The contract's own entrypoints are in [`src/contract.rs`](src/contract.rs). The
[endpoint reference](../../docs/reference/endpoints.md#position-nft) lists
signatures and authority, including the inherited OpenZeppelin
`NonFungibleToken` and `NonFungibleEnumerable` methods. The generated client
drops `Env`.
[`interfaces/position-nft/src/lib.rs`](../../interfaces/position-nft/src/lib.rs)
declares only what the controller calls: `mint`, `burn`, `owner_of`, `renew`,
and `upgrade`. The errors are the stock `NonFungibleTokenError` 200–214.

`owner_of` and `get_approved` panic `NonExistentToken` for an id never minted
or burned. To list a holder's positions, use `get_owner_token_id` and `balance`.
To list all positions, use `get_token_id` and `total_supply`.

## Storage and TTL

| Key | Tier | Holds |
| --- | --- | --- |
| `DataKey::Controller` | instance | Sole minter, burner, upgrader |
| `NFTStorageKey::Metadata` | instance | `base_uri`, `name`, `symbol` |
| `NFTSequentialStorageKey::TokenIdCounter` | instance | Next free token id |
| `NFTEnumerableStorageKey::TotalSupply` | instance | Live token count |
| `NFTStorageKey::Owner(token_id)` | persistent | Ownership record |
| `NFTStorageKey::Balance(address)` | persistent | Per-holder count |
| `NFTEnumerableStorageKey::OwnerTokens` / `OwnerTokensIndex` / `GlobalTokens` / `GlobalTokensIndex` | persistent | Enumeration lists |
| `NFTStorageKey::Approval(token_id)` | temporary | Per-token approval until `live_until_ledger` |
| `NFTStorageKey::ApprovalForAll(owner, operator)` | temporary | Operator approval until `live_until_ledger` |

`mint`, `burn`, `upgrade`, and `renew` call `renew_instance`
(`TTL_THRESHOLD_INSTANCE`, `TTL_BUMP_INSTANCE`, 180 days), so account creation
and deletion alone keep the instance alive. `owner_of` extends `Owner` to the
OpenZeppelin default of 30 days. `mint` and `renew(token_id)` extend `Owner`
and the holder's `Balance` with `TTL_THRESHOLD_USER` / `TTL_BUMP_USER` (120
days, the same as the controller's per-user entries), not the enumeration
entries. `renew` needs no auth. It moves no state and cannot shorten a
lifetime.

An archived `Owner` does not block a caller that simulates first. Simulation
returns the archived entries, and the transaction restores them in line, for
restore rent. Liquidation therefore succeeds against a lapsed `Owner`. Only a
hand-built footprint needs `RestoreFootprint`. To prevent archival, call
`renew(token_id)` before expiry
([INV-STOR-02](../../docs/reference/invariants.md#inv-stor-02)).

## Events

The contract has no bespoke events. OpenZeppelin token events are in
[inherited events](../../docs/reference/events.md#inherited-events). A burn
emits only `Burn`, with no zero-address `Transfer`. `burn` clears the owner
through `Base::update` (silent), then calls `emit_burn`.

## Security properties

**Mint and burn are controller-only.** Both call
`controller(e).require_auth()` on the address fixed at construction.

**Burn needs no holder auth.** OpenZeppelin `Burnable` is unused because
`Base::burn` calls `from.require_auth()`, and the controller must delete
accounts emptied by liquidation without the holder's signature. `burn`
reimplements `Enumerable::burn` without that check. No holder-facing burn
exists.

**Token ids are never reused.** `increment_token_id` is a monotonic instance
counter that `burn` does not decrement, so a deleted account id cannot return.

**Calls on a burned token panic.** With `Owner(token_id)` removed, `owner_of`,
`transfer`, `approve`, `renew`, and `token_uri` panic `NonExistentToken` (200).
`transfer_from` on a burned or never-minted id also fails with
`NonExistentToken` (200) for every spender, because the approval check reads
the owner first.

**A transfer does not clear debt.** The account carries collateral and debt, and
every solvency check keys on `account_id`, not the holder. An underwater
position stays liquidatable after transfer. The new holder can withdraw only
what keeps debt covered by LTV-weighted collateral.

**Approval hands over the whole account.** `approve` and `approve_for_all` let
another address move the position with its collateral and debt
([Account authority](../../docs/explanation/threat-model.md#account-authority)).

**A transfer suspends delegates but does not revoke them.** The controller
stores one `DelegateGrant` per account, stamped with `granted_by`, and honors
it only while `granted_by` owns the NFT. If the token returns to `granted_by`
before a later holder changes the list, those delegates act again. The owner
ends a delegate's authority with `remove_delegate` before the transfer. A later
holder clears a dormant grant with `add_delegate` (replaces it) or
`remove_delegate` (deletes it, no event). No view lists an account's
delegates. Rebuild them from `AccountDelegateEvent`.

**Upgrade is governance-reachable only.** `upgrade` needs controller auth,
reachable only through owner-gated `upgrade_position_nft`, a sensitive
timelocked governance operation.

## Layout

```text
contracts/position-nft/src/
  lib.rs        # crate root; exports PositionNft and PositionNftClient
  contract.rs   # constructor, mint, burn, renew, upgrade, token_uri,
                # NonFungibleToken and NonFungibleEnumerable exports
  test.rs       # unit tests: id 0 reservation, auth gates, TTL windows, token_uri
```

The controller side is in [`external/position_nft.rs`](../controller/src/external/position_nft.rs)
(call wrappers, id widening), [`storage/account.rs`](../controller/src/storage/account.rs)
(owner resolution, delegate grants), and [`markets.rs`](../controller/src/markets.rs)
(deploy, upgrade).

The integration tests are in [`tests/test-harness/tests/controller/position_nft.rs`](../../tests/test-harness/tests/controller/position_nft.rs).
