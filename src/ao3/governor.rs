//! Write governor: one write at a time, a minimum spacing floor, per-action
//! budgets, and `Retry-After` parking.
//!
//! AO3 allows 300 requests / 300 s per IP and only 3 comments / 900 s; the client
//! stays far below that (`docs/ao3-write-api.md` §8). Duplicate public writes are
//! worse than failed ones, so writes are serialized here rather than retried.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::error::{Error, Result};

/// Action budgets as `(max actions, window)`.
fn budget(action: &str) -> (usize, Duration) {
    match action {
        // Comments are the tightest because AO3 itself allows 3 / 15 min.
        "comment" => (3, Duration::from_secs(900)),
        "kudos" => (10, Duration::from_secs(60)),
        "subscribe" | "unsubscribe" => (20, Duration::from_secs(3_600)),
        "bookmark" | "reading" => (10, Duration::from_secs(60)),
        // Publishing is rare and consequential; one accidental double-post is
        // worse than a slow retry, so keep posting well under AO3's limits.
        "post_work" => (3, Duration::from_secs(3_600)),
        "publish_draft" => (10, Duration::from_secs(3_600)),
        "delete_work" => (5, Duration::from_secs(3_600)),
        _ => (10, Duration::from_secs(60)),
    }
}

struct State {
    last: Option<Instant>,
    times: HashMap<&'static str, VecDeque<Instant>>,
    parked: HashMap<&'static str, Instant>,
}

/// Single-writer gate shared by every write route.
pub struct WriteGovernor {
    gate: Mutex<State>,
    min_interval: Duration,
}

impl WriteGovernor {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            gate: Mutex::new(State {
                last: None,
                times: HashMap::new(),
                parked: HashMap::new(),
            }),
            min_interval,
        }
    }

    /// Waits for the right to perform `action` (one at a time, budgeted).
    ///
    /// Errors are typed rate limits; the caller surfaces them without retrying.
    pub async fn begin(&self, action: &'static str) -> Result<()> {
        let mut state = self.gate.lock().await;
        let now = Instant::now();

        if let Some(until) = state.parked.get(action).copied()
            && now < until
        {
            return Err(Error::RateLimited {
                retry_after: Some(until.duration_since(now).as_secs().saturating_add(1)),
            });
        }

        let (limit, window) = budget(action);
        {
            let times = state.times.entry(action).or_default();
            while let Some(front) = times.front().copied() {
                if now.duration_since(front) > window {
                    times.pop_front();
                } else {
                    break;
                }
            }
            if times.len() >= limit {
                let oldest = times.front().copied().unwrap_or(now);
                let wait = window
                    .as_secs()
                    .saturating_sub(now.duration_since(oldest).as_secs())
                    .max(1);
                return Err(Error::RateLimited {
                    retry_after: Some(wait),
                });
            }
        }

        let wait = state
            .last
            .map(|last| self.min_interval.saturating_sub(now.duration_since(last)))
            .unwrap_or_default();
        if !wait.is_zero() {
            // Holding the gate across the sleep serializes writers: one request
            // in flight, exactly as ADR-0002 requires.
            tokio::time::sleep(wait).await;
        }
        let at = Instant::now();
        state.last = Some(at);
        if let Some(times) = state.times.get_mut(action) {
            times.push_back(at);
        }
        Ok(())
    }

    /// Records the upstream's `Retry-After` so the next attempt parks.
    pub async fn record_outcome(&self, action: &'static str, retry_after: Option<u64>) {
        if let Some(seconds) = retry_after {
            let mut state = self.gate.lock().await;
            state
                .parked
                .insert(action, Instant::now() + Duration::from_secs(seconds));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn one_write_at_a_time_with_a_spacing_floor() {
        let governor = WriteGovernor::new(Duration::from_millis(20));
        let started = Instant::now();
        governor.begin("subscribe").await.expect("first");
        governor.begin("subscribe").await.expect("second");
        assert!(
            started.elapsed() >= Duration::from_millis(20),
            "the floor must be enforced"
        );
    }

    #[tokio::test]
    async fn budgets_are_enforced_per_action() {
        let governor = WriteGovernor::new(Duration::ZERO);
        for _ in 0..3 {
            governor.begin("comment").await.expect("within budget");
        }
        let err = governor.begin("comment").await.expect_err("budget spent");
        assert!(matches!(
            err,
            Error::RateLimited {
                retry_after: Some(_)
            }
        ));
        // A different action has its own budget.
        governor.begin("kudos").await.expect("kudos still allowed");
    }

    #[tokio::test]
    async fn retry_after_parks_the_action() {
        let governor = WriteGovernor::new(Duration::ZERO);
        governor.record_outcome("subscribe", Some(60)).await;
        let err = governor.begin("subscribe").await.expect_err("parked");
        assert!(matches!(
            err,
            Error::RateLimited {
                retry_after: Some(secs)
            } if secs > 0
        ));
        governor
            .begin("unsubscribe")
            .await
            .expect("other action free");
    }
}
