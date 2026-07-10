use soroban_sdk::token;
use test_harness::{assert_contract_error, errors, usdc_preset, LendingTest, HARNESS_HUB};

fn create_asset_contract(t: &LendingTest) -> soroban_sdk::Address {
    t.env
        .register_stellar_asset_contract_v2(t.admin())
        .address()
        .clone()
}

#[test]
fn test_create_liquidity_pool_rejects_asset_id_mismatch() {
    let t = LendingTest::new().build();
    let ctrl = t.ctrl_client();

    let asset = create_asset_contract(&t);
    let wrong_asset = create_asset_contract(&t);
    let decimals = token::Client::new(&t.env, &asset).decimals();

    let params = usdc_preset()
        .params
        .to_market_params(&wrong_asset, decimals);

    // Approve the declared asset so the one-shot approval gate passes and the
    // controller's asset/params.asset_id equality check is what rejects.
    ctrl.approve_token(&asset);

    let result = match ctrl.try_create_liquidity_pool(&HARNESS_HUB, &asset, &params) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(err) => Err(err.expect("expected contract error")),
    };

    assert_contract_error(result, errors::GenericError::WrongToken as u32);
}

#[test]
#[ignore]
fn test_create_liquidity_pool_rejects_asset_decimals_mismatch() {
    let t = LendingTest::new().build();
    let ctrl = t.ctrl_client();

    let asset = create_asset_contract(&t);
    let decimals = token::Client::new(&t.env, &asset).decimals();
    let mismatched_decimals = decimals.saturating_add(1);

    let params = usdc_preset()
        .params
        .to_market_params(&asset, mismatched_decimals);

    let result = match ctrl.try_create_liquidity_pool(&HARNESS_HUB, &asset, &params) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(err) => Err(err.expect("expected contract error")),
    };

    assert_contract_error(result, errors::INVALID_ASSET);
}
