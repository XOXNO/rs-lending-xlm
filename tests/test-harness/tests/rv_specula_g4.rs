//! Second opinion on Specula governance MC-6: a canceller council large
//! enough that the owner's uncancellable canceller reset no longer fits one
//! transaction, leaving a colluding council able to veto every timelocked
//! operation forever.
//!
//! The test environment enforces neither `txMaxContractEventsSizeBytes` nor
//! the per-transaction ledger-entry footprint limits, so the on-chain failure
//! itself cannot be reproduced here: a 78-member council resets fine in this
//! host. What can be measured exactly is the XDR size of every event the
//! reset emits, which is the quantity stellar-core sums (together with the
//! return value) against `txMaxContractEventsSizeBytes`. The tests pin the
//! per-revocation cost, the fixed cost of the `OperationExecuted` event, and
//! derive the council size at which the reset exceeds 16,384 bytes (the limit
//! Specula assumed) and 8,198 bytes (the protocol-20 launch value).

use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::xdr::{
    AccountId, ContractEventType, Limits, PublicKey, ScAddress, Uint256, WriteXdr,
};
use soroban_sdk::{vec, Address, BytesN, Env, Symbol, TryFromVal, Vec};
use test_harness::LendingTest;

/// `governance::constants::TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS`.
const RECOVERY: u32 = 518_400;
/// The `txMaxContractEventsSizeBytes` value Specula assumed for mainnet.
const ASSUMED_EVENT_LIMIT: usize = 16_384;
/// The protocol-20 launch value of `txMaxContractEventsSizeBytes`.
const LAUNCH_EVENT_LIMIT: usize = 8_198;
/// XDR size of `ScVal::Void`, the reset's return value, which stellar-core
/// adds to the event bytes.
const VOID_RETURN_BYTES: usize = 4;

fn salt(t: &LendingTest, byte: u8) -> BytesN<32> {
    BytesN::<32>::from_array(&t.env, &[byte; 32])
}

fn advance(t: &LendingTest, n: u32) {
    t.env.ledger().with_mut(|l| l.sequence_number += n);
}

/// A G-account address (44 XDR bytes), the shape a human council member has
/// on mainnet. `Address::generate` yields contract addresses (40 bytes).
fn account(env: &Env, index: u32) -> Address {
    let mut raw = [0u8; 32];
    raw[..4].copy_from_slice(&index.to_be_bytes());
    raw[31] = 0xA5;
    let key = PublicKey::PublicKeyTypeEd25519(Uint256(raw));
    Address::try_from_val(env, &ScAddress::Account(AccountId(key))).unwrap()
}

fn council(env: &Env, size: u32) -> Vec<Address> {
    let mut members: Vec<Address> = vec![env];
    for i in 0..size {
        members.push_back(account(env, i + 1));
    }
    members
}

/// Count and summed XDR size of the non-diagnostic contract events of the
/// most recent top-level invocation (the test host keeps one invocation's
/// events at a time). This is the per-event encoding stellar-core sums
/// against `txMaxContractEventsSizeBytes`.
fn contract_event_bytes_of_last_call(env: &Env) -> (usize, usize) {
    let events = env.host().get_events().unwrap().0;
    let mut count = 0;
    let mut bytes = 0;
    for e in events.iter() {
        if e.failed_call || e.event.type_ != ContractEventType::Contract {
            continue;
        }
        count += 1;
        bytes += e.event.to_xdr(Limits::none()).unwrap().len();
    }
    (count, bytes)
}

/// Seats `members` through a recovery reset (the owner's own path), then
/// executes a second reset to the empty list and returns `(events, bytes)`
/// for that second execution only.
fn measure_empty_reset(t: &LendingTest, members: &Vec<Address>, s: u8) -> (usize, usize) {
    let gov = t.gov_iface_client();
    let canceller = Symbol::new(&t.env, "CANCELLER");

    gov.propose_canceller_reset(members, &salt(t, s));
    advance(t, RECOVERY);
    gov.execute_canceller_reset(&None, members, &salt(t, s));
    for m in members.iter() {
        assert!(gov.has_role(&m, &canceller));
    }

    let empty: Vec<Address> = vec![&t.env];
    gov.propose_canceller_reset(&empty, &salt(t, s + 1));
    advance(t, RECOVERY);
    gov.execute_canceller_reset(&None, &empty, &salt(t, s + 1));
    let measured = contract_event_bytes_of_last_call(&t.env);
    for m in members.iter() {
        assert!(!gov.has_role(&m, &canceller));
    }
    assert!(gov.has_role(&t.admin(), &canceller), "owner keeps its seat");
    measured
}

