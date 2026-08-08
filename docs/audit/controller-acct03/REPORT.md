# Controller INV-ACCT-03 audit report

**Confirmed findings:** 0

## Context

INV-ACCT-03: credit only measured receipt. transfer_amount_measured snapshots recipient balance, transfers, returns post-pre; supply/repay/liq/recap/strategy-repay book that delta (not requested amount), so fee-on-transfer cannot mint unbacked claims. Liq also scales seizures by received/planned. balance_delta measures controller receipts (withdraw/swap/migrate). Outbound claim/refunds skip measure. Suspicious: multiply initial payment uses raw transfer + requested amount intermediately; final pool credit still remeasures via supply.

## Coverage

| Site | OK | Inspected | Kept |
|------|----|-----------|------|
| helper | yes | 20 | 0 |
| supply | yes | 14 | 0 |
| repay | yes | 12 | 0 |
| liquidation | yes | 14 | 0 |
| keepers | yes | 6 | 0 |
| strategy | yes | 16 | 0 |
| grep-bypass | yes | 20 | 0 |

## Residual notes

### helper
Audited transfer_amount_measured and all pool-inbound call sites (supply/repay/liq/recap/strategy-repay). Traced amount to pool args; FoT under-delivery scales liq seizures. balance_delta is controller-only. Multiply raw payment remeasured on final supply. No confirmed over-credit vs measured pool receipt.

### supply
Traced process_supply→aggregate→build_supply_entries→transfer_amount_measured(caller→pool)→make_pool_action(received)→pool_supply_call→merge. Multi-asset: per-leg measure after HubAssetKey aggregate. Pool trusts hub amount (cash=measured). Strategy process_deposit callers remeasure controller→pool. No FoT path credits amount_in.

### repay
Traced process_repay→build_repay_actions→transfer_amount_measured(caller→pool)→make_pool_action(measured)→pool_repay_call→merge_debt_leg. Overpay refund uses measured excess to payer. Permissionless repay auth-only; no requested-amount re-use. Compared supply/liq/strategy measured sites. Pool ops/repay trusted amount as hub-measured (ADR-0013).

### liquidation
Traced liquidate→process_liquidation→plan repay entries→transfer_amount_measured(to=pool)→make_pool_action(received)→pool_repay; under-delivery leg_usd floor + scale_seizures_to_received; seizures outbound unmeasured. No path credits debt/seizure from requested after measure. Bad-debt seize has no inbound transfer.

### keepers
Audited keepers inbound: recapitalize measures pool balance delta via transfer_amount_measured(payer→pool) and passes received to pool_recapitalize_call. claim_revenue is outbound (pool transfer_out to owner then controller→accumulator) and does not credit supply/debt. update_indexes/update_account_threshold move no tokens. No FoT path over-credits cash vs receipt.

### strategy
Traced strategy repay (legs::repay_debt_from_controller), swap_debt, repay_with_collateral, swap_collateral, multiply deposit, migrate_blend deposit/refund through transfer_amount_measured/balance_delta into process_deposit/execute_repayment. Pool credit args use measured deltas (to=pool). Multiply initial_payment raw transfer intermediate only; final supply remeasures. No confirmed FoT unbacked mint path.

### grep-bypass
All pool-inbound paths (supply/repay/liq/recap/strategy-repay) use transfer_amount_measured(to=pool) and book delta. Swaps/migrate use balance_delta then remeasure on deposit. Multiply initial_payment is raw+requested intermediate only; final process_deposit remeasures—FoT fails closed. Claim/refunds outbound. No confirmed unbacked credit.

