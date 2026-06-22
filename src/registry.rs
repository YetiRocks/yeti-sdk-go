//! Process-global handle registries and the shared async runtime.
//!
//! Go cannot hold Rust objects, so every live object the binding owns
//! ([`client::Client`], cursors, queries, record streams, subscriptions) lives
//! in a [`Registry`] keyed by an opaque `u64` handle. The async core is driven
//! to completion on a single shared multi-thread tokio [`Runtime`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tokio::runtime::Runtime;
use yeti_sdk::remote as client;

/// The shared multi-thread runtime that drives the async client. Created on
/// first use.
pub(crate) fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime for the Yeti Go binding")
    })
}

/// Process-global registry mapping opaque `u64` handles to live `T`. Handles
/// are never reused within a process run.
pub(crate) struct Registry<T> {
    next: AtomicU64,
    pub(crate) map: Mutex<HashMap<u64, T>>,
}

impl<T> Registry<T> {
    pub(crate) fn new() -> Self {
        Registry {
            next: AtomicU64::new(1),
            map: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn insert(&self, value: T) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.map.lock().unwrap().insert(id, value);
        id
    }

    pub(crate) fn remove(&self, id: u64) -> Option<T> {
        self.map.lock().unwrap().remove(&id)
    }

    /// Run `f` against the live `T` for `id` while holding the registry lock,
    /// returning `None` if the handle is dead. Used for the `&self` handle ops
    /// (a `Query`'s refine/terminal calls) that neither mutate nor consume the
    /// registered value. `f` borrows the value and the lock is held for its
    /// whole duration, so a blocking await on the borrowed future is driven to
    /// completion inside `f` (the borrow never escapes the guard).
    pub(crate) fn with<R>(&self, id: u64, f: impl FnOnce(&T) -> R) -> Option<R> {
        self.map.lock().unwrap().get(&id).map(f)
    }
}

pub(crate) fn clients() -> &'static Registry<client::Client> {
    static R: OnceLock<Registry<client::Client>> = OnceLock::new();
    R.get_or_init(Registry::new)
}

pub(crate) fn cursors() -> &'static Registry<client::ScanCursor> {
    static R: OnceLock<Registry<client::ScanCursor>> = OnceLock::new();
    R.get_or_init(Registry::new)
}

pub(crate) fn queries() -> &'static Registry<client::Query> {
    static R: OnceLock<Registry<client::Query>> = OnceLock::new();
    R.get_or_init(Registry::new)
}

pub(crate) fn record_streams() -> &'static Registry<client::RecordStream> {
    static R: OnceLock<Registry<client::RecordStream>> = OnceLock::new();
    R.get_or_init(Registry::new)
}

pub(crate) fn subscriptions() -> &'static Registry<client::Subscription> {
    static R: OnceLock<Registry<client::Subscription>> = OnceLock::new();
    R.get_or_init(Registry::new)
}

/// Resolve a live `Store` facade for `client`, or `None` if the handle is
/// dead.
pub(crate) fn store_of(client: u64) -> Option<client::Store> {
    clients()
        .map
        .lock()
        .unwrap()
        .get(&client)
        .map(client::Client::store)
}

pub(crate) fn log_of(client: u64) -> Option<client::Log> {
    clients()
        .map
        .lock()
        .unwrap()
        .get(&client)
        .map(client::Client::log)
}

pub(crate) fn publisher_of(client: u64) -> Option<client::Publisher> {
    clients()
        .map
        .lock()
        .unwrap()
        .get(&client)
        .map(client::Client::publisher)
}

pub(crate) fn subscriber_of(client: u64) -> Option<client::Subscriber> {
    clients()
        .map
        .lock()
        .unwrap()
        .get(&client)
        .map(client::Client::subscriber)
}

pub(crate) fn rate_limit_of(client: u64) -> Option<client::RateLimit> {
    clients()
        .map
        .lock()
        .unwrap()
        .get(&client)
        .map(client::Client::rate_limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_start_at_one_and_increase() {
        let reg: Registry<u32> = Registry::new();
        let h1 = reg.insert(10);
        let h2 = reg.insert(20);
        assert_eq!(h1, 1);
        assert_eq!(h2, 2);
        assert!(h2 > h1, "handles are monotonic");
    }

    #[test]
    fn remove_returns_the_value_then_none() {
        let reg: Registry<String> = Registry::new();
        let h = reg.insert("hello".to_owned());
        assert_eq!(reg.remove(h), Some("hello".to_owned()));
        assert_eq!(reg.remove(h), None, "a removed handle is dead");
    }

    #[test]
    fn with_borrows_live_value_and_misses_dead_handle() {
        let reg: Registry<Vec<u8>> = Registry::new();
        let h = reg.insert(vec![1, 2, 3]);
        let len = reg.with(h, |v| v.len());
        assert_eq!(len, Some(3));
        reg.remove(h);
        assert_eq!(reg.with(h, |v| v.len()), None);
    }

    #[test]
    fn runtime_drives_a_future_to_completion() {
        let n = runtime().block_on(async { 21 + 21 });
        assert_eq!(n, 42);
    }

    #[test]
    fn facade_resolver_misses_a_dead_client_handle() {
        // No client is registered under a fabricated handle, so the facade
        // resolvers must report the miss rather than panic.
        assert!(store_of(999_999).is_none());
        assert!(log_of(999_999).is_none());
        assert!(publisher_of(999_999).is_none());
        assert!(subscriber_of(999_999).is_none());
        assert!(rate_limit_of(999_999).is_none());
    }
}
