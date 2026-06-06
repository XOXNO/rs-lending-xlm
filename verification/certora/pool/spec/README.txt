Pool accounting — Certora proof domain
======================================

Core invariant
--------------
Pool state stays internally consistent: scaled positions reconstruct to actual
balances via indexes, reserves bound outgoing liquidity, revenue_ray never
exceeds supplied_ray, and cross-operation sequences preserve additivity (no
free value from supply/withdraw or borrow/repay round-trips).

Assumptions
-----------
- Pool is invoked through the production LiquidityPool ABI.
- Controller summaries are not used in this WASM; proofs target pool logic
  directly.
- summary-contract rules prove the pool side of controller summary contracts.

Conf → spec map
---------------
integrity.conf
  spec/integrity_rules.rs
  Rules: constructor, domain invariant, per-operation state preservation,
  bad-debt index floor, pool_integrity_reachability

additivity.conf / additivity-heavy.conf
  spec/additivity_rules.rs
  Rules: no-profit roundtrips, pool_additivity_reachability

summary-contract.conf / summary-contract-critical.conf
  spec/summary_contract_rules.rs
  Rules: each pool entry point satisfies the controller summary contract
  (lemma layer — run before controller solvency proofs that rely on summaries)

Lemma-before-main
-----------------
1. integrity.conf
2. additivity.conf
3. summary-contract.conf
4. summary-contract-critical.conf (audit / heavy)

Controller proofs that summarize pool calls are only accounting evidence after
summary-contract.conf passes.