# Formal verification

This directory contains Certora Sunbeam specifications for the protocol's
highest-risk arithmetic, accounting, solvency, liquidation, oracle, and
strategy properties.

Formal verification complements tests and review. A successful compilation,
submission, or older report is not a proof verdict for the current artifact.
Read the report associated with the exact built WASM and source fingerprint.

## Start here

| Goal | Command |
|---|---|
| Check feature paths, configuration, and rule coverage | ./certora/compile_all.sh |
| Also build and verify Certora WASM provenance | ./certora/compile_all.sh --wasm |
| Build focused prover artifacts | make certora-wasm |
| List available proof profiles | make certora-list |
| Submit the default hosted profile | make certora |
| Submit a chosen profile | make certora CERTORA_PROFILE=fast |

The hosted prover requires CERTORAKEY and the Certora command-line tool. Build
the focused WASM before submitting directly or through a profile.

## What is covered

| Area | Main question |
|---|---|
| Common | Does fixed-point arithmetic preserve its stated bounds and rounding rules? |
| Pool | Do shares, indexes, cash, revenue, settlement, fees, and flash accounting remain coherent? |
| Controller | Do account actions preserve authorization and solvency requirements? |
| Price system | Do source admission, freshness, tolerance, and fail-closed outcomes hold? |
| Shared summaries | Are cross-contract assumptions explicit and reviewable? |

Each proof area has configuration files, rule modules, fixtures, and
domain-specific documentation. The [pool-core guide](pool/spec/README.md)
explains the pool suite in review terms.

## Artifact integrity

Deployable WASM and prover WASM serve different purposes:

| Artifact | Purpose |
|---|---|
| Deploy artifact | Optimized bytecode for deployment and upgrade |
| Focused Certora artifact | Unoptimized bytecode containing one focused rule module |

Focused artifacts reduce prover transformation cost. They are not a separate
production behavior: production code has no focused-verification branch.

The artifact manifest binds each focused artifact to its source fingerprint and
feature set. Rebuild after changing a contract, rule, fixture, summary, or
relevant dependency. Do not submit a stale artifact.

    make certora-wasm
    python3 certora/scripts/check_wasm_artifacts.py

The focused build intentionally disables the Stellar optimizer. Optimized
bytecode can trigger internal prover transformation failures despite passing
ordinary WASM validation.

## Proof profiles

| Profile | Use |
|---|---|
| sanity | Reachability and non-vacuity checks |
| fast | Stable math, rate, integrity, and light controller properties |
| core | Main audit set: solvency, liquidation, strategies, pool accounting, and oracle rules |
| heavy | Expensive targeted proofs |
| manual | Core plus heavy |
| all | Sanity, fast, core, and heavy |

Start with sanity. Run fast or core for a relevant change. Use heavy only for
the targeted surface or an intentional full verification run.

The repository holds 343 rules across 103 conf files and 6 profiles. Reproduce
those numbers with:

    grep -rh '#\[rule\]' certora --include='*.rs' | wc -l
    find certora -name '*.conf' | wc -l

`certora/scripts/check_orphans.py` confirms that confs, rules and profiles stay
in sync. It reports every rule that no conf runs and every conf that names a
rule which no longer exists:

    python3 certora/scripts/check_orphans.py

Extra prover flags follow a double dash:

    ./certora/scripts/run_profile.py fast -- --dry-run

`run_profile.py` passes those flags to every conf in the profile and stops at
the first non-zero exit. To prove one rule, run its own conf directly rather
than a whole profile, because the rule exists in only some confs:

    certoraSorobanProver interest.conf --rule borrow_rate_capped
      # from certora/controller/confs/

## What runs on a pull request

`certora-local.yml` runs on pull requests that touch `certora/**` or
`common/src/**`. It invokes the local prover over a fixed set of confs with a
per-rule time cap, and treats proved violations and tooling errors as failures
while recording timeouts as warnings.

`certora-verification.yml` and `certora-fastRules.yml` submit hosted jobs and
run only on manual dispatch. No profile runs automatically on every pull
request.

## Local and hosted execution

The profile runner can submit hosted jobs or invoke a local prover installation.
For local execution, provide a compatible Java runtime, Certora CLI
dependencies, and a local prover binary. Build focused WASM first.

    ./certora/scripts/run_profile.py sanity --local

For expensive local rules, use the dedicated local runner. It isolates temporary
prover state and retains logs under the build directory.

    ./certora/scripts/run-rules-local.sh certora/pool/confs/position-accounting.conf

The local runner defaults to conservative parallelism and an 8 GiB Java heap.
Increase CERTORA_JAVA_HEAP, enable split parallelism, or enable per-assert
diagnostics only after measuring available host and solver capacity.

## How to read a proof result

A verdict applies only within its model.

- Check the artifact fingerprint and configuration used by the report.
- Read assumptions, fixtures, summaries, loop bounds, and rule preconditions.
- Treat a reached cross-contract summary as conditional on that summary.
- Distinguish a counterexample, timeout, loop-unwind failure, and transformation
  error before changing a rule or production code.
- Keep universal assertions and satisfy witnesses separate; reachability is not
  a substitute for a universal property.

## Important proof boundaries

Pool rules directly exercise the accounting transitions used before token
transfers. They do not model arbitrary token behavior, flash callbacks,
allowances, reentrancy, or transaction rollback.

Controller rules use explicit summaries for cross-contract work where a full
composition would be intractable. A controller verdict therefore does not
independently prove the summarized pool, oracle, token, or external-call
behavior.

Price-system rules separate success-path properties from fail-closed outcomes.
Controller valuation rules assume an accepted price unless the rule explicitly
models price failure.

Long unbounded batch processing and arbitrary multi-year accrual loops remain
outside the current proof model when no suitable induction invariant exists.

## Adding or changing a proof

1. State the invariant and threat first.
2. Identify whether it belongs in common, pool, controller, or price-system
   verification.
3. Add a fixture that makes the relevant state reachable.
4. Add a focused rule and a configuration with appropriate sanity policy.
5. Build artifacts and run the static checks.
6. Run the smallest relevant profile or rule, then record the exact report
   and artifact identity.
7. Update the domain guide when the proven boundary or residual assumption
   changes.

Prefer a small lemma before a large stateful rule. Do not hide a timeout by
loosening a property or increasing resource limits without explaining the
change.

## Troubleshooting

| Symptom | First action |
|---|---|
| Artifact-provenance failure | Rebuild focused WASM and run the artifact checker |
| Optimizer-related transformation error | Rebuild with make certora-wasm; it uses unoptimized prover WASM |
| Rule is unreachable or vacuous | Add or repair a satisfy witness before trusting a universal result |
| Expanded-command limit | Review the modeled surface and raise the relevant limit only when justified |
| Local host runs out of memory | Run one rule, keep split parallelism off, and lower Java heap before increasing it |
| Counterexample appears bitwise-spurious | Re-run the targeted rule with precise bitwise modeling |

## References

- [Certora Sunbeam documentation](https://docs.certora.com/en/latest/docs/sunbeam/index.html)
- [Sunbeam tutorials](https://certora-sunbeam-tutorials.readthedocs-hosted.com/en/latest/)
- [Protocol invariants](../docs/reference/invariants.md)
- [Threat model](../docs/explanation/threat-model.md)
