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
| `set_sanity_band(key, min_wad, max_wad)` | owner | Narrows the accepted range; a wider band reverts `SanityBandMustTighten`. Live-probes first |
| `set_tolerance(key, tolerance)` | owner | Sets the dual-source tolerance. Live-probes first |
| `upgrade(new_wasm_hash)` | owner | Renews the instance TTL, then replaces the Wasm |

Governance calls `set_sanity_band` on its immediate path. To widen a band, use
the timelocked `ConfigureAssetOracle` operation, which calls `set_oracle`.
Governance binds that operation to the band it reads from `oracle` at
proposal, so a narrowing in between makes the older reconfiguration revert
(`OracleBandChangedAfterProposal`).

`seed_oracle` and `remove_oracle` write the registry with no auth, validation
or attestation. They exist only under `cfg(test)` or the `testing` feature,
never in a release build.

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
providers/      # aquarius (LP) + multi_feed (bulk) + reflector
```

## References

- Errors: [`docs/reference/errors.md`](../../docs/reference/errors.md)
- Events: [`docs/reference/events.md`](../../docs/reference/events.md)
- Formal proofs: [`certora/README.md`](../../certora/README.md) (price system)