/// MC-6 arithmetic at HEAD. Resetting away N non-owner cancellers emits one
/// `RoleRevoked` per member (OZ `revoke_role_no_auth`, map data format,
/// `[role, account]` topics, `{caller}` data) plus one `OperationExecuted`.
/// The per-member cost is 204 bytes with the harness's contract-address owner
/// (208 with a G-account owner), the fixed cost is the executed event, and the
/// council size at which the reset crosses 16,384 bytes is 78 or 79 depending
/// only on the owner's address kind. At 8,198 bytes it is 38 or 39.
#[test]
fn mc6_reset_event_bytes_grow_linearly_with_council_size() {
    let one = {
        let t = LendingTest::new().build();
        measure_empty_reset(&t, &council(&t.env, 1), 1)
    };
    let two = {
        let t = LendingTest::new().build();
        measure_empty_reset(&t, &council(&t.env, 2), 1)
    };
    let five = {
        let t = LendingTest::new().build();
        measure_empty_reset(&t, &council(&t.env, 5), 1)
    };
    assert_eq!(one.0, 2, "one RoleRevoked plus OperationExecuted");
    assert_eq!(two.0, 3);
    assert_eq!(five.0, 6);

    let per_member = two.1 - one.1;
    let fixed = one.1 - per_member;
    assert_eq!(
        five.1,
        fixed + 5 * per_member,
        "event bytes are affine in the council size"
    );
    // ContractEvent: ext 4 + contract_id 36 + type 4 + body tag 4 + topics
    // (4 + "role_revoked" 20 + "CANCELLER" 20 + account address 44) + map
    // data (12 + "caller" key 16 + contract-address owner 40).
    assert_eq!(per_member, 204, "RoleRevoked XDR bytes per revoked member");
    // OperationExecuted with empty args: measured, not hand-derived.
    assert!(
        (300..=420).contains(&fixed),
        "OperationExecuted bytes {fixed}"
    );

    // Threshold for a contract-address owner (this harness) and for a
    // G-account owner (+4 bytes on every RoleRevoked `caller`).
    let first_failing = |limit: usize, per: usize| (limit - fixed - VOID_RETURN_BYTES) / per + 1;
    let at_16k_contract_owner = first_failing(ASSUMED_EVENT_LIMIT, per_member);
    let at_16k_account_owner = first_failing(ASSUMED_EVENT_LIMIT, per_member + 4);
    let at_8k_contract_owner = first_failing(LAUNCH_EVENT_LIMIT, per_member);
    let at_8k_account_owner = first_failing(LAUNCH_EVENT_LIMIT, per_member + 4);
    assert_eq!(
        at_16k_account_owner, 78,
        "Specula's figure, G-account owner"
    );
    assert_eq!(at_16k_contract_owner, 79);
    assert_eq!(at_8k_account_owner, 38);
    assert_eq!(at_8k_contract_owner, 39);
}

/// The test host enforces no event-size limit: a 79-member council, the
/// first size over 16,384 bytes with this harness's contract-address owner
/// (78 with a G-account owner, Specula's figure), resets to empty here with
/// 79 revocations. The on-chain failure is therefore a network-limit fact,
/// not something this harness can witness; it can only show the bytes are
/// over the line. The same holds at 39 members against the 8,198-byte launch
/// value of the limit.
#[test]
fn mc6_oversized_reset_exceeds_assumed_limit_but_executes_in_host() {
    let t = LendingTest::new().build();
    let (count, bytes) = measure_empty_reset(&t, &council(&t.env, 78), 1);
    assert_eq!(count, 79, "78 RoleRevoked plus OperationExecuted");
    assert!(
        bytes + VOID_RETURN_BYTES <= ASSUMED_EVENT_LIMIT,
        "78-member reset with a contract-address owner is {bytes} event bytes"
    );

    let t = LendingTest::new().build();
    let (count, bytes) = measure_empty_reset(&t, &council(&t.env, 79), 1);
    assert_eq!(count, 80, "79 RoleRevoked plus OperationExecuted");
    assert!(
        bytes + VOID_RETURN_BYTES > ASSUMED_EVENT_LIMIT,
        "79-member reset is {bytes} event bytes, limit {ASSUMED_EVENT_LIMIT}"
    );

    let t = LendingTest::new().build();
    let (count, bytes) = measure_empty_reset(&t, &council(&t.env, 39), 1);
    assert_eq!(count, 40);
    assert!(
        bytes + VOID_RETURN_BYTES > LAUNCH_EVENT_LIMIT,
        "39-member reset is {bytes} event bytes, launch limit {LAUNCH_EVENT_LIMIT}"
    );
}

/// Seating a council through the reset costs a `RoleGranted` per member and
/// the executed event carries the whole `new_cancellers` vector in its
/// `args`, so a reset that *installs* N members is heavier than one that
/// removes N. A single oversized grant therefore cannot be the way a large
/// council came to exist on chain: it must be built member by member through
/// `GrantGovRole`, each an owner-only timelocked operation.
#[test]
fn mc6_installing_reset_is_heavier_than_removing_reset() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let members = council(&t.env, 10);

    gov.propose_canceller_reset(&members, &salt(&t, 1));
    advance(&t, RECOVERY);
    gov.execute_canceller_reset(&None, &members, &salt(&t, 1));
    let (install_count, install_bytes) = contract_event_bytes_of_last_call(&t.env);
    assert_eq!(install_count, 11, "10 RoleGranted plus OperationExecuted");

    let empty: Vec<Address> = vec![&t.env];
    gov.propose_canceller_reset(&empty, &salt(&t, 2));
    advance(&t, RECOVERY);
    gov.execute_canceller_reset(&None, &empty, &salt(&t, 2));
    let (remove_count, remove_bytes) = contract_event_bytes_of_last_call(&t.env);
    assert_eq!(remove_count, 11, "10 RoleRevoked plus OperationExecuted");

    // 10 account addresses in `args`: 44 bytes each.
    assert_eq!(install_bytes - remove_bytes, 10 * 44);
}
