# 0010. Flash loans settle by measured allowance pull

**Status:** Accepted

## Decision

A flash-loan receiver receives funds, runs its callback, and repays through a
pre-authorized allowance. The pool verifies the expected token balance before
and after the callback, including the fee.

The receiver must be deployed code. During the callback, controller paths that
could observe or change monetary state are blocked by a shared reentrancy
guard.

## Guarantees

- Direct token pushes cannot impersonate valid repayment.
- Underpayment, overpayment assumptions, and callback failure revert atomically.
- Nested access to protected monetary paths is rejected.

## Auditor focus

Test balance snapshots, allowance scope, fee rounding, callback rollback, and
all cross-entry reentrancy paths.
