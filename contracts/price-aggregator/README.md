# Price Aggregator

The single oracle entry of the lending protocol. It combines provider feeds
into one WAD USD price per `PriceKey` and fails closed when they disagree.

| | |
| --- | --- |
| Owner | Governance, set at construction. No transfer, accept or renounce |
| Called by | Controller: `prices` for risk checks, `quotes` for views. It lifts `Address` to `PriceKey::Token` first |
| Client | [`interfaces/price-aggregator`](../../interfaces/price-aggregator) |

Full signatures are in `contracts/price-aggregator/src/lib.rs`; the generated
client drops the `Env` argument.

## Pipeline

```text
prices(keys) / quotes(keys)
  → Session::new · warm(keys)
  → for each key: resolve → Outcome
  → force (hard) | to_status (soft)
```

A price must pass three gates:

1. **Stale** (`PriceFeedStale`): a leg is older than its feed's
   `max_stale_seconds` or the asset's `max_price_stale_seconds`, or two market
   legs differ in age by more than `MAX_LEG_AGE_SPREAD_SECONDS`.
2. **Disagree** (`UnsafePriceNotAllowed`): two legs are outside the tolerance
   band, or one of two legs has no reading.
3. **Sanity**: the final price is not positive (`InvalidPrice`) or is outside
   `[min_sanity_price_wad, max_sanity_price_wad]` (`SanityBoundViolated`).

Chainlink sources store the network proxy address plus a 32-byte `data_id` and
read `latest_round(data_id, 18)` once per source. They are unsmoothed market
legs; missing, frozen, malformed, non-positive, oversized, or future-dated
rounds fail closed. The deployed Stellar proxy has no bulk-read entrypoint, so
session warming remains limited to RedStone-format providers.

`ProviderRef` is shared by the governance and price-aggregator ABIs. Run
`make <network> upgradeAll` before scheduling a Chainlink
`ConfigureAssetOracle`; the operator script checks both deployed ABIs and the
configured live round before proposing. Editing JSON alone does not change
on-chain state.

Mainnet USDT0 gives the Chainlink leg 93,600 seconds (26 hours) for its
published 24-hour heartbeat. The same source, asset, deviation, midpoint, and
sanity gates used by every other provider still apply.

## Curve LP collateral

`CurveStableLp` values plain Curve StableSwap LP shares with 2–3 coins:

```text
price_wad = floor(get_virtual_price() * min(all coin prices_wad) / 1e18)
pool_value_wad = floor(price_wad * total_supply() / 1e18)
```

Curve supplies the invariant/share calculation, including amplification ramps
and active reserve accounting. The aggregator resolves every constituent's
`PriceKey::Token` oracle and uses the oldest timestamp. It enforces the
existing freshness, depth, cycle and LP sanity-band rules. The pool value must
meet `min_pool_value_wad`.

This is conservative gross value, not a guarantee of one-coin redemption
proceeds. Collateral risk parameters must allow for fees and liquidation
liquidity.

### Admission policy

- Only plain pools qualify. Rate-bearing/rebasing pools and metapools are
  outside this policy; metadata alone cannot establish their safety.
- Each read rechecks the executable hash, ordered coins, live decimals and
  fixed rates `10^(36 - coin_decimals)`. An executable change makes the source
  unpriceable until it is reviewed and reconfigured.
- LP sources (Aquarius or Curve) are collateral-only. Curve's lowest-coin price
  understates an LP debt.
- `MAX_CURVE_COINS` is 3. Larger pools need their own code and
  transaction-resource validation.
- The mainnet reviewed list is empty: no Curve pool executable has been
  reviewed for mainnet. The testnet list holds the two fixture executables,
  whose upstream source is unknown (see the
  [fixture README](../swap-aggregator/tests/fixtures/curve/README.md)).

Trust assumptions:

- LP value depends on the amplification coefficient A whenever the pool is
  imbalanced, and the pool factory's admin can ramp A.
- The source pins the executable, not the factory. Governance must treat the
  factory admin as trusted, as it does an Aquarius stable pool's admin.
- On an imbalanced pool a ramp can move the mark either way. It never moves it
  above the normalized reserves per share times the lowest coin price.
- Each coin's issuer is trusted as for a directly listed token (clawback,
  freeze, upgrade, decimals change).

### Listing a Curve pool

