Controller safety — Certora proof domain
========================================

Core invariant
--------------
Accounts remain solvent under configured risk parameters: health factor gates
borrow and liquidation, oracle prices respect staleness/tolerance policy, and
controller-pool interactions preserve scaled-amount conservation and reserve
availability.

Assumptions
-----------
- Pool responses may be summarized via shared/summaries/ for tractability.
- Pool summary soundness is proved in pool/spec/summary_contract_rules.rs first.
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
  liquidation.conf, liquidation-light.conf, liquidation-integrity-heavy.conf
  spec/liquidation_rules.rs

Health / positions
  health.conf, positions.conf
  spec/health_rules.rs, position_rules.rs

Oracle
  oracle.conf, tolerance-math.conf
  spec/oracle_rules.rs, tolerance_math_rules.rs

Rates / indexes / interest / math
  indexes.conf, interest.conf, math.conf, boundary-math.conf,
  boundary-rates.conf, boundary-oracle.conf
  spec/index_rules.rs, interest_rules.rs, math_rules.rs, boundary_rules.rs

Strategy / flash loan / e-mode / guards
  strategy.conf, flash_loan.conf, emode.conf, market-guard.conf
  spec/strategy_rules.rs, flash_loan_rules.rs, emode_rules.rs,
  market_guard_rules.rs

Cross-contract consistency
  controller-pool-consistency.conf, controller-pool-consistency-light.conf
  spec/consistency_rules.rs

Lemma-before-main ordering
--------------------------
1. pool/confs/summary-contract.conf
2. controller/confs/tolerance-math.conf + oracle-compose.conf
3. controller/confs/solvency-*.conf + liquidation.conf
4. *-heavy.conf audit configs

See architecture/INVARIANTS.md sections 2–5 for runtime cross-references.