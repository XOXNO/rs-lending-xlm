# Testing with `LendingFixture`

`xoxno_contract_sdk::testutils::LendingFixture` deploys XOXNO Lending into a
test `Env` from the WASM that the crate embeds. It deploys governance, the
controller, the pool, the position NFT, the price aggregator, and two mock
oracles, with one hub and one spoke. Governance owns the controller, as on
mainnet, and every configuration step goes through the governance timelock.
The tests run on the host; they do not need a Wasm build of your contract.

Enable the fixture only in the dev-dependencies:

```toml
[dev-dependencies]
soroban-sdk = { version = "28", features = ["testutils"] }
xoxno-contract-sdk = { version = "0.1", features = ["testutils"] }
```

## Fixture API

- `LendingFixture::deploy(&env, &admin)` deploys the protocol. `admin` holds
  every governance role.
- `create_market(&MarketConfig::usdc())` creates a 7-decimal token, lists it
  in the default hub (`fixture.hub_id`) and spoke (`fixture.spoke_id`) with a
  dual-source oracle, and supplies the initial liquidity from a new provider.
  `MarketConfig::usdc()` and `MarketConfig::xlm()` use the mainnet market
  parameters and the "Blue Chip" asset settings. The returned `Market` has
  `key` (the controller `HubAssetKey`), `asset`, `token`, and `sac` (the asset
  admin client, for `mint`).
- `fixture.addresses()` returns the `LendingAddresses` for your contract's
  constructor.
- `set_price(&market, price_wad)` sets the USD price, WAD, on both mock feeds.
- `advance_time(seconds)` moves the timestamp, and the sequence by one ledger
  for each 5 seconds. Then it publishes every price again and accrues interest
  on every market.
- `add_hub`, `add_spoke`, `create_market_in`, `list_market`,
  `add_market_to_hub`, and `supply_liquidity` add hubs, spokes, and listings.
- `fixture.controller`, `fixture.pool`, `fixture.position_nft`, and
  `fixture.price_aggregator` are the generated clients, for assertions.

## Env changes

`deploy` changes the `Env`:

- The timestamp becomes at least 1,000,000 and the sequence at least 100.
  Each timelocked setup step moves the sequence forward by one ledger.
- The minimum persistent entry TTL becomes 10,000,000 ledgers, and the maximum
  entry TTL is above it. New persistent entries, your contract's included,
  start with this TTL, so a fixture test does not archive them. It does not
  prove that your contract renews its keys.
- The budget becomes unlimited, because the fixture setup exceeds the default
  test budget. To check one call against the network limits, call
  `env.cost_estimate().budget().reset_default()` just before the call, and
  read `env.cost_estimate()` after it. A contract registered as a Rust type
  runs natively, so its own costs are lower than as WASM; the protocol
  contracts run as WASM.

The fixture authorizes its own admin calls per call. It does not change the
auth mode of the `Env`.

## Test your own authorization

`env.mock_all_auths()` accepts every `require_auth` in the call tree. A test
with it passes even when your contract creates a wrong or missing
authorization entry. Do not use it for the calls under test. Mock only the
auth of external actors, per call:

- `client.mock_auths(&[MockAuth { .. }])` for a user call, with the exact
  arguments and sub-invocations the user signs
- `client.mock_all_auths()` on one client, for a setup call by another actor,
  such as `usdc.sac.mock_all_auths().mint(..)`

The auth of your contract then runs as on the network: direct invoker auth
for `caller.require_auth()`, and the `authorize_transfer_as_current` entries
(your own or the wrapper's) for nested token pulls. A missing or wrong entry
fails the test.

## Example

The contract under test takes `LendingAddresses`, the market, and the spoke in
its constructor, checks the pool against `get_pool_address()`, and stores
them. `deposit` uses the pointer helpers in
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

To check the deposit against the default budget, reset it just before the
call:

```rust
    env.cost_estimate().budget().reset_default();
    deposit(&env, &vault, &usdc.asset, &user, 1_000 * UNIT);
    let resources = env.cost_estimate().resources();
```

A test is evidence for the fixture state only. Before submission, still
require a successful simulation of the exact transaction on the target
network; see [SKILL.md](SKILL.md#completion-checks).
