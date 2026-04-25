# Certora Engagement Handoff

Audit-prep status of the Certora Soroban Prover surface in this repo. The
empirical end-to-end prover run is the one remaining gate before the
formal-verification track of the audit can complete; this document records
everything that ships locally and tells the engagement team how to dispatch
the cloud run.

For the rule-quality classification (tautological / weak / vacuous) and
remediation plan, see [`architecture/MATH_REVIEW.md §3`](../../architecture/MATH_REVIEW.md).

## Local validation status

Run from `controller/`:

```bash
cargo check -p controller --features certora --no-default-features
```

**Result on the audit-frozen commit**: clean (no errors, no warnings).
Vendored CVLR under `vendor/cvlr/` with `#![no_std]` patched onto
`cvlr-spec/src/lib.rs` resolves the prior `error[E0463]: can't find crate
for core` blocker (documented in the deleted `SPIKES.md` from commit
`7a24930`). Workspace `Cargo.toml` redirects every `cvlr-*` crate to the
vendored copy.

Orphan check:

```bash
python3 controller/certora/check_orphans.py
```

**Result**: `OK: 13 confs, 190 source rules, zero orphans`. Every rule
referenced in a `.conf` exists in the spec; every spec rule is referenced
by at least one conf.

## Spec inventory

13 conf files under `controller/confs/`, 190 unique source rules across 16
spec modules under `controller/certora/spec/`:

| Conf | Spec module | Rule fns |
|---|---|---|
| `boundary.conf` | `boundary_rules.rs` | 38 |
| `solvency.conf` | `solvency_rules.rs` | 34 |
| `strategy.conf` | `strategy_rules.rs` | 20 |
| `math.conf` | `math_rules.rs` | 19 |
| `interest.conf` | `interest_rules.rs` | 15 |
| `emode.conf` | `emode_rules.rs` | 15 |
| `liquidation.conf` | `liquidation_rules.rs` | 10 |
| `isolation.conf` | `isolation_rules.rs` | 9 |
| `oracle.conf` | `oracle_rules.rs` | 8 |
| `positions.conf` | `position_rules.rs` | 6 |
| `health.conf` | `health_rules.rs` | 6 |
| `indexes.conf` | `index_rules.rs` | 6 |
| `flash_loan.conf` | `flash_loan_rules.rs` | 4 |

Every conf carries `cargo_features = ["certora"]` and `build_script =
"../certora_build.py"`.

## Empirical prover run (engagement-team)

The Certora Soroban Prover binary is **not** installed at the repo path
(`.certora-venv/bin/`) in this environment. The engagement team uses their
own Certora account + key; this is a paid SaaS that runs in Certora's
cloud.

```bash
# One-time setup on the engagement machine:
pip install certora-cli
export CERTORAKEY=<engagement-team-key>

# Dispatch every conf:
for c in controller/confs/*.conf; do
    certoraSorobanProver "$c"
done
```

Each invocation uploads the build artefacts and the spec to Certora's
cloud, returns a job URL, and (asynchronously) produces a verdict per
rule.

Recommended order (math + solvency first; oracle / liquidation second;
boundary last because it has the most rules):

1. `math.conf` — pure half-up rounding (cheapest, fastest signal).
2. `indexes.conf` — index monotonicity.
3. `interest.conf` — rate model + supplier-rewards conservation.
4. `solvency.conf` — accounting conservation.
5. `health.conf` — HF math.
6. `positions.conf` — scaled balance.
7. `isolation.conf` + `emode.conf` — mode invariants.
8. `oracle.conf` — tolerance + staleness.
9. `liquidation.conf` — bonus / seizure / bad-debt.
10. `flash_loan.conf` — re-entry + repayment.
11. `strategy.conf` — multiply / swap_*.
12. `boundary.conf` — boundary-condition coverage.

## Known rule-quality gaps (do NOT treat as blockers)

`architecture/MATH_REVIEW.md §3` enumerates rule-quality classifications.
Summary:

| Class | Count | Action |
|---|---|---|
| Strong rules | 102 | Run as-is. |
| Weak rules | 9 | Tighten bounds (post-engagement remediation). |
| Tautological rules | 16 | Rewrite to call prod (post-engagement). |
| Vacuous rules | 4 | Repair preconditions (post-engagement). |
| Sanity satisfies | 29 | Keep — they are reachability checks, not assertions. |

The engagement-team verdicts on the strong rules are the priority signal.
A `pass` on a tautological rule is uninformative; a `fail` on a strong
rule is a finding.

## Pending toolchain items (`MATH_REVIEW.md §0`)

| Item | Status | Action |
|---|---|---|
| `cvlr-spec` compile blocker | Done | Vendored in `vendor/cvlr/`, workspace `Cargo.toml` redirects |
| `cargo check --features certora`: clean | Done | Audit-frozen commit `audit-2026-q2` |
| 7 backfilled solvency rules registered in `confs/solvency.conf` | Done | `controller/confs/solvency.conf` |
| Spec compiles after `MAX_BORROW_RATE_RAY` cap | Done | This audit-prep cycle |
| Empirical `certoraSorobanProver <conf>` run | Pending | Engagement-team handoff |
| Delete or repurpose `summaries/mod.rs` | Pending | Post-engagement remediation |
| Add `apply_summary!` wrappers at pool / oracle / SAC call sites | Pending | Post-engagement remediation |
| Delete dead `model.rs` ghost vars | Pending | Post-engagement remediation |
| Rewrite 13 tautological rules to call prod | Pending | Post-engagement remediation |

## What lives where

```
controller/
├── certora/
│   ├── HANDOFF.md            # this file
│   ├── check_orphans.py      # validates conf <-> spec coverage
│   ├── spec/
│   │   ├── mod.rs            # registers every spec module
│   │   ├── boundary_rules.rs
│   │   ├── compat.rs         # storage-shim adapters for spec context
│   │   ├── emode_rules.rs
│   │   ├── flash_loan_rules.rs
│   │   ├── health_rules.rs
│   │   ├── index_rules.rs
│   │   ├── interest_rules.rs
│   │   ├── isolation_rules.rs
│   │   ├── liquidation_rules.rs
│   │   ├── math_rules.rs
│   │   ├── model.rs          # ghost variables (currently unused)
│   │   ├── oracle_rules.rs
│   │   ├── position_rules.rs
│   │   ├── solvency_rules.rs
│   │   ├── strategy_rules.rs
│   │   └── summaries/
│   │       └── mod.rs        # empty placeholder (post-engagement repurpose)
│   └── results/
│       └── submission-*.md   # historical submission batches
├── confs/                    # 13 .conf files, one per spec module
└── certora_build.py          # build_script referenced by every .conf
```

## Sanity replay on a fresh clone

```bash
git clone <repo>
cd rs-lending-xlm
git checkout audit-2026-q2
cargo check -p controller --features certora --no-default-features
python3 controller/certora/check_orphans.py
```

Expected: `cargo check` exits 0; `check_orphans.py` prints
`OK: 13 confs, 190 source rules, zero orphans`.
