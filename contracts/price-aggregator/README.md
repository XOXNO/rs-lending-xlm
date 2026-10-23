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

`CurveStableLp` values plain Curve StableSwap LP shares with 2–8 coins:

```text
price_wad = floor(get_virtual_price() * min(all coin prices_wad) / 1e18)
pool_value_wad = floor(price_wad * total_supply() / 1e18)
```

Curve supplies the invariant/share calculation, including amplification ramps
and active reserve accounting. The aggregator resolves every constituent's
`PriceKey::Token` oracle, uses the oldest timestamp, and enforces the existing
freshness, depth, cycle and LP sanity-band rules. The pool value must meet
`min_pool_value_wad`. This is conservative gross value, not a guarantee of
one-coin redemption proceeds; collateral risk parameters must allow for fees
and liquidation liquidity.

To list: upgrade governance and the price aggregator for the new ABI, configure
all coin oracles first, then configure the LP token with `asset_decimals: 18`
and exactly one `CurveStableLp` source. Its fields are `pool`, `coins` (exact
pool order), `wasm_hash` (64 hexadecimal characters in CLI JSON), and
`min_pool_value_wad`. The pool address is also the LP token address. Create the
18-decimal lending market and enable collateral in the desired spoke normally.

Governance must review the pinned executable and underlying tokens for plain
pool semantics. Each read rechecks its hash, ordered coins, live decimals and
fixed rates `10^(36 - coin_decimals)`. Rate-bearing/rebasing pools and metapools
are outside this admission policy; metadata alone cannot establish their safety.
An executable change makes the source unpriceable until reviewed and reconfigured.
Actual WASM fixtures cover 2/3 coins; 8 is a structural resource ceiling, and
larger deployments require their own code and transaction-resource validation.
The generic production-fixture runner rejects Curve sources; Curve tests use
the real pool WASMs.

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
providers/      # aquarius + curve (LP) + chainlink + multi_feed (bulk) + reflector
```

## References

- Errors: [`docs/reference/errors.md`](../../docs/reference/errors.md)
- Events: [`docs/reference/events.md`](../../docs/reference/events.md)
- Formal proofs: [`certora/README.md`](../../certora/README.md) (price system)
