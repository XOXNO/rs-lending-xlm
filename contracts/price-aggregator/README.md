# Price Aggregator

The protocol's single oracle entry: combines provider feeds into one WAD USD
price per `PriceKey`, failing closed when they disagree.

| | |
| --- | --- |
| Owner | Governance, set at construction; no transfer, accept or renounce |
| Called by | Controller: `prices` for risk checks, `quotes` for views; lifts `Address` to `PriceKey::Token` first |
| Client | [`interfaces/price-aggregator`](../../interfaces/price-aggregator); drops `Env` |

## Pipeline

```text
prices(keys) / quotes(keys)
  → Session::new · warm(keys)
  → for each key: resolve → Outcome
  → force (hard) | to_status (soft)
```

Gates, failure order and source formulas:
[formulas](../../docs/reference/formulas.md#price-sources).

A Chainlink source stores the proxy address and a 32-byte `data_id`, reading
`latest_round(data_id, 18)` once. It is an unsmoothed market leg; a missing,
frozen, malformed, non-positive, oversized or future-dated round fails closed.
The Stellar proxy has no bulk read, so session warming covers only
RedStone-format providers. Mainnet USDT0 allows its Chainlink leg 93,600
seconds (26 hours) for a 24-hour heartbeat; all other gates apply.

`ProviderRef` is in both the governance and price-aggregator ABIs. Run
`make <network> upgradeAll` before scheduling a Chainlink
`ConfigureAssetOracle`; the operator script checks both deployed ABIs and the
live round first. Editing JSON alone changes nothing on-chain.

## Curve LP collateral

`CurveStableLp` values plain 2–3-coin Curve StableSwap LP shares
([formula](../../docs/reference/formulas.md#price-sources)). Curve computes the
invariant per share, including amplification ramps and active reserves; the
aggregator resolves each coin's `PriceKey::Token` oracle and enforces
freshness, depth, cycle and LP sanity-band rules. The result is conservative
gross value, not one-coin redemption proceeds; risk parameters must allow for
fees and liquidation liquidity.

### Admission policy

- Only plain pools qualify. Metadata alone cannot establish the safety of
  rate-bearing/rebasing pools or metapools.
- Each read rechecks executable hash, ordered coins, live decimals and fixed
  rates `10^(36 - coin_decimals)`; a changed executable is unpriceable until
  reviewed and reconfigured.
- LP sources (Aquarius or Curve) are collateral-only; Curve's lowest-coin price
  understates LP debt. No contract enforces this rule. `configs/script.sh`
  refuses a borrowable LP listing (`refuse_borrowable_lp`, `validateConfigs`),
  so it is an operator guard.
- `MAX_CURVE_COINS` is 3; larger pools need own code and resource validation.
- Mainnet reviewed list: empty. Testnet: the two fixture executables, upstream
  source unknown ([fixture README](../swap-aggregator/tests/fixtures/curve/README.md)).

Trust assumptions:

- On an imbalanced pool, LP value depends on amplification A, which the
  factory admin can ramp, moving the mark either way but never above
  normalized reserves per share times the lowest coin price.
- The source pins the executable, not the factory: trust the factory admin as
  an Aquarius stable pool's admin.
- Trust each coin issuer as for a directly listed token (clawback, freeze,
  upgrade, decimals change).

### Listing a Curve pool

1. Review the pinned executable and tokens for plain-pool semantics.
2. Record it in `configs/<network>/curve_reviewed_pools.json`
   ([Reviewed-pool record](#reviewed-pool-record)).
3. Configure every coin oracle.
4. Set the LP `min_sanity_price_wad` at or below the smallest coin
   `min_sanity_price_wad` × lowest expected virtual price / WAD, so the LP band
   never trips before the coin bands.
5. Configure the LP token with `asset_decimals: 18` and exactly one
   `CurveStableLp` source.
6. Set `pool` (also the LP token), `coins` (exact pool order), `wasm_hash` (64
   hex characters in CLI JSON) and `min_pool_value_wad`.
7. Create the 18-decimal market with `can_be_borrowed: false`.
8. Enable collateral in the desired spoke.
9. Near the [resource limits](#curve-resource-bench), first measure the
   production RedStone adapter, Chainlink proxy and token executables.

Operator scripts enforce before proposing:

- `validateConfigs` and the direct `configureMarketOracle` proposal reject a
  `CurveStableLp` `wasm_hash` not listed for the network.
- `validateConfigs` rejects, and `addAssetToSpoke` / `editAssetInSpoke` refuse,
  a borrowable LP listing. Prices are keyed by address, so this covers every
  market (any hub or name) sharing the LP's address, and any market whose
  `Scaled` quote (Token or reference) is LP-priced.

#### Reviewed-pool record

Format:
`{"pools": [{wasm_hash, source_repo, source_commit, review, expected_factory}]}`.
`wasm_hash` is lowercase 64-hex. No field is empty; write `unknown` rather than
guess. Mainnet refuses `unknown`, and requires a 40-hex `source_commit`, a
`source_repo` naming the reviewed source (not `?`, nor containing `TODO`,
`TBD` or `n/a` in any case), and a factory contract address in
`expected_factory` (`^C[A-Z2-7]{55}$`).

### Curve resource bench

Curve tests use the real 2- and 3-coin pool WASMs; the generic
production-fixture runner rejects Curve sources.

- **Native case.** The harness liquidates `POSITION_LIMIT_MAX` 3-coin LP legs
  in the default Soroban budget, about 55M CPU
  ([threat model](../../docs/explanation/threat-model.md#numeric-and-resource-limits)),
  with native controller and feed, SAC coins, single-source coin oracles and
  one debt leg.
- **Worst case.** `curve_lp_liquidation_worst_case_all_wasm` runs the whole
  path as WASM: custom-WASM coins, two-source coin oracles, 5 debt legs,
  `mock_redstone.wasm` for both feeds, `production_fixture.wasm` for every coin
  and debt asset. Measured: 107.7M of 400,000,000 `tx_max_instructions`, 15.1M
  of 41,943,040 `tx_memory_limit`.

The bench asserts all eight `MAINNET_LIMITS` (mainnet transaction limits); the
other six are `tx_max_footprint_entries` 400, `tx_max_disk_read_entries` 200,
`tx_max_write_ledger_entries` 200, `tx_max_disk_read_bytes` 200,000,
`tx_max_write_bytes` 132,096, `tx_max_contract_events_size_bytes` 16,384.
Production RedStone, Chainlink and token executables can cost more than the
fixtures.

## Entrypoints

Signatures and authority: [endpoint reference](../../docs/reference/endpoints.md#price-aggregator).
`prices` and `quotes` are bulk only; pass a `Vec<PriceKey>` even for one key.
`price_spread` returns the two leg prices (WAD).

`set_sanity_band` reverts `SanityBandMustTighten` on a wider band. It and
`set_tolerance` probe only for configuration errors, so a value the live price
fails is still committed. Governance calls `set_sanity_band` immediately; to
widen a band, use the timelocked `ConfigureAssetOracle` (calls `set_oracle`).
A `set_oracle` replacement changing `asset_decimals` fails
`InvalidOracleDecimals`. `upgrade` renews the instance TTL first.
`seed_oracle` and `remove_oracle` write the registry with no auth, validation
or attestation; they exist only under `cfg(test)` or the `testing` feature.

## Storage and TTL

| Key | Tier | Holds |
| --- | --- | --- |
| `AggregatorKey::Oracle(PriceKey)` | persistent | One `AssetOracle` configuration |
| `AggregatorKey::OracleKeys` | instance | Index of every registered `PriceKey` |
| OpenZeppelin `Ownable` owner | instance | Governance |

Every entrypoint, views included, renews the instance TTL (`renew_instance`);
every `Oracle` read and write extends with `TTL_THRESHOLD_SHARED` /
`TTL_BUMP_SHARED`.

## Events

`set_oracle`, `set_sanity_band` and `set_tolerance` publish
`UpdateAssetOracleEvent` with the committed configuration
([events](../../docs/reference/events.md#price-aggregator-events)).

## Layout

```text
lib.rs          # entrypoints + session orchestration
session.rs      # clock, stack, multi-feed warm, memos
engine.rs       # resolve → Outcome → force | to_status
admin.rs        # set_oracle / set_sanity_band / set_tolerance, attest, cascade
registry.rs     # persistent oracle storage, key index, config event
validation.rs   # write-time config checks (sources, depth, staleness, decimals)
tolerance.rs    # dual-source disagreement bounds
observation.rs  # provider payload → normalized WAD observation
properties.rs   # write-time dependency walk
providers/      # aquarius + curve (LP) + chainlink + multi_feed (bulk) + reflector
```

Formal proofs: [`certora/README.md`](../../certora/README.md) (price system).
