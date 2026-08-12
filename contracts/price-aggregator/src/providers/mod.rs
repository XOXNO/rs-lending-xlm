//! Groups the price-source integrations that the aggregator engine dispatches
//! to: Aquarius LP pools, the multi-feed aggregation source, RedStone,
//! Reflector, and XOXNO.

pub(crate) mod aquarius;
pub(crate) mod multi_feed;
pub(crate) mod redstone;
pub(crate) mod reflector;
pub(crate) mod xoxno;
