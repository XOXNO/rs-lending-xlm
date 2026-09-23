//! Groups the price-source integrations that the aggregator engine dispatches
//! to: Aquarius LP pools, RedStone-format multi-feed contracts (RedStone and
//! Xoxno), and Reflector.

pub(crate) mod aquarius;
pub(crate) mod multi_feed;
pub(crate) mod reflector;
