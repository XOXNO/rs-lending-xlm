# 0013. Inbound credit uses measured receipt

**Status:** Accepted

**Implemented by:** common/src/token.rs (`transfer_amount_measured`), contracts/controller/src/payments.rs, contracts/controller/src/positions/liquidation/math.rs (`scale_seizures_to_received`), contracts/controller/src/positions/liquidation/apply.rs, contracts/pool/src/ops/withdraw.rs, contracts/pool/src/guards.rs.

## Decision

The controller receives or forwards inbound tokens and credits the pool only by
the measured amount that arrived. The pool performs controlled outbound
transfers. Requested amounts are never sufficient evidence of received value.

This accommodates fee-on-transfer and other non-standard token behavior
without minting claims against missing collateral.

## Guarantees

- Supply, repayment, recapitalization, and strategy settlement cannot
  over-credit a short transfer.
- Liquidation collateral seizures shrink when repayment under-delivers.
- Direct pool donations do not rewrite internal accounting.

## Auditor focus

Trace every inbound token route, including intermediate swaps and refunds, to
the final credited amount. Treat any requested-amount reuse as high risk.
