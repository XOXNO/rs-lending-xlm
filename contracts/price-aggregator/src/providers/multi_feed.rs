//! Multi-feed adapter (RedStone + XOXNO): cache-first read, bulk helper for warm.
//! Always soft: bad market data → `None`.

#[cfg(not(feature = "certora"))]
use common::oracle::providers::redstone::RedStonePriceFeedClient;
use common::oracle::providers::redstone::{read_price_data_uncached, RedStonePriceData};
use common::types::MultiFeedRef;
use soroban_sdk::{Address, String};
#[cfg(not(feature = "certora"))]
use soroban_sdk::{Env, Vec};

use crate::observation::OracleObservation;
use crate::session::Session;

#[cfg(feature = "certora")]
pub(crate) use certora_read::read_multi_feed_source;

#[cfg(feature = "certora")]
mod certora_read {
    use super::*;
    cvlr_soroban_macros::apply_summary!(
        crate::spec::summaries::read_multi_feed_source_summary,
        pub(crate) fn read_multi_feed_source(
            session: &mut Session,
            feed: &MultiFeedRef,
            decimals: u32,
        ) -> Option<OracleObservation> {
            super::read_multi_feed_source_impl(session, feed, decimals)
        }
    );
}

#[cfg(not(feature = "certora"))]
pub(crate) use read_multi_feed_source_impl as read_multi_feed_source;

pub(crate) fn read_multi_feed_source_impl(
    session: &mut Session,
    feed: &MultiFeedRef,
    decimals: u32,
) -> Option<OracleObservation> {
    let env = session.env().clone();
    let now_secs = session.now_secs();
    let price_data = read_price_data(session, &feed.contract, &feed.feed_id)?;
    OracleObservation::from_multi_feed(&env, now_secs, &price_data, decimals)
}

fn read_price_data(
    session: &mut Session,
    contract: &Address,
    feed_id: &String,
) -> Option<RedStonePriceData> {
    if let Some(data) = session.get_feed(contract, feed_id) {
        return Some(data);
    }
    let env = session.env().clone();
    let data = read_price_data_uncached(&env, contract, feed_id)?;
    session.set_feed(contract, feed_id, data.clone());
    Some(data)
}

#[cfg(not(feature = "certora"))]
pub(crate) fn read_price_data_bulk(
    env: &Env,
    contract: &Address,
    feed_ids: &Vec<String>,
) -> Option<Vec<RedStonePriceData>> {
    match RedStonePriceFeedClient::new(env, contract).try_read_price_data(feed_ids) {
        Ok(Ok(data)) if data.len() == feed_ids.len() => Some(data),
        _ => None,
    }
}
