//! Runtime metrics for indexer registries.
//!
//! This module provides the `RegistryMetrics` struct which tracks usage statistics
//! for registries, including the number of registered items, decode calls, and
//! cache hits. It also enforces capacity limits.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Metrics and capacity tracking for a registry.
#[derive(Debug)]
pub struct RegistryMetrics {
    /// Number of items currently registered.
    pub registered_count: AtomicUsize,
    /// Total number of decode attempts/calls.
    pub decode_calls: AtomicU64,
    /// Total number of successful decodes/hits.
    pub decode_hits: AtomicU64,
    /// Maximum capacity of the registry (0 = unlimited).
    pub capacity_limit: usize,
    /// Name of the registry for logging.
    pub name: &'static str,
}

impl RegistryMetrics {
    /// Creates a new metrics instance.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the registry (e.g., "InstructionDecoder")
    /// * `capacity_limit` - The maximum number of items allowed (0 for unlimited)
    pub fn new(name: &'static str, capacity_limit: usize) -> Self {
        Self {
            registered_count: AtomicUsize::new(0),
            decode_calls: AtomicU64::new(0),
            decode_hits: AtomicU64::new(0),
            capacity_limit,
            name,
        }
    }

    /// Checks if the registry is at capacity.
    ///
    /// # Returns
    ///
    /// `true` if the registry has a limit and successful registration would exceed it.
    /// Returns `true` if the registry enforces a limit and adding one more item would exceed it.
    #[must_use]
    pub fn is_full(&self) -> bool {
        if self.capacity_limit == 0 {
            return false;
        }
        self.registered_count.load(Ordering::Relaxed) >= self.capacity_limit
    }

    /// Increments the registered count.
    pub fn inc_registered(&self) {
        self.registered_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the decode call count.
    pub fn inc_calls(&self) {
        self.decode_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the decode hit count.
    pub fn inc_hits(&self) {
        self.decode_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Reports current metrics to logs.
    pub fn report(&self) {
        let count = self.registered_count.load(Ordering::Relaxed);
        let calls = self.decode_calls.load(Ordering::Relaxed);
        let hits = self.decode_hits.load(Ordering::Relaxed);

        let limit_str = if self.capacity_limit == 0 {
            "unlimited".to_string()
        } else {
            self.capacity_limit.to_string()
        };

        crate::utils::logging::log(
            crate::utils::logging::LogLevel::Info,
            &format!(
                "Registry [{}] Stats: {}/{} items | Calls: {} | Hits: {}",
                self.name, count, limit_str, calls, hits
            ),
        );
    }
}
