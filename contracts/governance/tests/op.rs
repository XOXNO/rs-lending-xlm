extern crate std;
use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::{Limits, ScVal, WriteXdr};
use soroban_sdk::{IntoVal, TryFromVal, Val};

fn b64(env: &Env, op: AdminOperation) -> std::string::String {
    let val: Val = op.into_val(env);
    let sc = ScVal::try_from_val(env, &val).unwrap();
    sc.to_xdr_base64(Limits::none()).unwrap()
}

/// All-zero contract address strkey (`Address::generate` is deterministic
/// from a fresh `Env`). Mirrors the `PARITY_ADDR` constant in the sdk-js
/// `governance.test.ts` byte-parity suite.
const PARITY_ADDR: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM";

/// Pins `AdminOperation` XDR; sdk-js builders must match these bytes.
#[test]
fn admin_op_xdr_is_byte_stable() {
    let env = Env::default();
    let addr = Address::generate(&env);
    assert_eq!(
        addr.to_string(),
        soroban_sdk::String::from_str(&env, PARITY_ADDR)
    );

    assert_eq!(
        b64(&env, AdminOperation::DeployPool),
        "AAAAEAAAAAEAAAABAAAADwAAAApEZXBsb3lQb29sAAA="
    );
    assert_eq!(
        b64(&env, AdminOperation::Unpause),
        "AAAAEAAAAAEAAAABAAAADwAAAAdVbnBhdXNlAA=="
    );
    assert_eq!(
        b64(&env, AdminOperation::UpdateGovDelay(34560)),
        "AAAAEAAAAAEAAAACAAAADwAAAA5VcGRhdGVHb3ZEZWxheQAAAAAAAwAAhwA="
    );
    assert_eq!(
        b64(&env, AdminOperation::SetSwapAggregator(addr.clone())),
        "AAAAEAAAAAEAAAACAAAADwAAABFTZXRTd2FwQWdncmVnYXRvcgAAAAAAABIAAAABAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE="
    );
    assert_eq!(
        b64(
            &env,
            AdminOperation::SetSpokeLiquidationCurve(SpokeLiquidationCurveArgs {
                spoke_id: 1,
                target_hf_wad: 1_020_000_000_000_000_000,
                hf_for_max_bonus_wad: 510_000_000_000_000_000,
                liquidation_bonus_factor_bps: 10_000,
            })
        ),
        "AAAAEAAAAAEAAAACAAAADwAAABhTZXRTcG9rZUxpcXVpZGF0aW9uQ3VydmUAAAARAAAAAQAAAAQAAAAPAAAAFGhmX2Zvcl9tYXhfYm9udXNfd2FkAAAACgAAAAAAAAAABxPiTENzAAAAAAAPAAAAHGxpcXVpZGF0aW9uX2JvbnVzX2ZhY3Rvcl9icHMAAAADAAAnEAAAAA8AAAAIc3Bva2VfaWQAAAADAAAAAQAAAA8AAAANdGFyZ2V0X2hmX3dhZAAAAAAAAAoAAAAAAAAAAA4nxJiG5gAA"
    );
}
