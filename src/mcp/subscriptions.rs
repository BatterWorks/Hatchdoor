//! Modern-surface tool-list change subscriptions (#170).
//!
//! `subscriptions/listen` is served by rmcp's `ServerHandler::listen` seam;
//! this module owns the two pieces Hatchdoor adds around it: the validated
//! bearer-token marker our transport middleware attaches to every admitted
//! request (so a subscription can be attributed to a credential without
//! retaining tokens anywhere else), and the per-token live-subscription cap
//! enforced when a listen request is established.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The number of concurrent `subscriptions/listen` streams one bearer token may
/// hold; surfaced in the rejection message.
pub(crate) const MAX_SUBSCRIPTIONS_PER_TOKEN: usize = 4;

/// Non-serialized extension marker carrying the bearer token that was
/// constant-time-compared by the `/mcp` authorization middleware. Read by the
/// adapter's `listen` implementation to attribute a subscription to a token.
/// It never leaves the process and is not logged.
#[derive(Clone)]
pub struct McpBearerToken(pub Arc<str>);

/// Shared registry of live subscriptions per bearer token.
#[derive(Default)]
pub struct SubscriptionRegistry {
    counts: Mutex<HashMap<Arc<str>, usize>>,
}

impl SubscriptionRegistry {
    pub fn new() -> Self {
        Self::default()
    }
}

/// One acquired slot in [`SubscriptionRegistry`]; releasing on drop so every
/// exit path of `listen` (graceful end, cancellation, error) frees its place.
pub struct SubscriptionSlot {
    registry: Arc<SubscriptionRegistry>,
    token: Arc<str>,
}

impl SubscriptionRegistry {
    /// Acquire one live-subscription slot for `token`, or fail when that token
    /// already holds the maximum.
    pub fn try_acquire(self: &Arc<Self>, token: &Arc<str>) -> Option<SubscriptionSlot> {
        let mut counts = self.counts.lock().expect("subscription counts");
        let count = counts.entry(token.clone()).or_insert(0);
        if *count >= MAX_SUBSCRIPTIONS_PER_TOKEN {
            return None;
        }
        *count += 1;
        drop(counts);
        Some(SubscriptionSlot {
            registry: self.clone(),
            token: token.clone(),
        })
    }

    fn release(&self, token: &Arc<str>) {
        let mut counts = self.counts.lock().expect("subscription counts");
        if let Some(count) = counts.get_mut(token) {
            *count -= 1;
            if *count == 0 {
                counts.remove(token);
            }
        }
    }
}

impl Drop for SubscriptionSlot {
    fn drop(&mut self) {
        self.registry.release(&self.token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(value: &str) -> Arc<str> {
        Arc::from(value)
    }

    #[test]
    fn four_subscriptions_are_admitted_and_the_fifth_is_rejected() {
        let registry = Arc::new(SubscriptionRegistry::new());
        let t = token("tok");
        let mut slots = Vec::new();
        for _ in 0..MAX_SUBSCRIPTIONS_PER_TOKEN {
            slots.push(registry.try_acquire(&t).expect("within the cap"));
        }
        assert!(registry.try_acquire(&t).is_none(), "fifth is rejected");
        // Another token has its own independent budget.
        let other = token("other");
        assert!(registry.try_acquire(&other).is_some());

        drop(slots);
        assert!(
            registry.try_acquire(&t).is_some(),
            "released slots free budget"
        );
    }

    #[test]
    fn an_errored_subscription_releases_its_slot() {
        let registry = Arc::new(SubscriptionRegistry::new());
        let t = token("tok");
        drop(registry.try_acquire(&t));
        assert!(registry.try_acquire(&t).is_some());
    }
}
