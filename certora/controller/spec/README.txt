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
- Pool calls are trusted summaries. pool/spec/summary_contract_rules.rs checks
  selected seeded output shapes, not universal refinement; controller verdicts
  that use those summaries are conditional evidence.
- Oracle price resolution may be harness-summarised; the strict fail-closed
  band behaviour is proved in oracle_rules, ratio math in tolerance_math_rules
  (lemma layer).

Conf → spec map (by theme)
--------------------------
Solvency / caps (split from monolithic solvency.conf)
  solvency-reserves.conf — supply caps, revenue, utilization
  solvency-borrow.conf — borrow limits, LTV
  solvency-index.conf — index monotonicity, compound, scaled reconstruction
  solvency-roundtrip.conf — roundtrips, cache, sanity rules
  global-solvency-heavy.conf, no-collateral-no-debt.conf
  spec/solvency_rules.rs

Liquidation
  liquidation.conf, liquidation-accounting-math.conf, liquidation-bonus.conf,
  liquidation-estimation.conf, liquidation-integrity-heavy.conf
  spec/liquidation_rules.rs

Health / positions
  health.conf, health-gated.conf, positions.conf, hf-lemmas.conf
  spec/health_rules.rs, position_rules.rs, hf_lemma_rules.rs
  (health_ghost.rs — ghost-state support module, no rules)

Oracle
  Owned by price-aggregator/confs and price-aggregator/spec. Controller rules
  consume the shared fail-closed price-feed summary.

Rates / indexes / interest / math
  indexes.conf, interest.conf, math.conf, math-bv.conf (bit-precise
  escalation), boundary-math.conf, boundary-rates.conf, boundary-oracle.conf
  spec/index_rules.rs, interest_rules.rs, math_rules.rs, boundary_rules.rs

Strategy / flash loan / spoke / guards
  strategy.conf, strategy-repay-collateral.conf (heavy: full
  withdraw+swap+repay path, one rule per invocation), flash_loan.conf,
  spoke.conf, market-guard.conf
  spec/strategy_rules.rs, flash_loan_rules.rs, spoke_rules.rs,
  market_guard_rules.rs

Cross-contract consistency
  controller-pool-consistency.conf, controller-pool-consistency-light.conf
  spec/consistency_rules.rs

Account isolation (frame rules)
  account-isolation.conf
  spec/account_isolation_rules.rs

Support modules (no rules)
  spec/compat.rs — single-asset ABI shims for multi-asset entrypoints
  spec/health_ghost.rs — ghost state for health rules
  spec/mod.rs — module mount; harness/ — storage/oracle/pool summaries

Proof ordering
--------------
1. pool/confs/summary-contract.conf (seeded summary-shape smoke only)
2. price-aggregator/confs/tolerance-math.conf
3. controller/confs/solvency-*.conf + liquidation.conf
4. *-heavy.conf audit configs

See architecture/INVARIANTS.md sections 2–5 for runtime cross-references.
