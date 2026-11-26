# Flash callbacks

`flash_loan` gives temporary cash, repaid through a pool allowance.
`flash_position` mints debt onto an account, and declared collateral returns
to the controller.

Implement `FlashLoanReceiver` and `FlashPositionReceiver` from
`xoxno_contract_sdk::lending`. Each trait fixes the callback name and arity.
`#[contractimpl] impl FlashLoanReceiver for MyContract` exports the callback.

These files are the sources:

- cash settlement:
  [`contracts/pool/src/ops/flash.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/pool/src/ops/flash.rs)
- position settlement:
  [`contracts/controller/src/strategies/flash_position.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/strategies/flash_position.rs)
- compiled callback fixtures:
  [`mock/flash-loan-receiver/src/lib.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/mock/flash-loan-receiver/src/lib.rs)
  and
  [`mock/flash-position-receiver/src/lib.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/mock/flash-position-receiver/src/lib.rs)

The mocks are test fixtures. Production receivers add trusted-invoker gates,
instance TTL renewal, payload validation, and application checks.

## Cash flash loan

The pool transfers `amount` and invokes:

```rust
fn execute_flash_loan(
    env: Env,
    initiator: Address,
    asset: Address,
    amount: i128,
    fee: i128,
    pool: Address,
    data: Bytes,
);
```

The pool then requires its balance to equal its post-payout level, checks
allowance for `amount + fee`, and calls `transfer_from`. A failed check raises
`InvalidFlashloanRepay`. Therefore, follow these rules:

- approve the pool, and never transfer repayment directly
- use the callback `fee`, and never recompute it
- hold at least `amount + fee` when the callback returns
- use a short-lived allowance and checked addition

The following example shows a cash callback:

```rust
use soroban_sdk::{contract, contractimpl, panic_with_error, Address, Bytes, Env};
use xoxno_contract_sdk::lending::helpers::approve_flash_repayment;
use xoxno_contract_sdk::lending::FlashLoanReceiver;

#[contract]
pub struct Receiver;

#[contractimpl]
impl FlashLoanReceiver for Receiver {
    fn execute_flash_loan(
        env: Env,
        initiator: Address,
        asset: Address,
        amount: i128,
        fee: i128,
        pool: Address,
        data: Bytes,
    ) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
        let cfg = config(&env);
        cfg.pool.require_auth();
        if pool != cfg.pool || initiator != cfg.operator {
            panic_with_error!(&env, ReceiverError::InvalidCaller);
        }
        let plan = decode_and_validate(&env, &data);
        execute_plan(&env, &plan, &asset, amount);
        let total = amount
            .checked_add(fee)
            .unwrap_or_else(|| panic_with_error!(&env, ReceiverError::Overflow));
        approve_flash_repayment(&env, &asset, &pool, total);
    }
}
```

`approve_flash_repayment` lets the pool pull `total` from the receiver until
the next ledger. Direct invoker auth covers the `approve`. Check balance and
profit before you approve. For the TTL constants, see
[SKILL.md](SKILL.md#storage-ttl).

The initiator is never the receiver. The initiator is still on the call stack
when the pool calls the receiver, and the host rejects re-entry
([Reentrancy](#reentrancy)). Start the loan from an account or a separate
contract stored as `cfg.operator`. A contract initiator authorizes against a
stored address ([SKILL.md](SKILL.md#contract-caller-rules) rule 6).

## Flash position

`flash_position` creates or reuses a `Multiply`, `Long`, or `Short` account,
mints debt, transfers the measured receipt to the receiver, and invokes:

```rust
fn execute_flash_position(
    env: Env,
    initiator: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
    fee: i128,
    amount_received: i128,
    controller: Address,
    data: Bytes,
);
```

`fee` is `0`. The callback neither repays nor approves debt. It transfers
purchased collateral to the controller. After the callback returns, the
controller performs these steps:

1. measures each declared collateral balance delta
2. requires every minimum and at least one positive deposit
3. deposits positive declared deltas on `account_id`
4. refunds positive deltas for declared refund assets to the caller
5. requires debt and collateral to remain open
6. runs LTV, health-factor, and minimum borrow collateral checks

Undeclared tokens left on the controller are neither deposited nor refunded.
Refunded unused debt tokens do not repay the minted debt.

The following example shows a position callback:

```rust
use soroban_sdk::{contractimpl, panic_with_error, token, Address, Bytes, Env};
use xoxno_contract_sdk::lending::FlashPositionReceiver;

#[contractimpl]
impl FlashPositionReceiver for Receiver {
    #[allow(clippy::too_many_arguments)]
    fn execute_flash_position(
        env: Env,
        initiator: Address,
        account_id: u64,
        asset: Address,
        _amount: i128,
        _fee: i128,
        amount_received: i128,
        controller: Address,
        data: Bytes,
    ) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
        let cfg = config(&env);
        cfg.controller.require_auth();
        if controller != cfg.controller || initiator != cfg.operator {
            panic_with_error!(&env, ReceiverError::InvalidCaller);
        }
        validate_expected_account(&env, account_id);
        let plan = decode_and_validate(&env, &data);
        let collateral_out = swap_and_measure(&env, &asset, amount_received, &plan);
        token::Client::new(&env, &plan.collateral).transfer(
            &env.current_contract_address(),
            &controller,
            &collateral_out,
        );
    }
}
```

A plain token `transfer` sends the collateral under direct invoker auth.
`authorize_transfer_as_current` applies only to a pull another contract makes
in your next call, such as controller `supply`. Nine parameters trigger
clippy's `too_many_arguments` lint. An `allow` on the method covers the
macro-generated code. An `allow` on the `impl` block does not.

As with flash loans, the initiator is an account or a separate contract. A
contract initiator runs the resolve helper before `flash_position` and the
store helper on the returned ID
([positions.md](positions.md#canonical-local-account-pointer)). New accounts
belong to the initiator. Reuse needs current NFT ownership or an active
delegate grant. The receiver gains no ownership.

## Callback gate and payload

Callbacks are public. Require auth from the configured pool or controller,
check that the callback address equals it, and check the initiator, account,
and operation before you execute.

Encode a small `#[contracttype]` plan to XDR. Before you move funds, validate
token addresses, route, minimums, and expiry. Take addresses and protocol amounts
from callback arguments, never from `data`.

## Reentrancy

The host rejects a call into a contract already on the stack with
`Error(Context, InvalidAction)`. A callback cannot call the controller, views
included. A `flash_loan` callback cannot call the pool. The controller flash
guard also blocks user position verbs, strategy verbs, keeper updates,
revenue claims, liquidation, bad-debt cleanup, and recapitalization.

A callback may call the router, which is a separate contract, with its own
exact token authorization.

## Settlement completion

A normal callback return is not completion. Require simulation and final
success of the enclosing call, and check these conditions:

- cash flash loan: the pool pulled exactly `amount + fee` and booked the fee
- flash position: every post-callback controller step in
  [Flash position](#flash-position) passed

After a flash position, verify `account_exists`, NFT owner, mode, spoke, and the
expected supply and debt positions. Store and renew the local pointer only from
the returned account ID.
