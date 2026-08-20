# 0016. Interest accrues in bounded time chunks

**Status:** Accepted

**Implemented by:** common/src/rates/compound.rs (`MAX_COMPOUND_DELTA_MS`), common/src/rates/index.rs, common/src/rates/simulate.rs, common/src/rates/curve.rs, common/src/constants/shared.rs (`MILLISECONDS_PER_YEAR`), common/src/constants/pool.rs (`SUPPLY_INDEX_FLOOR_RAW`, `MAX_BORROW_RATE_RAY`), contracts/pool/src/time.rs (`now_ms`), contracts/pool/src/interest.rs (`add_protocol_revenue`).

## Decision

Rates are represented in RAY per millisecond. When time advances, accrual
processes bounded chunks of a truncated exponential approximation. Each chunk
uses the market state produced by the preceding chunk.

The design keeps arithmetic bounded across long inactivity periods without
pretending that a single unbounded calculation is safe.

## Guarantees

- Time cannot run backward and zero elapsed time is a no-op.
- Borrow and supply indexes remain inside their configured domains.
- Long gaps produce deterministic, bounded work and conservative rounding.

## Auditor focus

Check unit conversion, chunk boundaries, rate-model replacement after time has
passed, overflow caps, and the split of accrued value into suppliers and revenue.
