//! Layered resource protection on `/mcp` (#171).
//!
//! The decision package's layers, in the order the transport middleware applies
//! them:
//!
//! 1. **Tool quota** — at most [`TOOL_CALLS_PER_MINUTE`] tool calls per minute
//!    per bearer token. Protocol handling (`initialize`, `ping`,
//!    notifications), discovery (`server/discover`), list handling
//!    (`tools/list`), and `subscriptions/listen` streams stay outside the
//!    quota: a client that is only reading metadata can never lock itself out.
//! 2. **Concurrency caps** — at most [`MAX_CONCURRENT_ORDINARY_CALLS`] tool
//!    calls execute at once process-wide, of which at most
//!    [`MAX_CONCURRENT_EXPENSIVE_SEARCHES`] may be expensive searches
//!    (`search_notes`). A search holds both a search slot and an ordinary slot,
//!    so searches can never starve ordinary calls entirely.
//! 3. **Untouched locks** — the existing single-operation write/reindex locks
//!    are deliberately not part of this module; layered limiting sits in front
//!    of them without replacing them.
//!
//! Over-limit requests are rejected with HTTP 429 and a `Retry-After` header
//! rather than a JSON-RPC error: they never reach dispatch, so there is no
//! request id owed an answer. Every layer can be switched off by configuration
//! (`HATCHDOOR_MCP_RATE_LIMITS_ENABLED`, see `McpConfig`) for deployments that
//! sit behind their own gateway.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Semaphore;

use super::subscriptions::McpBearerToken;

/// Tool calls one bearer token may make per rolling minute.
pub(crate) const TOOL_CALLS_PER_MINUTE: usize = 120;

/// The quota window, matching "per minute".
pub(crate) const QUOTA_WINDOW: Duration = Duration::from_secs(60);

/// Tool calls executing concurrently, process-wide, across all tokens.
pub(crate) const MAX_CONCURRENT_ORDINARY_CALLS: usize = 8;

/// Expensive `search_notes` calls executing concurrently, process-wide. Held in
/// addition to an ordinary slot.
pub(crate) const MAX_CONCURRENT_EXPENSIVE_SEARCHES: usize = 2;

/// Retry-After advertised when all concurrency slots are busy; clients should
/// retry shortly rather than back off for a whole window.
pub(crate) const CONCURRENCY_RETRY_AFTER: Duration = Duration::from_secs(1);

/// How the transport middleware classified one POST body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestClass {
    /// Everything outside the tool quota: protocol lifecycle, discovery,
    /// list handling, notifications.
    Exempt,
    /// An ordinary `tools/call`.
    ToolCall,
    /// A `tools/call` for `search_notes`, the known expensive search.
    ExpensiveSearch,
}

/// Classify one JSON-RPC method name (+ tool name for calls). Kept total so a
/// malformed or unknown body still lands somewhere safe: anything unrecognized
/// is treated as an ordinary tool call candidate only when it names
/// `tools/call`; everything else is exempt.
pub(crate) fn classify(method: Option<&str>, tool_name: Option<&str>) -> RequestClass {
    match method {
        Some("tools/call") => match tool_name {
            Some("search_notes") => RequestClass::ExpensiveSearch,
            _ => RequestClass::ToolCall,
        },
        _ => RequestClass::Exempt,
    }
}

/// The `Retry-After` value for a rejection duration, in whole seconds (at
/// least 1).
pub(crate) fn retry_after_seconds(retry_in: Duration) -> u64 {
    retry_in.as_secs().max(1)
}

/// Shared state behind one `/mcp` transport instance: the per-token rolling
/// window plus the two process-wide concurrency pools. Cheap to construct per
/// test; exactly one instance per transport in production.
pub(crate) struct RateLimiter {
    quota: Mutex<HashMap<Arc<str>, VecDeque<std::time::Instant>>>,
    ordinary: Arc<Semaphore>,
    expensive_searches: Arc<Semaphore>,
}

