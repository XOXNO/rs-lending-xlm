# Price Aggregator

Single oracle entry for the lending protocol. All pricing uses `PriceKey`.

## Pipeline

```text
prices(keys) / quotes(keys)
  → Session::new · warm(keys)   // multi-feed bulk by adapter (≥2 feeds)
  → for each key: resolve → Outcome   // one evaluator; providers always soft
  → force (hard) | to_status (soft)   // only edge that diverges
```

## Three gates

1. **Stale** — per-feed window + asset ceiling  
2. **Disagree** — dual sources outside tolerance  
3. **Sanity** — final USD not in `[min, max]`

## Surface

| Call | Role |
| --- | --- |
| `price` / `prices` | Fail-closed (`PriceKey` / `Vec<PriceKey>`) |
| `quote` / `quotes` | Soft diagnostics |
| `oracle` / `set_oracle` | Config read / owner write |
| `set_sanity_band` / `set_tolerance` | Live band edits (both live-probe before commit) |
| `price_spread` | `(low, high)` after gates |

Controller lifts `Address → PriceKey::Token` before calling.

## Layout

```text
lib.rs          # contract entrypoints + session orchestration
session.rs      # clock, stack, multi-feed warm, memos
engine.rs       # resolve → Outcome → force | to_status
admin.rs        # storage, attest, validate, set_*, events
properties.rs   # write-time dependency walk
providers/      # multi_feed (bulk) + reflector
interfaces/…    # client ABI mirror
```

## Owner

Governance at construct only. `#[only_owner]` gates writes; no transfer,
accept, or renounce on the ABI. Consumers: controller (risk), views (quotes).
