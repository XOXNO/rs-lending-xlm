# Goal: Controller INV-ACCT-03 — measured inbound payments

**Status:** complete (first full pass)  
**Scope:** `contracts/controller/src/payments/` + every call site of `transfer_amount_measured` / measured deltas  
**Invariant:** [INV-ACCT-03](../../reference/invariants.md) — inbound value is credited by measured receipt only  
**Driver:** `.grok/workflows/controller-acct03-audit.rhai`  
**Run:** `controller-acct03-audit` (~5m 41s)  
**Report:** `docs/audit/controller-acct03/REPORT.md`  
**Confirmed findings:** 0  
**Related:** pool ops audit residual (pool trusts hub pre-fund); ADR `docs/explanation/decisions/0013-token-custody-split-measured-deltas.md`

## Objective

Confirm every path that pulls tokens into the controller (or credits pool-facing amounts from inbound transfers) uses measured balance deltas, not the requested amount.

## Call-site checklist

| Site | Path | Status |
|------|------|--------|
| Core helper | `payments/transfer.rs::transfer_amount_measured` | **clean** |
| Supply | `positions/supply.rs` | **clean** |
| Repay | `positions/repay.rs` | **clean** |
| Liquidation | `positions/liquidation/apply.rs` | **clean** |
| Keepers / recap | `keepers/mod.rs` | **clean** |
| Strategy legs | `strategies/legs.rs` | **clean** |
| Grep bypass | full controller inbound surface | **clean** |

## Result summary

- All pool-inbound paths book **measured** deltas (`to=pool`) into pool mutation args.
- Liquidation scales seizures when FoT under-delivers.
- Multiply / swap intermediate raw transfers remeasure on final deposit — FoT fails closed.
- Outbound claim/refunds intentionally unmeasured.
- Residual (not a finding): multiply `initial_payment` uses raw+requested intermediate only; final supply remeasures.

## Out of scope

- Pool-internal cash book (covered by pool-ops audit)
- Oracle / governance
- Outbound transfer measurement (unless it confuses inbound accounting)
