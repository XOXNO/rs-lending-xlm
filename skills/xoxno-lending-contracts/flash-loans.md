# Flash callbacks

Use `flash_loan` for temporary cash repaid through a pool allowance. Use
`flash_position` to mint debt onto an account and return declared collateral
to the controller.

Authoritative sources:

- cash settlement:
  [`contracts/pool/src/ops/flash.rs`](../../contracts/pool/src/ops/flash.rs)
- position settlement:
  [`contracts/controller/src/strategies/flash_position.rs`](../../contracts/controller/src/strategies/flash_position.rs)
- compiled callback fixtures:
  [`mock/flash-loan-receiver/src/lib.rs`](../../mock/flash-loan-receiver/src/lib.rs)
  and
  [`mock/flash-position-receiver/src/lib.rs`](../../mock/flash-position-receiver/src/lib.rs)

The mocks are test fixtures, not deployable production receivers. Reuse their
exact callback arity and token auth shape, while adding trusted-invoker gates,
instance TTL renewal, payload validation, and application-specific checks.

## Cash flash loan

The pool transfers `amount`, invokes:

```rust
pub fn execute_flash_loan(
    env: Env,
    initiator: Address,
    asset: Address,
    amount: i128,
    fee: i128,
    pool: Address,
    data: Bytes,
);
```

Then it requires the receiver balance to remain at the post-payout level,
checks allowance for `amount + fee`, and calls `transfer_from`. Therefore:

- approve the pool; never transfer repayment directly
- use the callback `fee`, do not recompute it
- keep at least `amount + fee` until callback return
- use a short-lived allowance and checked addition

Focused production shape:

```rust
use common::ttl::renew_instance;
use soroban_sdk::{panic_with_error, Address, Bytes, Env};

pub fn execute_flash_loan(
    env: Env,
    initiator: Address,
    asset: Address,
    amount: i128,
    fee: i128,
    pool: Address,
    data: Bytes,
) {
    renew_instance(&env);
    let cfg = config(&env);
    cfg.pool.require_auth();
    if pool != cfg.pool || initiator != env.current_contract_address() {
        panic_with_error!(&env, ReceiverError::InvalidCaller);
    }
    let plan = decode_and_validate(&env, &data);
    execute_plan(&env, &plan, &asset, amount);
    let total = amount
        .checked_add(fee)
        .unwrap_or_else(|| panic_with_error!(&env, ReceiverError::Overflow));
    approve_repayment(&env, &asset, &pool, total);
}
```

`approve_repayment` must self-authorize the exact token `approve` invocation;
use the compiled implementation in
[`mock/flash-loan-receiver`](../../mock/flash-loan-receiver/src/lib.rs).
Balance and profit checks belong before approval.

The public initiator entrypoint must also renew the receiver instance before
calling `ControllerClient::flash_loan`.

## Flash position

`flash_position` creates or reuses a `Multiply`, `Long`, or `Short` account,
mints debt, transfers the measured receipt to the receiver, and invokes:

```rust
pub fn execute_flash_position(
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

`fee` is currently zero. The callback does not repay or approve debt. It
transfers purchased collateral from the receiver to the controller. After
return, the controller:

1. measures each declared collateral balance delta
2. requires every minimum and at least one positive deposit
3. deposits positive declared deltas on `account_id`
4. refunds positive deltas for declared refund assets
5. requires debt and collateral to remain open
6. runs LTV, health-factor, and minimum-borrow settlement checks

An undeclared token left on the controller is neither deposited nor refunded.
Refunding unused debt tokens does not repay the minted debt.

Focused callback shape:

```rust
use common::token::authorize_transfer_as_current;
use common::ttl::renew_instance;
use soroban_sdk::{panic_with_error, token, Address, Bytes, Env};

pub fn execute_flash_position(
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
    renew_instance(&env);
    let cfg = config(&env);
    cfg.controller.require_auth();
    if controller != cfg.controller || initiator != env.current_contract_address() {
        panic_with_error!(&env, ReceiverError::InvalidCaller);
    }
    validate_expected_account(&env, account_id);
    let plan = decode_and_validate(&env, &data);
    let collateral_out = swap_and_measure(&env, &asset, amount_received, &plan);
    let me = env.current_contract_address();
    authorize_transfer_as_current(
        &env,
        &plan.collateral,
        &me,
        &controller,
        collateral_out,
    );
    token::Client::new(&env, &plan.collateral).transfer(
        &me,
        &controller,
        &collateral_out,
    );
}
```

The initiator branch must call the resolve helper before `flash_position` and
the store helper with the returned ID afterward; both helpers are in
[positions.md](positions.md#canonical-local-account-pointer). Its public
entrypoint must call `renew_instance(&env)` as well.

## Callback gate and payload

Callbacks are public. Require auth from the configured pool/controller and
compare the callback address argument with that configured address. Also bind
the callback to an expected initiator/account/operation so a valid protocol
caller cannot execute an unintended plan.

Encode a small `#[contracttype]` plan to XDR. Decode and validate token
addresses, route, minimums, and expiry before moving funds. Do not trust
addresses or protocol-computed amounts copied into `data`; use callback
arguments for those.

## Reentrancy

During either callback the controller flash guard blocks user position verbs,
strategy verbs, keeper updates, liquidation, and recapitalization. Views and
the owner-only account/delegate renewal paths are not guarded the same way.
Do not design a callback that depends on reentering controller mutation.

The router is a separate contract and may be called inside a callback with its
own exact token authorization.

## Settlement completion

A callback returning normally is not completion. Require transaction
simulation and final success of the enclosing call:

- cash flash loan: pool pulled exactly `amount + fee` and booked the fee
- flash position: controller measured the declared collateral minimum,
  deposited it on the expected account, processed refunds, and passed open
  position plus solvency checks

After a successful flash position, verify `account_exists`, NFT owner, mode,
spoke, and the expected supply/debt positions. Store/renew the local pointer
only from the returned account ID.
