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
        b64(&env, AdminOperation::UpdateGovDelay(34560)),
        "AAAAEAAAAAEAAAACAAAADwAAAA5VcGRhdGVHb3ZEZWxheQAAAAAAAwAAhwA="
    );
    assert_eq!(
        b64(&env, AdminOperation::SetAggregator(addr.clone())),
        "AAAAEAAAAAEAAAACAAAADwAAAA1TZXRBZ2dyZWdhdG9yAAAAAAAAEgAAAAEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAQ=="
    );
}
