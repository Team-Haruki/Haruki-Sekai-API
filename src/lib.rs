pub mod api;
pub mod client;
pub mod config;
pub mod crypto;
pub mod db;
pub mod error;
pub mod ingest_engine;
pub mod models;
pub mod updater;
pub mod utils;

use crate::client::SekaiClient;
use crate::config::Config;
use sea_orm::DatabaseConnection;
use std::collections::HashMap;
use std::sync::Arc;

/// A shared in-flight upstream call: holds the (status, serialized-json) result
/// once the leader resolves it; followers await and clone the Arc<str> cheaply.
pub type CoalescedCell = Arc<tokio::sync::OnceCell<(u16, Arc<str>)>>;

/// In-process single-flight for read-endpoint responses. Concurrent requests for
/// the same cache key share one in-flight upstream call (and its result) instead
/// of each hitting the game server, capping per-key upstream/account usage at ~1
/// per cache window regardless of concurrency.
#[derive(Default)]
pub struct RequestCoalescer {
    pub inflight: parking_lot::Mutex<HashMap<String, CoalescedCell>>,
}

struct InflightGuard<'a> {
    coalescer: &'a RequestCoalescer,
    key: &'a str,
    cell: CoalescedCell,
}

impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        let mut inflight = self.coalescer.inflight.lock();
        let should_remove = inflight
            .get(self.key)
            .is_some_and(|cell| Arc::ptr_eq(cell, &self.cell));
        if should_remove {
            inflight.remove(self.key);
        }
    }
}

impl RequestCoalescer {
    /// Run `fetch` under single-flight for `key`: concurrent callers with the same
    /// key share one in-flight execution and clone its result. Returns
    /// `(result, was_leader)` — the leader is the caller that actually ran `fetch`
    /// (and is responsible for any post-fetch work such as caching).
    pub async fn coalesce<F, Fut>(
        &self,
        key: &str,
        fetch: F,
    ) -> (Result<(u16, Arc<str>), crate::error::AppError>, bool)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(u16, Arc<str>), crate::error::AppError>>,
    {
        let (cell, is_leader) = {
            let mut inflight = self.inflight.lock();
            match inflight.get(key) {
                Some(cell) => (cell.clone(), false),
                None => {
                    let cell: CoalescedCell = Arc::new(tokio::sync::OnceCell::new());
                    inflight.insert(key.to_string(), cell.clone());
                    (cell, true)
                }
            }
        };
        // Keep cleanup tied to the leader future's lifetime. If that request is
        // cancelled while fetching, Drop still removes the abandoned slot so a
        // follower cannot populate an OnceCell that remains cached forever.
        let _guard = is_leader.then(|| InflightGuard {
            coalescer: self,
            key,
            cell: cell.clone(),
        });
        let outcome = cell.get_or_try_init(fetch).await.cloned();
        (outcome, is_leader)
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub clients: HashMap<crate::config::ServerRegion, Arc<SekaiClient>>,
    pub db: Option<DatabaseConnection>,
    pub master_db: Option<DatabaseConnection>,
    pub redis: Option<redis::aio::ConnectionManager>,
    pub jwt_secret: Option<String>,
    pub coalescer: Arc<RequestCoalescer>,
}

#[cfg(test)]
mod coalescer_tests {
    use super::RequestCoalescer;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    // A burst of concurrent callers for the same key must run `fetch` exactly once
    // (single-flight), all receive the shared result, exactly one is the leader,
    // and the in-flight slot is cleaned up afterwards.
    #[tokio::test]
    async fn coalesces_concurrent_callers_to_one_fetch() {
        let coalescer = Arc::new(RequestCoalescer::default());
        let calls = Arc::new(AtomicU32::new(0));
        let mut handles = Vec::new();
        for _ in 0..32 {
            let c = coalescer.clone();
            let calls = calls.clone();
            handles.push(tokio::spawn(async move {
                c.coalesce("k", || async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    // Hold the in-flight window open so the whole burst attaches.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Ok((200u16, Arc::from("{\"ok\":true}")))
                })
                .await
            }));
        }
        let results = futures::future::join_all(handles).await;
        let mut leaders = 0;
        for r in results {
            let (outcome, is_leader) = r.expect("task panicked");
            let (status, json) = outcome.expect("fetch failed");
            assert_eq!(status, 200);
            assert_eq!(&*json, "{\"ok\":true}");
            if is_leader {
                leaders += 1;
            }
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1, "fetch must run once");
        assert_eq!(leaders, 1, "exactly one leader");
        assert!(
            coalescer.inflight.lock().is_empty(),
            "in-flight slot cleaned up"
        );
    }

    // A second call after the first window completes runs `fetch` again (the slot
    // is not a permanent cache — TTL caching is the Redis layer's job).
    #[tokio::test]
    async fn re_fetches_after_window_completes() {
        let coalescer = RequestCoalescer::default();
        let calls = Arc::new(AtomicU32::new(0));
        for _ in 0..3 {
            let calls = calls.clone();
            let (outcome, leader) = coalescer
                .coalesce("k", || async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok((200u16, Arc::from("x")))
                })
                .await;
            assert!(outcome.is_ok());
            assert!(leader);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn cancelled_leader_does_not_leave_a_permanent_result() {
        let coalescer = Arc::new(RequestCoalescer::default());
        let fetch_started = Arc::new(tokio::sync::Notify::new());
        let release_fetch = Arc::new(tokio::sync::Notify::new());

        let leader = {
            let coalescer = coalescer.clone();
            let fetch_started = fetch_started.clone();
            let release_fetch = release_fetch.clone();
            tokio::spawn(async move {
                coalescer
                    .coalesce("k", || async move {
                        fetch_started.notify_one();
                        release_fetch.notified().await;
                        Ok((200, Arc::from("stale")))
                    })
                    .await
            })
        };

        fetch_started.notified().await;
        leader.abort();
        assert!(leader.await.unwrap_err().is_cancelled());
        assert!(coalescer.inflight.lock().is_empty());

        let (first, is_leader) = coalescer
            .coalesce("k", || async { Ok((200, Arc::from("fresh-1"))) })
            .await;
        assert!(is_leader);
        assert_eq!(&*first.unwrap().1, "fresh-1");

        let (second, is_leader) = coalescer
            .coalesce("k", || async { Ok((200, Arc::from("fresh-2"))) })
            .await;
        assert!(is_leader);
        assert_eq!(&*second.unwrap().1, "fresh-2");
    }
}
