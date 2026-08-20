# 0002. One pool, isolated markets

**Status:** Accepted

**Implemented by:** common/src/types/controller.rs (`HubAssetKey`), contracts/pool/src/ops/market.rs (per-market `supply_index`, cash, shares, and revenue), contracts/controller/src/config/asset.rs (`upsert_spoke_asset`, `AssetAlreadyInSpoke`), contracts/controller/src/storage/spoke.rs.

## Decision

The protocol uses one central pool to custody all listed assets. Each market is
identified by a hub-asset key, not merely a token address.

A market therefore owns its own cash, supply and borrow shares, interest
indexes, revenue, and rate configuration. Listing the same token twice under
different hub assets creates distinct accounting domains.

## Guarantees

- Value and debt never move between markets merely because their token address
  is the same.
- A single pool simplifies custody and controlled outbound transfers.
- Per-market accounting remains isolated inside that custody boundary.

## Auditor focus

Test cross-market operations, duplicate token listings, and market-key
selection at every accounting boundary.
