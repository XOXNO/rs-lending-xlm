use controller::types::PositionMode;
use soroban_sdk::xdr::{ScErrorCode, ScErrorType};
use soroban_sdk::{contract, contractimpl, token, vec, Address, Env, Symbol};
use test_harness::{
    hub_asset, mock_swap_payload_xdr, reflector_primary_anchor_config, usd, LendingTest, ALICE,
    BOB, DEFAULT_TOLERANCE, HARNESS_SPOKE,
};

#[contract]
pub struct EvilToken;

#[contractimpl]
impl EvilToken {
    pub fn arm(env: Env, controller: Address, victim: Address, account_id: u64, usdc: Address) {
        let s = env.storage().instance();
        s.set(&Symbol::new(&env, "CTRL"), &controller);
        s.set(&Symbol::new(&env, "VICTIM"), &victim);
        s.set(&Symbol::new(&env, "ACC"), &account_id);
        s.set(&Symbol::new(&env, "USDC"), &usdc);
        s.set(&Symbol::new(&env, "FIRED"), &false);
    }

    pub fn decimals(_env: Env) -> u32 {
        7
    }

    // The price aggregator probes `decimals`/`symbol` before it will accept an
    // oracle for an address, so the hostile token has to answer both.
    pub fn symbol(env: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&env, "EVIL")
    }

    pub fn name(env: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&env, "EVIL")
    }

    pub fn balance(env: Env, _id: Address) -> i128 {
        let _ = env;
        1_000_000_000_000i128
    }

    pub fn transfer(env: Env, _from: Address, _to: Address, _amount: i128) {
        let s = env.storage().instance();
        let fired: bool = s.get(&Symbol::new(&env, "FIRED")).unwrap_or(true);
        if fired {
            return;
        }
        s.set(&Symbol::new(&env, "FIRED"), &true);

        let controller: Address = s.get(&Symbol::new(&env, "CTRL")).unwrap();
        let victim: Address = s.get(&Symbol::new(&env, "VICTIM")).unwrap();
        let account_id: u64 = s.get(&Symbol::new(&env, "ACC")).unwrap();
        let usdc: Address = s.get(&Symbol::new(&env, "USDC")).unwrap();

        let ctrl = controller::ControllerClient::new(&env, &controller);
        let _ = ctrl.get_health_factor(&account_id);
        let _ = (victim, usdc, vec![&env, 0i128]);
    }
}

#[test]
fn poc_multiply_initial_payment_token_cannot_reenter_the_controller() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(BOB, "USDC", 50_000.0);
    t.supply(BOB, "ETH", 50.0);

    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");

    t.fund_router("USDC", 40_000.0);

    t.resolve_market("USDC")
        .token_admin
        .mint(&alice, &10000_0000000i128);

    let open_swap = mock_swap_payload_xdr(&t.env, eth.clone(), usdc.clone(), 2000_0000000);
    let alice_id = t.ctrl_client().multiply(
        &alice,
        &0u64,
        &HARNESS_SPOKE,
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &PositionMode::Multiply,
        &open_swap,
        &Some((hub_asset(usdc.clone()), 5000_0000000i128)),
        &None,
    );

    let evil = t.env.register(EvilToken, ());
    EvilTokenClient::new(&t.env, &evil).arm(&t.controller, &alice, &alice_id, &usdc);

    // Price the hostile token. Without an oracle `process_multiply` dies in
    // `prefetch_strategy_prices` with #216 OracleNotConfigured and
    // `EvilToken::transfer` is never invoked, which is what made the original
    // assertion vacuous. A configured feed carries the flow all the way to
    // `transfer_amount_measured` (multiply.rs:169), the one token call on the
    // multiply path that runs outside `with_flash_guard`.
    t.mock_reflector_client().set_price(&evil, &usd(1));
    t.mock_reflector_client().set_twap_price(&evil, &usd(1));
    t.configure_market_oracle(
        &evil,
        &reflector_primary_anchor_config(
            &t.env,
            &t.mock_reflector,
            &evil,
            usd(1),
            DEFAULT_TOLERANCE.tolerance_bps,
        ),
    );

    let usdc_before = token::Client::new(&t.env, &usdc).balance(&alice);
    let recorded_before = t
        .ctrl_client()
        .get_collateral_amount(&alice_id, &hub_asset(usdc.clone()));

    let debt_swap = mock_swap_payload_xdr(&t.env, eth.clone(), usdc.clone(), 1000_0000000);
    let convert_swap = mock_swap_payload_xdr(&t.env, evil.clone(), usdc.clone(), 10_0000000);

    let res = t.ctrl_client().try_multiply(
        &alice,
        &alice_id,
        &HARNESS_SPOKE,
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &PositionMode::Multiply,
        &debt_swap,
        &Some((hub_asset(evil.clone()), 1i128)),
        &Some(convert_swap),
    );

    let usdc_after = token::Client::new(&t.env, &usdc).balance(&alice);
    let recorded_after = t
        .ctrl_client()
        .get_collateral_amount(&alice_id, &hub_asset(usdc.clone()));

    // What this PoC reaches: `transfer_amount_measured` (multiply.rs:169) really
    // invokes the caller-chosen payment token, outside `with_flash_guard`, and
    // that token really calls back into the controller. What stops it is not the
    // strategy guard but the Soroban host, which forbids re-entering a contract
    // already on the call stack. So the terminal fact is the host error, and it
    // must stay a *rejection*: pinning it here means a future host or SDK that
    // permitted re-entry would fail this test instead of silently opening the
    // path. What this PoC does NOT cover: a hostile token calling a *different*
    // contract that then calls the controller (no host re-entry, guard-only),
    // which `meta/reentrancy_matrix.rs` probes by injecting the flag directly.
    let host_error = res
        .expect_err("re-entry from the initial-payment token must abort multiply")
        .expect("expected a host error value, not a bare InvokeError");
    assert_eq!(
        host_error,
        soroban_sdk::Error::from_type_and_code(ScErrorType::Context, ScErrorCode::InvalidAction),
        "the host must refuse the nested controller call; got {host_error:?}"
    );

    assert_eq!(
        usdc_after, usdc_before,
        "a reverted multiply must move no USDC to the caller"
    );
    assert_eq!(
        recorded_after, recorded_before,
        "a reverted multiply must credit no collateral"
    );
}
