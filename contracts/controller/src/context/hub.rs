use crate::config;
use crate::context::Cache;

impl Cache {
    /// Memoized [`config::require_hub_active`]: the first check per hub id reads
    /// persistent storage (and renews its TTL); repeats within the same
    /// invocation short-circuit. Only hubs proven active are cached — an
    /// inactive or missing hub panics before insertion, so the memo can never
    /// hold a stale verdict.
    pub(crate) fn require_hub_active(&mut self, hub_id: u32) {
        if self.verified_hubs.contains_key(hub_id) {
            return;
        }
        config::require_hub_active(&self.env, hub_id);
        self.verified_hubs.set(hub_id, true);
    }
}
