---
name: writing-flash-position-receivers
description: Use when writing a Soroban receiver for XOXNO Lending flash_position — the zero-fee callback-multiply entrypoint that mints account debt and requires a healthy account after the callback.
---

# Writing XOXNO Lending Flash-Position Receivers

**REQUIRED BACKGROUND:** `lending-protocol-fundamentals` and
`writing-flash-loan-receivers`.

## This is not a cash flash loan

`flash_loan` pays cash and pulls principal plus fee back. `flash_position`
mints **strategy debt** onto an account with **zero fee**, sends the tokens to
your contract, and requires that account to be solvent after you push
collateral to the controller.

Do **not** implement `execute_flash_loan`. Do **not** `approve` the pool.
There is no repayment pull.

Two gates run before the mint: `mode` must be `Multiply`, `Long` or `Short`
(`InvalidPositionMode`, #111), and the debt market must have
`is_flashloanable` set (`FlashloanNotEnabled`, #401). `multiply` is not
gated on the flag because its funds only ever reach the governance-owned
router; `flash_position` hands them to your contract.

```rust
fn flash_position(
    caller: Address,
    account_id: u64,   // 0 creates
    spoke_id: u32,
    mode: PositionMode,
    debt: HubAssetKey,
    amount: i128,
    receiver: Address,
    data: Bytes,
    collaterals: Vec<(HubAssetKey, i128)>, // >=1 asset, >=1 min > 0
    refund_assets: Vec<Address>,
) -> u64;
```

```rust
pub fn execute_flash_position(
    env: Env,
    initiator: Address,
    account_id: u64,
    asset: Address,
    amount: i128,            // gross debt minted
    fee: i128,               // always 0
    amount_received: i128,   // measured tokens already on this contract
    controller: Address,     // push collaterals here
    data: Bytes,
);
```

During the callback, transfer listed collateral tokens to `controller`. After
it returns, the controller measures its balance increase, requires each
declared minimum, deposits onto `account_id`, refunds `refund_assets` deltas
to `caller`, and runs ordinary solvency gates.

Re-entering `supply` / `borrow` / `flash_loan` / other strategies reverts
`FlashLoanOngoing` (#400).

## Common mistakes

- Reusing a cash flash-loan receiver (`execute_flash_loan` / pool approve).
- Sending collateral to the pool instead of the controller.
- Empty `collaterals` or all-zero mins (`InvalidPayments` / `CollateralRequired`).
- Assuming leftover debt token on the controller repays the account — it does not.

## Leftovers and refunds

Nothing is auto-repaid. The account keeps the full minted debt and accrues
interest on it until someone calls `repay`. Three places tokens can end up:

- **On your receiver.** Whatever you do not push to the controller stays on
  your contract. The protocol never touches it again.
- **Pushed to the controller and listed in `refund_assets`.** The controller
  refunds its balance increase in each listed asset to `caller` after the
  callback. Each entry must be an active listed asset of the account's spoke,
  no duplicates, at most `max_supply_positions` entries. Listing the debt
  token here is the way to get an unspent remainder back; then `repay` it.
- **Pushed to the controller, not a declared collateral, not in
  `refund_assets`.** Stranded on the controller. It is credited to no position
  and refunded to nobody.

The collateral minimum only has to be strictly positive, so a healthy
account can use this call to route borrowed tokens to your receiver against
dust collateral. That is intended and is economically the same as
`borrow(..., to)` on that account: the same solvency, min-borrow, and cap
gates run at finalize, and the debt is real.

## Test mock and live coverage

`mock/flash-position-receiver/` is the testnet mock (pre-fund, `set_plan`,
push). It does not mint. Run it against a freshly deployed controller:

```bash
make integration-flash-position
```
