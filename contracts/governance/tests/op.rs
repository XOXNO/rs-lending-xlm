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

/// Pins the contract-side `AdminOperation` XDR encoding. The timelock hashes
/// these into the operation id, and sdk-js builders must encode the same
/// bytes (a mismatch would strand proposals). The identical base64 strings
/// are asserted in sdk-js `governance.test.ts`; if the enum/struct layout
/// changes here, both this test and the SDK gate must be updated together.
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
    assert_eq!(
        b64(
            &env,
            AdminOperation::UpdatePoolCaps(PoolCapsArgs {
                asset: addr.clone(),
                supply_cap: 100_000_000_000_000,
                borrow_cap: 50_000_000_000_000,
            })
        ),
        "AAAAEAAAAAEAAAACAAAADwAAAA5VcGRhdGVQb29sQ2FwcwAAAAAAEQAAAAEAAAADAAAADwAAAAVhc3NldAAAAAAAABIAAAABAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEAAAAPAAAACmJvcnJvd19jYXAAAAAAAAoAAAAAAAAAAAAALXmIPSAAAAAADwAAAApzdXBwbHlfY2FwAAAAAAAKAAAAAAAAAAAAAFrzEHpAAA=="
    );
}
