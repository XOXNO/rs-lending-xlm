# Oracle V2 Threat Model

Scope: `controller/src/oracle`, `controller/src/config.rs` oracle configuration, and deployment config files.

## System Model

The controller stores one oracle config per market. Runtime code reads a normalized observation from the configured provider, composes primary plus optional anchor, and returns `PriceFeed { price_wad, asset_decimals, timestamp }` to accounting code. Configuration is controlled by the `ORACLE` role through `configure_market_oracle`, which probes provider contracts before activating a market.

Supported providers are Reflector SEP-40 and RedStone multi-feed. Reflector can be spot or TWAP and is validated for `base() == Other("USD")`. RedStone is spot-only and is assumed USD-quoted by `feed_id`.

## Trust Boundaries

- Controller to Reflector contract: external SEP-40 calls for `base`, `decimals`, `resolution`, `lastprice`, and `prices`.
- Controller to RedStone adapter: external calls for `read_price_data_for_feed`.
- ORACLE/admin role to market config: privileged but still high-risk input because a bad oracle config directly controls collateral/debt valuation.
- Indexer/API boundary: `UpdateAssetOracleEvent` exposes generic provider fields; downstream consumers must branch by provider and chain.
- Off-chain feed operators to on-chain adapter: RedStone liveness and signer threshold are outside controller control.

## Assets

- Solvency and liquidation correctness.
- User liveness for repay, withdraw, and view paths.
- Market activation safety.
- Oracle configuration integrity.
- Indexer/API correctness for risk monitoring and UI warnings.

## Attacker Capabilities

- Malicious or malfunctioning provider contract can return wrong prices, timestamps, decimals, or history.
- Compromised ORACLE role can configure wrong feed ids/contracts within the accepted shape.
- External oracle downtime can make configured anchors missing or stale.
- Users can call market operations that force price reads under different `OraclePolicy` values.

## Key Threats

- Wrong-quote RedStone feed configured as USD: controller cannot verify RedStone quote metadata on-chain, so a non-USD or fundamental feed can pass if positive and fresh.
- Anchor liveness failure: a stale or missing anchor can still freeze strict paths by design, but permissive paths now fall back with tolerance flags set to false.
- Single-source misconfiguration: RedStone-only or Reflector spot-only markets can be configured by policy, reducing independence of price validation.
- Staleness window mismatch between Reflector primary and RedStone anchor is mitigated by source-specific RedStone freshness limits.

## Mitigations To Add

- Add an explicit RedStone USD feed allowlist or source metadata validation path.
- Add an explicit `anchor_available`/source-status field to public views instead of overloading tolerance booleans.
