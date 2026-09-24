# Flash callbacks

Use `flash_loan` for temporary cash repaid through a pool allowance. Use
`flash_position` to mint debt onto an account and return declared collateral
to the controller.

Implement the callback traits from `xoxno_contract_sdk::lending`:
`FlashLoanReceiver` and `FlashPositionReceiver`. Each trait fixes the callback
name and arity, and `#[contractimpl] impl FlashLoanReceiver for MyContract`
exports the callback.

Authoritative sources:

- cash settlement:
  [`contracts/pool/src/ops/flash.rs`](../../contracts/pool/src/ops/flash.rs)
- position settlement:
  [`contracts/controller/src/strategies/flash_position.rs`](../../contracts/controller/src/strategies/flash_position.rs)
- compiled callback fixtures:
  [`mock/flash-loan-receiver/src/lib.rs`](../../mock/flash-loan-receiver/src/lib.rs)
  and
  [`mock/flash-position-receiver/src/lib.rs`](../../mock/flash-position-receiver/src/lib.rs)

The mocks are test fixtures, not deployable production receivers. A
production receiver adds trusted-invoker gates, instance TTL renewal, payload
validation, and application-specific checks.

## Cash flash loan

The pool transfers `amount`, invokes:

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

Then it requires the pool balance to still equal its post-payout level,
checks allowance for `amount + fee`, and calls `transfer_from`. A failed
balance or allowance check raises `InvalidFlashloanRepay`. Therefore:

- approve the pool; never transfer repayment directly
- use the callback `fee`, do not recompute it
- hold at least `amount + fee` when the callback returns
- use a short-lived allowance and checked addition

Focused production shape:

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

`approve_flash_repayment` approves the pool to pull `total` from the receiver,
with an expiration at the next ledger. The receiver calls the token itself, so
direct invoker auth covers the `approve`; it needs no extra auth entry.
Balance and profit checks belong before approval. The TTL constants are in
[SKILL.md](SKILL.md#storage-ttl).

The initiator is never the receiver itself. The pool calls the receiver while
the initiator is still on the call stack, and the host rejects a call into a
contract that is already on the stack (see [Reentrancy](#reentrancy)). Start
the loan from an account or from a separate contract, and store its address as
`cfg.operator`. A contract initiator authorizes its entrypoint against a
stored address, as in [SKILL.md](SKILL.md#contract-caller-rules) rule 7.

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

The controller passes `fee` as `0`. The callback does not repay or approve
debt. It transfers purchased collateral from the receiver to the controller.
After return, the controller:

1. measures each declared collateral balance delta
2. requires every minimum and at least one positive deposit
3. deposits positive declared deltas on `account_id`
4. refunds positive deltas for declared refund assets to the caller
5. requires debt and collateral to remain open
6. runs LTV, health-factor, and minimum borrow collateral checks

An undeclared token left on the controller is neither deposited nor refunded.
Refunding unused debt tokens does not repay the minted debt.

Focused callback shape:

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

Send the collateral with a plain token `transfer` from the receiver. The
receiver calls the token itself, so direct invoker auth covers it. Use
`authorize_transfer_as_current` only for a pull that another contract makes
inside your next call, such as a controller `supply`. The callback has nine
parameters, so clippy's `too_many_arguments` lint fires in your crate. The
`allow` on the method suppresses it for the code the macro generates; an
`allow` on the `impl` block does not.

The initiator is an account or a separate contract, as for flash loans. A
contract initiator calls the resolve helper before `flash_position` and the
store helper with the returned ID afterward; both helpers are in
[positions.md](positions.md#canonical-local-account-pointer). The account the
initiator opens or reuses is its own, not the receiver's.

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

The Soroban host rejects a call into a contract that is already on the call
stack with `Error(Context, InvalidAction)`. A callback therefore cannot call
the controller, including its views, and a `flash_loan` callback cannot call
the pool. The controller flash guard is a second layer: it blocks user
position verbs, strategy verbs, keeper updates, revenue claims, liquidation,
bad-debt cleanup, and recapitalization. Do not design a callback that calls
the controller.

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