1. Review the pinned executable and underlying tokens for plain pool semantics.
2. Record the reviewed executable in `configs/<network>/curve_reviewed_pools.json`.
   See [Reviewed-pool record](#reviewed-pool-record).
3. Configure every coin oracle.
4. Set the LP's `min_sanity_price_wad` at or below the smallest coin
   `min_sanity_price_wad` times the lowest expected virtual price / WAD. The LP
   band then never trips before the coin bands.
5. Configure the LP token with `asset_decimals: 18` and exactly one
   `CurveStableLp` source.
6. Set the source fields: `pool`, `coins` (exact pool order), `wasm_hash` (64
   hexadecimal characters in CLI JSON) and `min_pool_value_wad`. The pool
   address is also the LP token address.
7. Create the 18-decimal lending market with `can_be_borrowed: false`.
8. Enable collateral in the desired spoke.
9. If the pool sits near the [resource limits](#curve-resource-bench), measure
   the production RedStone adapter, Chainlink proxy and listed token
   executables first.

The operator scripts enforce two of these rules before proposing:

- `validateConfigs` and the direct `configureMarketOracle` proposal reject a
  `CurveStableLp` source whose `wasm_hash` is not listed for that network.
- `validateConfigs` rejects a spoke listing that makes an LP market borrowable.
  `addAssetToSpoke` / `editAssetInSpoke` refuse one before proposing.

Prices are keyed by asset address. The borrowable check therefore covers every
market (any hub or name) sharing the LP's address. It also covers a market
whose `Scaled` quote (a Token or a reference) is LP-priced.

#### Reviewed-pool record

The file holds
`{"pools": [{wasm_hash, source_repo, source_commit, review, expected_factory}]}`.

- `wasm_hash` is a lowercase 64-hex hash.
- Every field is non-empty. Write `unknown` rather than guess.
- On mainnet, `unknown` is refused.
- On mainnet, `source_commit` must be a 40-hex git commit.
- On mainnet, `source_repo` must name the reviewed source. A value that is `?`
  or contains `TODO`, `TBD` or `n/a`, in any case, is refused.
- On mainnet, `expected_factory` must be the factory's contract address
  (`^C[A-Z2-7]{55}$`).

### Curve resource bench

Curve tests use the real pool WASMs; the generic production-fixture runner
rejects Curve sources. Real-WASM fixtures cover 2 and 3 coins.

- **Native case.** The harness liquidates `POSITION_LIMIT_MAX` 3-coin LP legs
  within the default Soroban budget, about 55M CPU (see the
  [threat model](../../docs/explanation/threat-model.md#numeric-and-resource-limits)).
  The controller and feed run natively, with SAC coins, single-source coin
  oracles and one debt leg.
- **Worst case.** `curve_lp_liquidation_worst_case_all_wasm` runs every
  contract on the path as WASM, with custom-WASM coins, two-source coin oracles
  and 5 debt legs. It uses `mock_redstone.wasm` for both feed providers and
  `production_fixture.wasm` for every coin and debt asset.

The bench asserts all eight resource ceilings in `MAINNET_LIMITS`, which mirror
the mainnet transaction limits:

| Limit | Mainnet ceiling | Worst case measured |
| --- | --- | --- |
| `tx_max_instructions` | 400,000,000 | 107.7M |
| `tx_memory_limit` | 41,943,040 | 15.1M |
| `tx_max_footprint_entries` | 400 | — |
| `tx_max_disk_read_entries` | 200 | — |
| `tx_max_write_ledger_entries` | 200 | — |
| `tx_max_disk_read_bytes` | 200,000 | — |
| `tx_max_write_bytes` | 132,096 | — |
| `tx_max_contract_events_size_bytes` | 16,384 | — |

The production RedStone adapter, Chainlink proxy and listed token executables
can cost more than the fixtures.

## Entrypoints

`prices` and `quotes` are bulk only. Pass a `Vec<PriceKey>` even for one key.

| Entrypoint | Caller | Does |
| --- | --- | --- |
| `prices(keys) -> Map<PriceKey, PriceFeedRaw>` | anyone | Fail-closed read; panics when a key fails a gate |
| `quotes(keys) -> Map<PriceKey, PriceStatus>` | anyone | Soft read; a failing key returns `valid: false` with its `error_code` |
| `price_spread(key) -> (low, high)` | anyone | The two leg prices (WAD). Fail-closed |
| `oracle(key) -> Option<AssetOracle>` | anyone | The registered configuration |
| `get_owner() -> Option<Address>` | anyone | The owner |
| `set_oracle(key, oracle)` | owner | Registers a configuration after validation and attestation. A replacement must keep the stored `asset_decimals` (`InvalidOracleDecimals`) |
| `set_sanity_band(key, min_wad, max_wad)` | owner | Narrows the accepted range; a wider band reverts `SanityBandMustTighten`. Probes for configuration errors only. A band under which the live price fails is committed |
| `set_tolerance(key, tolerance)` | owner | Sets the dual-source tolerance. Probes for configuration errors only. A tolerance under which the live price fails is committed |
| `upgrade(new_wasm_hash)` | owner | Renews the instance TTL, then replaces the Wasm |

Governance calls `set_sanity_band` on its immediate path. To widen a band, use
the timelocked `ConfigureAssetOracle` operation, which calls `set_oracle`.

`seed_oracle` and `remove_oracle` write the registry with no auth, validation
or attestation. They exist only under `cfg(test)` or the `testing` feature,
never in a release build.

## Storage and TTL

| Key | Tier | Holds |
| --- | --- | --- |
| `AggregatorKey::Oracle(PriceKey)` | persistent | One `AssetOracle` configuration |
| `AggregatorKey::OracleKeys` | instance | Index of every registered `PriceKey` |
| OpenZeppelin `Ownable` owner | instance | Governance |

Every entrypoint, views included, renews the instance TTL with
`renew_instance`. Every read and write of an `Oracle` entry extends it with
`TTL_THRESHOLD_SHARED` and `TTL_BUMP_SHARED`.

## Events

`set_oracle`, `set_sanity_band` and `set_tolerance` publish
`UpdateAssetOracleEvent` (`["config", "asset_oracle"]`) with the committed
configuration. See [events](../../docs/reference/events.md#price-aggregator-events).

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

## References

- Errors: [`docs/reference/errors.md`](../../docs/reference/errors.md)
- Events: [`docs/reference/events.md`](../../docs/reference/events.md#price-aggregator-events)
- Formal proofs: [`certora/README.md`](../../certora/README.md) (price system)
