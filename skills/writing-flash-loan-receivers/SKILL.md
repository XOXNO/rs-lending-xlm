---
name: writing-flash-loan-receivers
description: Use when writing a Soroban flash-loan receiver contract for XOXNO Lending — implementing the execute_flash_loan callback, approve-based repayment, or debugging InvalidFlashloanReceiver / InvalidFlashloanRepay errors.
---

# Writing XOXNO Lending Flash-Loan Receivers

**REQUIRED BACKGROUND:** the `lending-protocol-fundamentals` skill.

## Overview

Flash loans start at the controller and execute in the pool: the pool
transfers the loan to your receiver contract, invokes its callback, and pulls
back `amount + fee` via token allowance — all in one atomic transaction.

```rust
// Controller entrypoint (initiator side; also buildable off-chain — see
// the using-lending-sdk skill)
fn flash_loan(caller: Address, asset: HubAssetKey, amount: i128,
              receiver: Address, data: Bytes);
```

Markets must have `is_flashloanable = true` or the call reverts
(`FlashloanNotEnabled`). The fee is per-market `flashloan_fee` (bps,
capped at 500) and is booked as protocol revenue. The exact fee is passed into
your callback — never hardcode it.

## Receiver callback contract

`receiver` must be a **deployed Wasm contract** exposing:

```rust
pub fn execute_flash_loan(
    env: Env,
    initiator: Address,  // forwarded loan originator
    asset: Address,      // loaned token
    amount: i128,        // loaned amount, already transferred to you
    fee: i128,           // premium owed on top
    pool: Address,       // the pool that will pull repayment
    data: Bytes,         // your opaque payload, forwarded verbatim
);
```

Repayment is **pull-based**: during the callback, `approve` the pool for
`amount + fee` on the loaned token; after the callback returns, the pool
executes `transfer_from`. Do not `transfer` tokens back directly — the pool
verifies its own balance is unchanged by the callback and reverts on any
direct push.

```rust
// inside execute_flash_loan
let total = amount.checked_add(fee)
    .unwrap_or_else(|| panic_with_error!(&env, MyError::Overflow));
let expiration = env.ledger().sequence() + 1;
token::Client::new(&env, &asset)
    .approve(&env.current_contract_address(), &pool, &total, &expiration);
```

A reference implementation exercising all success/failure modes lives at
`contracts/flash-loan-receiver/src/lib.rs` in the protocol repo.

## Security requirements for production receivers

- **Gate the caller.** The reference receiver (`contracts/flash-loan-receiver/`) is test-only/adversarial harness (multi-mode XDR for smoke/fuzz); a
  production receiver MUST verify the invoker is the trusted pool (or
  controller) before acting on `data`.
- **Reentrancy is blocked protocol-side** (shared `FlashLoanOngoing` guard with strategies; see INVARIANTS §2.5, ADR 0006) — re-entering `flash_loan` or
  controller verbs from the callback reverts — but treat `data` as untrusted
  input regardless.
- **Approve exactly `amount + fee`** with a short expiration ledger; a
  standing unlimited allowance is a drain vector if the pool address in your
  code path is ever attacker-influenced.

## Failure modes

| Error | Cause |
|---|---|
| `FlashloanNotEnabled` | market `is_flashloanable` is false |
| `InvalidFlashloanReceiver` | `receiver` is not a deployed Wasm contract |
| `InsufficientLiquidity` | pool's tracked reserves can't fund `amount` |
| `InvalidFlashloanRepay` | allowance short, balance drifted, or callback pushed tokens directly |
| `AmountMustBePositive` | non-positive amount / negative fee |

## Common mistakes

- **Repaying by `transfer` instead of `approve`** — the pool's balance
  bracket check reverts the whole transaction.
- **Approving only `amount`** — the fee is owed too; approve `amount + fee`.
- **Using a G-address or undeployed contract as receiver** — rejected with
  `InvalidFlashloanReceiver`.
- **Calling the pool's `flash_loan` directly** — it is controller-only;
  initiate through the controller entrypoint.
- **Hardcoding the fee** — it is per-market config; read it from the
  callback argument.
