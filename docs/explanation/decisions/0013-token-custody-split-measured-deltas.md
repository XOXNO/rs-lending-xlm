# 0013. Token custody split: the controller pre-transfers and credits measured deltas; the pool only transfers out

Status: Accepted

## Context

Tokens on Soroban are contracts, and a listed token is not guaranteed to be
well-behaved: fee-on-transfer, rebasing, or otherwise non-standard transfer
semantics can make the amount that arrives differ from the amount that was sent.
A lending protocol that credits the *declared* amount against such a token
mints unbacked claims. At the same time, the pool is the custody layer for all
markets and deliberately holds no user, risk, or pricing state — every inbound
payment is initiated by the controller on a user's behalf. The design question
is where token movement and amount verification live, and which layer is
allowed to trust which numbers.

## Decision

The pool never pulls tokens for supply, repay, or recapitalize. The controller
moves tokens caller-to-pool first, then invokes the pool with the amount as a
plain argument — and that argument is always a *measured* quantity:
`contracts/controller/src/payments/transfer.rs::transfer_amount_measured`
snapshots the recipient's balance, performs the transfer, and returns
`post - pre`. Its rustdoc states the rationale directly: credit only what
actually arrived, so a fee-on-transfer token cannot cause the protocol to
credit more than it received. Every inbound flow routes through it — supply
(`contracts/controller/src/positions/supply.rs`), repay
(`contracts/controller/src/positions/repay.rs`), liquidation legs
(`contracts/controller/src/positions/liquidation/apply.rs`), recapitalization
(`contracts/controller/src/keepers/mod.rs::recapitalize`), and strategy legs
(`contracts/controller/src/strategies/legs.rs`).

Liquidation goes one step further: when a repayment leg under-delivers, the
received value is what credits the debt book, and
`contracts/controller/src/positions/liquidation/math.rs::scale_seizures_to_received`
shrinks the planned collateral seizures by `received / planned` with floor
rounding, so a liquidator paying with a fee-on-transfer token cannot collect
collateral they never paid for.

Inside the pool, custody is outbound-only: the sole general-purpose token
operation is `contracts/pool/src/cache/cash.rs::transfer_out`. The flash-loan
path (`contracts/pool/src/ops/flash.rs`) is the single exception — it pays the
principal out and pulls repayment via the pool's only `transfer_from`, and it is
also the only code that reads the pool's live token balance (see ADR 0010).
Everywhere else the pool trusts the controller-declared amount and maintains its
tracked `cash` counter from those declarations alone.

## Alternatives

**Pool-side `transfer_from` with user allowances granted to the pool.** Users
would approve the pool and the pool would pull on demand. That puts amount
verification in the layer with no pricing or risk context, forces users to
maintain allowances against an internal contract they never call, and couples
the pool's ABI to token-behavior edge cases. Keeping the pool's inbound surface
at zero (flash aside) makes its custody audit trivially small.

**Trusting the declared transfer amount.** Assuming standard-token semantics
and crediting the requested amount is simpler and one balance-read cheaper, but
it converts any non-standard token listing into an unbacked-claim mint, and
listing review becomes a load-bearing security control. Measuring the delta
makes the token's actual behavior part of the verified surface instead of an
assumption.

**Reconciling the pool's tracked `cash` against its live token balance.**
The pool could periodically true itself up to `balance()`. This is rejected
deliberately: donations to the pool address must not raise `cash`, because a
donation-driven exchange-rate or utilization shift is a classic manipulation
primitive. Tracked cash moves only through accounted operations; surplus
tokens sit inert.

## Consequences

Inbound value is verified exactly once, at the trust boundary where the caller
lives, and the number the pool books equals the number that arrived — this is
what keeps the ACCT domain of ../../reference/invariants.md closed under
non-standard tokens, and the liquidation scaling keeps the LIQ domain's
value-for-value property honest (see ../threat-model.md for the malicious-token
attacker profile). Users sign the familiar pre-authorization pattern against the
controller only; the pool never appears in their auth trees.

What it makes easy: the pool's token-handling audit is three call sites
(`transfer_out` plus the two flash operations); donation attacks are inert by
construction.

What it makes hard: genuinely rebasing-down tokens still shrink pool holdings
after crediting — measuring at arrival bounds crediting, not later drift, so
listing policy still matters. Every new inbound flow must route through
`transfer_amount_measured` and pass the measured value onward; a single call
site that forwards the requested amount instead re-opens the fee-on-transfer
hole. The pool must stay free of new inbound token paths, or the flash-loan
balance assertions and the donation-inertness argument both erode.
