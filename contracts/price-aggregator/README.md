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

1. **Stale** — per-feed window + asset ceiling  
2. **Disagree** — dual sources outside tolerance  
3. **Sanity** — final USD not in `[min, max]`

## Surface

All reads are bulk: there is no single-key `price` or `quote` call. Pass a
`Vec<PriceKey>` even for one key.

| Entrypoint | Signature | Who may call | What it does |
| --- | --- | --- | --- |
| `get_owner` | `get_owner(env: Env) -> Option<Address>` | anyone | Returns the configured owner, or `None` when unset. |
| `prices` | `prices(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceFeedRaw>` | anyone | Fail-closed read. Panics when a key fails any gate. |
| `quotes` | `quotes(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceStatus>` | anyone | Soft read. Never panics; returns `PriceStatus { valid: false }` for a failing key. |
| `price_spread` | `price_spread(env: Env, key: PriceKey) -> (i128, i128)` | anyone | Returns `(low, high)` after the gates. Fail-closed. |
| `oracle` | `oracle(env: Env, key: PriceKey) -> Option<AssetOracle>` | anyone | Reads the registered configuration for one key. |
| `set_oracle` | `set_oracle(env: Env, key: PriceKey, oracle: AssetOracle)` | owner | Registers a configuration after validation and attestation. |
| `set_sanity_band` | `set_sanity_band(env: Env, key: PriceKey, min_wad: i128, max_wad: i128)` | owner | Sets the accepted USD range. Live-probes before committing. |
| `set_tolerance` | `set_tolerance(env: Env, key: PriceKey, tolerance: OracleTolerance)` | owner | Sets the dual-source disagreement tolerance. Live-probes before committing. |

`set_sanity_band` applies a one-way ratchet on the immediate (no-timelock)
path: the owner may only narrow the band (`min_wad` >= the current min and
`max_wad` <= the current max), never widen it — a widening call panics with
`SanityBandMustTighten`. Widening must go through the timelocked
`ConfigureAssetOracle` path.

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

Governance at construct only. `#[only_owner]` gates writes; no transfer,
accept, or renounce on the ABI. Consumers: controller (risk), views (quotes).
