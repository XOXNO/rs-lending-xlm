# 0020. Flash position is zero-fee strategy debt with an external callback

**Status:** Accepted

## Decision

A new controller strategy, `flash_position`, mints strategy debt with `charge_fee = false`, forwards the measured tokens to a caller-chosen Wasm receiver, and accepts only measured collateral pushes onto the same account. The account must be solvent at `strategy_finalize`. The callback cannot repay, withdraw, or re-enter monetary controller paths.

This is the multiply *shape* (mint strategy debt, obtain collateral, solvency
gate) with the aggregator swap replaced by an external callback. It is not a
cash flash loan. Unlike `multiply`, `PositionMode::Normal` and same-asset
borrow-then-supply are allowed; solvency is the gate.

Zero fee is allowed because this entrypoint cannot round-trip to a closed
position. A successful call must still hold the minted debt and at least
one supply position (`FlashPositionClosed` otherwise). The only existing `charge_fee = false` path, Blend migration,
confines proceeds to a governance-approved destination. Here the confinement
is weaker: the call must leave an open, solvent position on the authorized
account.

A strictly positive collateral *minimum* is required, but it may be dust. An
already-healthy account can therefore extract debt and leave the tokens on the
receiver without posting economically meaningful new collateral. That is
accepted: it is the same as `borrow(..., to)` on that account, including the
ordinary solvency / min-borrow / cap gates. It is *not* a cash flash loan and
must not steal unaccounted pool or third-party funds. Leftover debt token is
never auto-repaid.

## Guarantees

- Pool `flash_loan` still requires exact principal-plus-fee repayment and `is_flashloanable`.
- `flash_position` honours `is_flashloanable` on the debt market and accepts only the Multiply, Long and Short modes: the minted debt reaches a caller-chosen contract, the exact custody the flag denies. `multiply` stays ungated because its funds reach only the governance-owned router.
- Strategy debt is minted before the callback; pool cash and debt shares stay consistent.
- The receiver cannot keep the funds unless the account was already solvent enough to borrow them, and at least one strictly positive collateral minimum is supplied.
- Leftover undeclared tokens are never credited as positions. Caller-listed `refund_assets` can recover them.
- Prices used at finalize are the pre-callback snapshot.

## Auditor focus

Round-trip attempts (push debt token back and hope for auto-repay), empty/all-zero collateral lists, `receiver = controller` or `receiver = pool`, reentry of every guarded entrypoint, fee-on-transfer debt tokens, `is_flashloanable = false` refusing the call, `PositionMode::Normal` refusing the call, and protocol revenue remaining unchanged.
