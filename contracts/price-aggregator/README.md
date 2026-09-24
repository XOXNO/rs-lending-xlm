# Price Aggregator

Single oracle entry for the lending protocol. All pricing uses `PriceKey`.

## Pipeline

```text
prices(keys) / quotes(keys)
  → Session::new · warm(keys)
  → for each key: resolve → Outcome
  → force (hard) | to_status (soft)
```

## Three gates

1. **Stale** (`PriceFeedStale`) — a leg is older than its feed's
   `max_stale_seconds` or the asset's `max_price_stale_seconds`, or two market
   legs differ in age by more than `MAX_LEG_AGE_SPREAD_SECONDS`
2. **Disagree** (`UnsafePriceNotAllowed`) — two legs fall outside the tolerance
   band, or one of two legs has no reading
3. **Sanity** — the final WAD USD price is not positive (`InvalidPrice`) or is
   outside `[min_sanity_price_wad, max_sanity_price_wad]` (`SanityBoundViolated`)

## Surface

`prices` and `quotes` are bulk: there is no single-key `price` or `quote` call.
Pass a `Vec<PriceKey>` even for one key.

| Entrypoint | Signature | Who may call | What it does |
| --- | --- | --- | --- |
| `get_owner` | `get_owner(env: Env) -> Option<Address>` | anyone | Returns the configured owner, or `None` when unset. |
| `prices` | `prices(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceFeedRaw>` | anyone | Fail-closed read. Panics when a key fails any gate. |
| `quotes` | `quotes(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceStatus>` | anyone | Soft read. Does not panic on a failing key; returns `PriceStatus { valid: false }` with its `error_code`. |
| `price_spread` | `price_spread(env: Env, key: PriceKey) -> (i128, i128)` | anyone | Returns the two leg prices (WAD) as `(low, high)`. Fail-closed. |
| `oracle` | `oracle(env: Env, key: PriceKey) -> Option<AssetOracle>` | anyone | Reads the registered configuration for one key. |
| `set_oracle` | `set_oracle(env: Env, key: PriceKey, oracle: AssetOracle)` | owner | Registers a configuration after validation and attestation. |
| `set_sanity_band` | `set_sanity_band(env: Env, key: PriceKey, min_wad: i128, max_wad: i128)` | owner | Narrows the accepted WAD USD range. Live-probes before committing. |
| `set_tolerance` | `set_tolerance(env: Env, key: PriceKey, tolerance: OracleTolerance)` | owner | Sets the dual-source disagreement tolerance. Live-probes before committing. |
| `upgrade` | `upgrade(env: Env, new_wasm_hash: BytesN<32>)` | owner | Renews the instance TTL, then replaces the contract Wasm. |

`set_sanity_band` only narrows the band: `min_wad` must be >= the current min
and `max_wad` <= the current max, else it panics with `SanityBandMustTighten`.
Governance calls it on the immediate (no-timelock) path. Widening goes through
the timelocked `ConfigureAssetOracle` operation, which calls `set_oracle`.

Two further entrypoints, `seed_oracle` and `remove_oracle`, are compiled only
under `cfg(test)` or the `testing` feature. They write the registry directly,
skipping owner authorization, validation and attestation, so they are absent
from a release build.

The controller lifts `Address` to `PriceKey::Token` before calling.

## Layout

```text
lib.rs          # contract entrypoints + session orchestration
session.rs      # clock, stack, multi-feed warm, memos
engine.rs       # resolve → Outcome → force | to_status
admin.rs        # set_oracle / set_sanity_band / set_tolerance, attest, cascade
registry.rs     # persistent oracle storage, key index, config event
validation.rs   # write-time config checks (sources, depth, staleness, decimals)
tolerance.rs    # dual-source disagreement bounds
observation.rs  # provider payload → normalized WAD observation
properties.rs   # write-time dependency walk
providers/      # aquarius (LP) + multi_feed (bulk) + reflector
interfaces/…    # client ABI mirror
```

## Owner

Governance deploys the contract and passes itself as the constructor's
`owner`. `#[only_owner]` gates every write, `upgrade` included. The ABI has no
ownership transfer, accept, or renounce entrypoint. The controller reads
`prices` for risk checks and `quotes` for views.
