# 0009. An account chooses its risk regime once

**Status:** Accepted

## Decision

Each account binds to one spoke when it is created. A spoke defines the
asset-level risk rules used by that account. The binding does not change for
the account's lifetime.

This prevents an account from borrowing under one policy and later selecting a
more favorable policy for withdrawal, valuation, or liquidation.

## Guarantees

- Every position in an account is evaluated under one coherent risk regime.
- An account cannot migrate risk configuration by changing a call argument.
- Governance may evolve future listings without silently rewriting existing
  account identity.

## Auditor focus

Check creation defaults, all account-loading paths, position mutations, and
any migration or deletion operation for a way to rewrite the binding.
