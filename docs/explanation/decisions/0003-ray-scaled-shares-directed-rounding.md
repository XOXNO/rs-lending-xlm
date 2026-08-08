# 0003. Scaled shares and deliberate rounding

**Status:** Accepted

## Decision

Supply and debt positions are stored as RAY-scaled shares. Their asset value is
derived from a current interest index rather than rewritten on every accrual.

Conversions use a fixed rounding policy. Supplying mints fewer shares on dust,
withdrawing burns enough shares, borrowing mints enough debt, and repaying
burns only debt that the payment covers. A positive movement that would change
zero shares is rejected.

## Guarantees

- Interest accrues through indexes without iterating through accounts.
- Rounding cannot create a free supply, borrow, withdrawal, repayment, or
  settlement operation.
- The remaining dust bias is predictable and pool-conservative.

## Auditor focus

Check every share mint, burn, close, and index conversion together. A locally
reasonable rounding rule can be unsafe when paired with a different readout.
