Controller safety — Certora proof domain
========================================

Proof focus
-----------
Controller rules cover entrypoint gates, position-direction properties, and
selected accounting bounds. Health component comparisons reuse one frozen
valuation snapshot. Cross-contract claims remain conditional on the trusted
pool and price summaries described below.

Assumptions
-----------
- Pool responses may be summarized via shared/summaries/ for tractability.
  Those summaries are asserted assumptions, not proved refinements, so a
  controller verdict that reaches one is conditional evidence.
- Oracle price resolution may be harness-summarised; the strict fail-closed
  band behaviour is proved in price-aggregator/spec/oracle_rules.rs, ratio math
  in price-aggregator/spec/tolerance_math_rules.rs (lemma layer).
- Cross-contract `call` is not implemented in the prover and returns a havoced
  value, so a rule that bypasses a summary observes an unconstrained result.

Conf → spec map
---------------
account_isolation_rules.rs
  account-isolation.conf, account-isolation-sanity.conf

boundary_rules.rs
  boundary-math.conf, boundary-oracle.conf, boundary-rates.conf,
  boundary-math-sanity.conf, boundary-compound-sanity.conf,
  boundary-bad-debt-sanity.conf, supply-dust-sanity.conf

consistency_rules.rs
  controller-pool-consistency.conf

flash_loan_rules.rs
  flash_loan.conf, flash_loan-reverts.conf, flash_loan-reverts-sanity.conf,
  flash-loan-sanity.conf

health_rules.rs
  health.conf, health-gated.conf, health-post-gate.conf, health-recovery.conf,
  health-sanity.conf, health-gated-sanity.conf, health-post-gate-sanity.conf,
  health-strategy-sanity.conf
  (health_ghost.rs — ghost-state support module, no rules)

hf_lemma_rules.rs
  hf-lemmas.conf, hf-lemmas-sanity.conf

index_rules.rs
  indexes.conf, indexes-sanity.conf

interest_rules.rs
  interest.conf, interest-compound.conf, interest-index.conf

liquidation_rules.rs
  liquidation.conf, liquidation-accounting-math.conf,
  liquidation-additivity.conf, liquidation-bonus.conf,
  liquidation-estimation.conf, liquidation-sanity.conf

market_guard_rules.rs
  market-guard-reverts.conf, market-guard-reverts-sanity.conf,
  market-guard-sanity.conf

math_rules.rs
  math.conf, math-bv.conf (bit-precise escalation), math-reverts.conf,
  math-reverts-sanity.conf

position_rules.rs
  positions.conf, positions-sanity.conf

solvency_rules.rs
  solvency-borrow.conf, solvency-index.conf, solvency-roundtrip.conf,
  compound-output.conf, scaled-reconstruction.conf,
  solvency-borrow-reverts.conf, solvency-reserves-reverts.conf,
  repay-gates-reverts.conf, solvency-borrow-reverts-sanity.conf,
  solvency-reserves-reverts-sanity.conf, solvency-sanity.conf,
  roundtrip-sanity.conf, index-cache-sanity.conf

spoke_rules.rs
  spoke.conf, spoke-usage.conf, spoke-usage-liquidation.conf,
  bulk-borrow-duplicate-leg.conf, spoke-reverts.conf,
  spoke-usage-reverts.conf, spoke-sanity.conf, spoke-usage-sanity.conf,
  spoke-usage-liq-sanity.conf, spoke-reverts-sanity.conf,
  spoke-usage-reverts-sanity.conf

strategy_rules.rs
  strategy-bad-debt.conf, strategy-revenue.conf, strategy-flash-position.conf,
  strategy-repay-collateral.conf (heavy: full withdraw+swap+repay path, one
  rule per invocation), strategy-swap-collateral.conf, strategy-swap-debt.conf,
  strategy-reverts.conf, strategy-flash-position-reverts.conf,
  strategy-swap-collateral-reverts.conf, strategy-swap-debt-reverts.conf,
  strategy-multiply-sanity.conf, strategy-bad-debt-sanity.conf,
  strategy-revenue-sanity.conf, strategy-repay-collateral-sanity.conf,
  strategy-flash-position-sanity.conf, strategy-swap-collateral-sanity.conf,
  strategy-swap-debt-sanity.conf, strategy-flash-position-reverts-sanity.conf

Oracle
  Owned by price-aggregator/confs and price-aggregator/spec. Controller rules
  consume the shared fail-closed price-feed summary.

Support modules (no rules)
  spec/compat.rs — single-asset ABI shims for multi-asset entrypoints
  spec/health_ghost.rs — ghost state for health rules
  spec/fixture.rs — protocol, market and account seeding
  spec/mod.rs — module mount; harness/ — storage/oracle/pool summaries

Revert-shaped rules
-------------------
Every `call(...); cvlr_assert!(false);` rule lives in a `-reverts.conf` at
`rule_sanity: none`, because the TAC vacuity check reports SANITY_FAILED on
that shape by construction. Each one is paired with a satisfy witness that
completes the same fixture, either a `<rule>_fixture_completes` twin in the
sibling `-reverts-sanity.conf` or a documented existing witness. See
"Sanity checking on WASM" in certora/README.md.

Proof ordering
--------------
1. common/confs/math.conf and rates.conf (library lemmas)
2. price-aggregator/confs/tolerance-math.conf
3. controller/confs/solvency-*.conf + liquidation.conf
4. the heavy profile

Runtime cross-references: controller/pool source and harness tests.
