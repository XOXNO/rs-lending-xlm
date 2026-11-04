# Parameter validators: exact acceptance domains, proved on the server

Feature `validation` calls the production `common::validation` functions.
Each `*_accepts` claim proves the validator returns normally on the
specified domain; each `*_rejects` claim proves that outside it the call never
returns normally: every rejecting path ends in the validator's contract error
(the code is checked), and the harness's `false` after the call is never
reached. Inputs range over every `u32` / `i128` value.

| Validator | Accepted domain | Accept | Reject (error code) |
|---|---|---|---|
| `validate_risk_bounds(ltv, threshold, bonus)` | `ltv < threshold <= 10000` and `threshold * (10000 + bonus) <= 10000^2` | `PASSED` 15 nodes, 719 steps, 809 s | `InvalidLiqThreshold` (113) on every rejecting path; 11 nodes, 768 steps, 928 s |
| `validate_twap_records(records)` | `1 <= records <= 12` | `PASSED` 7 nodes, 260 steps, 388 s | 219 (`records == 0`), 228 (`records > 12`); 11 nodes, 319 steps, 459 s |
| `validate_liquidation_curve(target, hf, factor)` | `WAD < target <= 10 WAD`, `0 < hf < target`, `0 < factor <= 10000` | `PASSED` 47 nodes, 1,506 steps, 870 s | `InvalidLiquidationCurve` (134); 19 nodes, 1,595 steps, 527 s |

False controls `test_risk_bounds_reject_wrong` and
`test_twap_records_reject_wrong` (a valid input claimed rejected) both
`FAILED` at the completed `false` assertion, showing that a normal return is
detected. Records and graphs: [`server/`](server/summary.json); toolchain and
trusted rules: [`TRUSTED-BASE.md`](../../../pipeline/TRUSTED-BASE.md).
