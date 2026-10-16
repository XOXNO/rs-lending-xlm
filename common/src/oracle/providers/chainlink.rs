//! Cross-contract client binding and call helper for Chainlink Data Feeds on
//! Stellar. One proxy serves all feeds, selected by a 32-byte data ID.

use soroban_sdk::{contractclient, contracttype, Address, BytesN, Env, Error, I256};

/// Number of decimal places requested from Chainlink and returned by the
/// currently published Stellar feeds.
pub const CHAINLINK_DECIMALS: u32 = 18;

/// A Chainlink price-feed round. Its timestamp is in Unix seconds.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ChainlinkRound {
    pub round_id: u64,
    pub answer: I256,
    pub timestamp: u64,
}

/// Read-only interface exposed by the Chainlink Data Feeds proxy on Stellar.
#[contractclient(name = "ChainlinkDataFeedsProxyClient")]
#[allow(dead_code)]
pub trait ChainlinkDataFeedsProxy {
    fn latest_round(env: Env, data_id: BytesN<32>, decimals: u32) -> Result<ChainlinkRound, Error>;
}

/// Reads the latest round for `data_id`. Returns `None` for a missing/frozen
/// feed, an unsupported precision, a contract error, or an invocation failure.
pub fn read_latest_round(
    env: &Env,
    contract: &Address,
    data_id: &BytesN<32>,
    decimals: u32,
) -> Option<ChainlinkRound> {
    match ChainlinkDataFeedsProxyClient::new(env, contract).try_latest_round(data_id, &decimals) {
        Ok(Ok(round)) => Some(round),
        _ => None,
    }
}

#[cfg(test)]
#[path = "../../../tests/oracle/providers/chainlink.rs"]
mod tests;
