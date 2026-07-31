pub(crate) mod caps;
pub(crate) use caps::{SpokeUsageContext, UsageSide};

#[cfg(test)]
#[path = "../../tests/spoke.rs"]
mod tests;
