# Testing with `LendingFixture`

`xoxno_contract_sdk::testutils::LendingFixture` deploys XOXNO Lending into a
test `Env` from the embedded WASM: governance, controller, pool, position NFT,
price aggregator, and two mock oracles, with one hub and one spoke.
Governance owns the controller, as on mainnet. Every configuration step goes
through its timelock. Tests run on the host. No Wasm build is needed.

A fixture test covers only the embedded WASM (crate `0.2.0`, protocol
`v1.1.0`, commit `1053ae033`). It does not cover later changes, such as the current
low-decimal rules. Compare the target deployment with the embedded WASM before
you choose the crate, and simulate on the target network. Enable `testutils` only in
dev-dependencies ([SKILL.md](SKILL.md#dependencies)).

## Fixture API

The fixture exposes these functions and fields:

- `LendingFixture::deploy(&env, &admin)`: deploys the fixture. `admin` holds
  every governance role.
- `create_market(&MarketConfig::usdc())`: creates a 7-decimal token, lists it
  in the default hub (`fixture.hub_id`) and spoke (`fixture.spoke_id`) with a
  dual-source oracle, and seeds liquidity from a new provider.
  `MarketConfig::usdc()` and `MarketConfig::xlm()` use mainnet market parameters
  and "Blue Chip" asset settings. The returned `Market` has `key` (controller
  `HubAssetKey`), `asset`, `token`, and `sac` (asset admin client, for `mint`).
- `fixture.addresses()`: `LendingAddresses` for your constructor.
- `set_price(&market, price_wad)`: USD price (WAD) on both mock feeds.
- `advance_time(seconds)`: moves the timestamp and the sequence, at one ledger per
  5 s. It republishes every price and accrues interest on every market.
- `add_hub`, `add_spoke`, `create_market_in`, `list_market`,
  `add_market_to_hub`, `supply_liquidity`: more hubs, spokes, and listings.
- `fixture.controller`, `fixture.pool`, `fixture.position_nft`,
  `fixture.price_aggregator`: generated clients for assertions.

## Env changes

`deploy` changes the `Env`:

- The timestamp is at least 1,000,000 and the sequence is at least 100. Each
  timelocked setup step advances the sequence by one ledger.
- The minimum persistent entry TTL is 10,000,000 ledgers, and the maximum is
  above it. New persistent entries, yours included, start at the minimum.
  Tests therefore never archive them and cannot prove your contract renews its
  keys.
- The budget is unlimited, because setup exceeds the default. To check one
  call, run `env.cost_estimate().budget().reset_default()` immediately before the
  call and read `env.cost_estimate()` after the call (see the example below).
  A contract registered as a Rust type runs natively and costs less than the
  same contract as WASM. Protocol contracts run as WASM.

The fixture authorizes its admin calls per call and leaves the `Env` auth
mode unchanged.

## Test your own authorization

`env.mock_all_auths()` accepts every authorization in the tree, so it hides
missing or wrong contract auth. In tests, mock only external users:

- `client.mock_auths(&[MockAuth { .. }])` mocks a user call, with the exact
  arguments and sub-invocations that the user signs
- `client.mock_all_auths()` on one client mocks another actor's setup call, for
  example `usdc.sac.mock_all_auths().mint(..)`

Your contract's auth then runs as on the network. It uses direct invoker auth
for `caller.require_auth()` and `authorize_transfer_as_current` entries for
nested pulls. A missing or wrong entry fails the test.

## Example

The contract under test takes `LendingAddresses`, the market, and the spoke in
its constructor, checks the pool against `get_pool_address()`, and stores
them. `deposit` uses the pointer helpers from
[positions.md](positions.md#canonical-local-account-pointer):

```rust
pub fn deposit(env: Env, from: Address, amount: i128) -> u64 {
    from.require_auth();
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
    let cfg: Config = env.storage().instance().get(&Key::Config).unwrap();
    token::Client::new(&env, &cfg.market.asset).transfer(
        &from,
        env.current_contract_address(),
        &amount,
    );
    let lending = XoxnoLending::new(&env, &cfg.lending);
    let account_id = resolve_account(&env, &lending.controller());
    let account_id = lending.deposit(account_id, cfg.spoke_id, &cfg.market, amount);
    store_account(&env, account_id);
    account_id
}
```

The tests mock only the user's signature and the setup calls of other actors:

```rust
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{vec, Address, Env, IntoVal};
use xoxno_contract_sdk::lending::constants::{NEW_ACCOUNT, WAD};
use xoxno_contract_sdk::testutils::{LendingFixture, MarketConfig};

const UNIT: i128 = 10_000_000;
const DAY: u64 = 86_400;

fn deposit(env: &Env, vault: &MyVaultClient, token: &Address, user: &Address, amount: i128) -> u64 {
    vault
        .mock_auths(&[MockAuth {
            address: user,
            invoke: &MockAuthInvoke {
                contract: &vault.address,
                fn_name: "deposit",
                args: (user, amount).into_val(env),
                sub_invokes: &[MockAuthInvoke {
                    contract: token,
                    fn_name: "transfer",
                    args: (user, &vault.address, amount).into_val(env),
                    sub_invokes: &[],
                }],
            },
        }])
        .deposit(user, &amount)
}

#[test]
fn deposit_opens_one_account_and_earns_interest() {
    let env = Env::default();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let xlm = fixture.create_market(&MarketConfig::xlm());
    let vault = MyVaultClient::new(
        &env,
        &env.register(
            MyVault,
            (fixture.addresses(), usdc.key.clone(), fixture.spoke_id),
        ),
    );

    let user = Address::generate(&env);
    usdc.sac.mock_all_auths().mint(&user, &(1_500 * UNIT));
    let account_id = deposit(&env, &vault, &usdc.asset, &user, 1_000 * UNIT);
    assert_eq!(
        deposit(&env, &vault, &usdc.asset, &user, 500 * UNIT),
        account_id
    );
    assert_eq!(
        fixture
            .position_nft
            .owner_of(&u32::try_from(account_id).unwrap()),
        vault.address
    );

    let borrower = Address::generate(&env);
    xlm.sac.mock_all_auths().mint(&borrower, &(100_000 * UNIT));
    let borrower_account = fixture.controller.mock_all_auths().supply(
        &borrower,
        &NEW_ACCOUNT,
        &fixture.spoke_id,
        &vec![&env, (xlm.key.clone(), 100_000 * UNIT)],
    );
    fixture.controller.mock_all_auths().borrow(
        &borrower,
        &borrower_account,
        &vec![&env, (usdc.key.clone(), 5_000 * UNIT)],
        &None,
    );
    fixture.set_price(&xlm, WAD * 12 / 100);
    fixture.advance_time(30 * DAY);

    let collateral = fixture
        .controller
        .get_collateral_amount(&account_id, &usdc.key);
    assert!(collateral > 1_500 * UNIT);
}

#[test]
fn deposit_without_the_user_signature_fails() {
    let env = Env::default();
    let fixture = LendingFixture::deploy(&env, &Address::generate(&env));
    let usdc = fixture.create_market(&MarketConfig::usdc());
    let vault = MyVaultClient::new(
        &env,
        &env.register(
            MyVault,
            (fixture.addresses(), usdc.key.clone(), fixture.spoke_id),
        ),
    );
    let user = Address::generate(&env);
    usdc.sac.mock_all_auths().mint(&user, &(1_000 * UNIT));

    assert!(vault.try_deposit(&user, &(1_000 * UNIT)).is_err());
    assert_eq!(usdc.token.balance(&user), 1_000 * UNIT);
}
```

To check the deposit against the default budget:

```rust
    env.cost_estimate().budget().reset_default();
    deposit(&env, &vault, &usdc.asset, &user, 1_000 * UNIT);
    let resources = env.cost_estimate().resources();
```

A test is evidence for the fixture state only. Before submission, still
require successful simulation of the exact transaction on the target network
([SKILL.md](SKILL.md#completion-checks)).
