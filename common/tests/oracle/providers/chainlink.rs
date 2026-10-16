extern crate std;

use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contracterror, contractimpl, Address, BytesN, Env, I256};

#[contract]
struct StubChainlink;

#[contractimpl]
impl StubChainlink {
    pub fn latest_round(env: Env, _data_id: BytesN<32>, decimals: u32) -> ChainlinkRound {
        assert_eq!(decimals, CHAINLINK_DECIMALS);
        ChainlinkRound {
            round_id: 9,
            answer: I256::from_i128(&env, 1_000_000_000_000_000_000),
            timestamp: 123,
        }
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum StubChainlinkError {
    NoDataPresent = 50,
}

#[contract]
struct FailingChainlink;

#[contractimpl]
impl FailingChainlink {
    pub fn latest_round(
        _env: Env,
        _data_id: BytesN<32>,
        _decimals: u32,
    ) -> Result<ChainlinkRound, StubChainlinkError> {
        Err(StubChainlinkError::NoDataPresent)
    }
}

#[test]
fn missing_proxy_returns_none() {
    let env = Env::default();
    assert!(read_latest_round(
        &env,
        &Address::generate(&env),
        &BytesN::from_array(&env, &[1; 32]),
        CHAINLINK_DECIMALS,
    )
    .is_none());
}

#[test]
fn live_proxy_returns_round() {
    let env = Env::default();
    let proxy = env.register(StubChainlink, ());
    let round = read_latest_round(
        &env,
        &proxy,
        &BytesN::from_array(&env, &[2; 32]),
        CHAINLINK_DECIMALS,
    )
    .expect("round");
    assert_eq!(round.round_id, 9);
    assert_eq!(round.answer.to_i128(), Some(1_000_000_000_000_000_000));
    assert_eq!(round.timestamp, 123);
}

#[test]
fn proxy_error_returns_none() {
    let env = Env::default();
    let proxy = env.register(FailingChainlink, ());
    assert!(read_latest_round(
        &env,
        &proxy,
        &BytesN::from_array(&env, &[3; 32]),
        CHAINLINK_DECIMALS,
    )
    .is_none());
}