/// Held for the duration of one admitted tool call; releasing on drop frees
/// both its concurrency slots on every exit path.
#[derive(Debug)]
pub(crate) struct ConcurrencyGuard {
    _ordinary: tokio::sync::OwnedSemaphorePermit,
    _expensive: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            quota: Mutex::new(HashMap::new()),
            ordinary: Arc::new(Semaphore::new(MAX_CONCURRENT_ORDINARY_CALLS)),
            expensive_searches: Arc::new(Semaphore::new(MAX_CONCURRENT_EXPENSIVE_SEARCHES)),
        }
    }

    /// Record one tool call against the token's rolling-minute window, or
    /// report how long until the oldest recorded call leaves the window.
    pub fn check_quota(
        &self,
        token: &McpBearerToken,
        now: std::time::Instant,
    ) -> Result<(), Duration> {
        let mut windows = self.quota.lock().expect("quota windows");
        let window = windows.entry(token.0.clone()).or_default();
        while let Some(&oldest) = window.front() {
            if now.duration_since(oldest) < QUOTA_WINDOW {
                break;
            }
            window.pop_front();
        }
        if window.len() >= TOOL_CALLS_PER_MINUTE {
            let oldest = *window.front().expect("full window is non-empty");
            return Err(QUOTA_WINDOW.saturating_sub(now.duration_since(oldest)));
        }
        window.push_back(now);
        Ok(())
    }

    /// Try to take the concurrency slots one classified call needs, without
    /// waiting. Returns the guard to hold across dispatch, or how long the
    /// client should wait before retrying.
    pub async fn try_acquire(&self, class: RequestClass) -> Result<ConcurrencyGuard, Duration> {
        let ordinary = match Arc::clone(&self.ordinary).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => return Err(CONCURRENCY_RETRY_AFTER),
        };
        let expensive = if class == RequestClass::ExpensiveSearch {
            // Never waits: on failure the ordinary permit above is dropped by
            // this early return and both slots stay consistent.
            match Arc::clone(&self.expensive_searches).try_acquire_owned() {
                Ok(permit) => Some(permit),
                Err(_) => return Err(CONCURRENCY_RETRY_AFTER),
            }
        } else {
            None
        };
        Ok(ConcurrencyGuard {
            _ordinary: ordinary,
            _expensive: expensive,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(value: &str) -> McpBearerToken {
        McpBearerToken(Arc::from(value))
    }

    #[test]
    fn classification_exempts_everything_but_tools_call() {
        assert_eq!(classify(Some("initialize"), None), RequestClass::Exempt);
        assert_eq!(classify(Some("tools/list"), None), RequestClass::Exempt);
        assert_eq!(
            classify(Some("server/discover"), None),
            RequestClass::Exempt
        );
        assert_eq!(classify(Some("ping"), None), RequestClass::Exempt);
        assert_eq!(classify(None, None), RequestClass::Exempt);
        assert_eq!(
            classify(Some("notifications/initialized"), None),
            RequestClass::Exempt
        );
        assert_eq!(
            classify(Some("tools/call"), Some("get_note")),
            RequestClass::ToolCall
        );
        assert_eq!(
            classify(Some("tools/call"), Some("create_note")),
            RequestClass::ToolCall
        );
        assert_eq!(
            classify(Some("tools/call"), Some("search_notes")),
            RequestClass::ExpensiveSearch
        );
    }

    #[test]
    fn quota_admits_the_limit_and_rejects_the_next_call_with_retry_after() {
        let limiter = RateLimiter::new();
        let t = token("tok");
        let start = std::time::Instant::now();
        for i in 0..TOOL_CALLS_PER_MINUTE {
            assert!(
                limiter
                    .check_quota(&t, start + Duration::from_millis(i as u64))
                    .is_ok(),
                "call {i} within the limit is admitted"
            );
        }
        let rejection = limiter
            .check_quota(
                &t,
                start + Duration::from_millis(TOOL_CALLS_PER_MINUTE as u64),
            )
            .expect_err("the call past the limit is rejected");
        assert!(rejection <= QUOTA_WINDOW && rejection > Duration::ZERO);
        assert_eq!(rejection.as_secs().max(1), rejection.as_secs());
    }

    #[test]
    fn quota_windows_are_per_token_and_expire_after_a_minute() {
        let limiter = RateLimiter::new();
        let a = token("a");
        let b = token("b");
        let start = std::time::Instant::now();
        for i in 0..TOOL_CALLS_PER_MINUTE {
            limiter
                .check_quota(&a, start + Duration::from_millis(i as u64))
                .expect("within the limit");
        }
        assert!(
            limiter
                .check_quota(&a, start + Duration::from_secs(1))
                .is_err()
        );
        assert!(
            limiter
                .check_quota(&b, start + Duration::from_secs(1))
                .is_ok(),
            "another token has its own budget"
        );
        assert!(
            limiter
                .check_quota(
                    &a,
                    start + QUOTA_WINDOW + Duration::from_millis(TOOL_CALLS_PER_MINUTE as u64)
                )
                .is_ok(),
            "the window rolls and budget returns"
        );
    }

    #[tokio::test]
    async fn concurrency_caps_ordinary_calls_and_expensive_searches_independently() {
        let limiter = Arc::new(RateLimiter::new());

        let mut guards = Vec::new();
        for _ in 0..MAX_CONCURRENT_ORDINARY_CALLS {
            guards.push(
                limiter
                    .try_acquire(RequestClass::ToolCall)
                    .await
                    .expect("within the cap"),
            );
        }
        assert_eq!(guards.len(), MAX_CONCURRENT_ORDINARY_CALLS);
        assert!(
            limiter.try_acquire(RequestClass::ToolCall).await.is_err(),
            "the ninth ordinary call is rejected"
        );
        drop(guards);
        assert!(
            limiter.try_acquire(RequestClass::ToolCall).await.is_ok(),
            "released slots admit again"
        );

        let mut searches = Vec::new();
        for _ in 0..MAX_CONCURRENT_EXPENSIVE_SEARCHES {
            searches.push(
                limiter
                    .try_acquire(RequestClass::ExpensiveSearch)
                    .await
                    .expect("within the cap"),
            );
        }
        assert!(
            limiter
                .try_acquire(RequestClass::ExpensiveSearch)
                .await
                .is_err(),
            "the third concurrent search is rejected"
        );
        assert!(
            limiter.try_acquire(RequestClass::ToolCall).await.is_ok(),
            "busy searches do not block ordinary calls"
        );
        drop(searches);
        assert!(
            limiter
                .try_acquire(RequestClass::ExpensiveSearch)
                .await
                .is_ok(),
            "released search slots admit again"
        );
    }

    #[test]
    fn retry_after_values_are_whole_seconds_at_least_one() {
        assert_eq!(retry_after_seconds(Duration::from_millis(250)), 1);
        assert_eq!(retry_after_seconds(Duration::from_secs(37)), 37);
        assert_eq!(retry_after_seconds(Duration::ZERO), 1);
    }
}
