//! Helpers used by more than one module of the controller suite.
//!
//! Every module here compiles into the same test binary, so a helper needed in
//! two places belongs in this file rather than copied into both.

use common::math::fp::Bps;
use flash_loan_receiver::{FlashLoanMode, FlashLoanRequest};
use soroban_sdk::testutils::{ContractEvents, MockAuth, MockAuthInvoke};
use soroban_sdk::xdr::{ContractEventBody, ScVal, ToXdr, VecM};
use soroban_sdk::{Address, Bytes, IntoVal, Val, Vec};
use test_harness::{hub_asset, HubAssetKey, LendingTest};

pub fn raw_units(t: &LendingTest, asset_name: &str, units: i128) -> i128 {
    units * 10i128.pow(t.resolve_market(asset_name).decimals)
}

pub fn flash_fee(t: &LendingTest, asset_name: &str, amount: i128) -> i128 {
    let config = t.get_asset_config(asset_name);
    Bps::from(config.flashloan_fee).flash_loan_fee_on(&t.env, amount)
}

pub fn flash_guard_cleared(t: &LendingTest) -> bool {
    t.env.as_contract(&t.controller, || {
        !controller::test_support::is_flash_loan_ongoing(&t.env)
    })
}

pub fn receiver_data(t: &LendingTest, mode: FlashLoanMode) -> Bytes {
    FlashLoanRequest { mode }.to_xdr(&t.env)
}

/// `flash_loan` under an auth tree that covers exactly the top-level call, so a
/// receiver cannot borrow the caller's authorization for a nested invocation.
pub fn strict_flash_loan(
    t: &LendingTest,
    caller: &Address,
    asset: &HubAssetKey,
    amount: i128,
    receiver: &Address,
    data: &Bytes,
) -> Result<(), std::string::String> {
    let args: Vec<Val> = (
        caller.clone(),
        asset.clone(),
        amount,
        receiver.clone(),
        data.clone(),
    )
        .into_val(&t.env);
    let invoke = MockAuthInvoke {
        contract: &t.controller,
        fn_name: "flash_loan",
        args,
        sub_invokes: &[],
    };
    let auths = [MockAuth {
        address: caller,
        invoke: &invoke,
    }];

    match t
        .ctrl_client()
        .mock_auths(&auths)
        .try_flash_loan(caller, asset, &amount, receiver, data)
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(conv)) => Err(std::format!("conversion error: {conv:?}")),
        Err(Ok(contract_err)) => Err(std::format!("{contract_err:?}")),
        Err(Err(invoke)) => Err(std::format!("invoke error: {invoke:?}")),
    }
}

pub fn get_indexes(t: &LendingTest, asset: &str) -> (i128, i128) {
    let asset_addr = t.resolve_asset(asset);
    let ctrl = t.ctrl_client();
    let assets = soroban_sdk::Vec::from_array(&t.env, [hub_asset(asset_addr)]);
    let idx = ctrl.get_market_indexes_detailed(&assets).get(0).unwrap();
    (idx.supply_index, idx.borrow_index)
}

fn topic_pair_matches(body: &ContractEventBody, first: &str, second: &str) -> bool {
    let ContractEventBody::V0(body) = body;
    matches!(
        (body.topics.first(), body.topics.get(1)),
        (Some(ScVal::Symbol(a)), Some(ScVal::Symbol(b)))
            if a.0.to_string() == first && b.0.to_string() == second
    )
}

pub fn data_for_topic(events: &ContractEvents, first: &str, second: &str) -> std::vec::Vec<ScVal> {
    events
        .events()
        .iter()
        .filter_map(|event| {
            if !topic_pair_matches(&event.body, first, second) {
                return None;
            }
            let ContractEventBody::V0(body) = &event.body;
            Some(body.data.clone())
        })
        .collect()
}

pub fn as_vec(v: &ScVal) -> &VecM<ScVal> {
    match v {
        ScVal::Vec(Some(entries)) => &entries.0,
        other => panic!("expected ScVal::Vec, got {:?}", other),
    }
}

pub fn count_topic(events: &ContractEvents, first: &str, second: &str) -> usize {
    events
        .events()
        .iter()
        .filter(|event| topic_pair_matches(&event.body, first, second))
        .count()
}
