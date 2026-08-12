//! Per-invocation memoization of hub-active checks on `Cache`.

use crate::config;
use crate::context::Cache;

impl Cache {
    /// Verifies that `hub_id` is active, memoizing the result for the
    /// current invocation. Returns immediately if `hub_id` was already
    /// verified; otherwise delegates to [`config::require_hub_active`],
    /// which reads persistent storage and panics if the hub is inactive or
    /// missing, then records `hub_id` as verified. Only active hubs are ever
    /// recorded, so the memo cannot hold a stale verdict.
    pub(crate) fn require_hub_active(&mut self, hub_id: u32) {
        if self.verified_hubs.contains_key(hub_id) {
            return;
        }
        config::require_hub_active(&self.env, hub_id);
        self.verified_hubs.set(hub_id, true);
    }
}
