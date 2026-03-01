//! Configurable retry logic for transient RPC failures.
//!
//! This module provides:
//! - [`compute_backoff`] — exponential-backoff delay calculator with optional jitter.
//! - [`is_transient`] — classifies a [`SolanaIndexerError`] as retryable or not.
//! - [`RetryingRpcProvider`] — decorator that wraps any [`RpcProvider`] with configurable retries.

use crate::config::RetryConfig;
use crate::utils::error::{Result, SolanaIndexerError};
use crate::utils::rpc::RpcProvider;
use async_trait::async_trait;
use solana_client::rpc_response::RpcConfirmedTransactionStatusWithSignature;
use solana_sdk::{
    account::Account, commitment_config::CommitmentConfig, pubkey::Pubkey, signature::Signature,
};
use solana_transaction_status::EncodedConfirmedTransactionWithStatusMeta;
use std::time::Duration;
use tokio::time::sleep;

// ─────────────────────────────────────────────────────────────────────────────
// Backoff calculation
// ─────────────────────────────────────────────────────────────────────────────

/// Computes the delay before the next retry.
///
/// `attempt` is 1-indexed: `attempt = 1` is the delay before the first retry,
/// `attempt = 2` before the second, etc.
///
/// Formula: `delay = initial_backoff_ms * backoff_multiplier^(attempt - 1)`,
/// capped at `max_backoff_ms`, then ±25 % jitter if enabled.
#[must_use]
pub fn compute_backoff(cfg: &RetryConfig, attempt: u32) -> Duration {
    let base = cfg.initial_backoff_ms as f64
        * cfg
            .backoff_multiplier
            .powi(attempt.saturating_sub(1) as i32);
    let capped = base.min(cfg.max_backoff_ms as f64);

    let ms = if cfg.jitter {
        // Simple pseudo-random jitter: scale by a fraction derived from the
        // current nanosecond count (no external rand dep needed).
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        // jitter factor in [0.75, 1.25]
        let factor = 0.75 + (nanos % 1_000_000) as f64 / 1_000_000.0 * 0.5;
        capped * factor
    } else {
        capped
    };

    Duration::from_millis(ms as u64)
}

// ─────────────────────────────────────────────────────────────────────────────
// Error classification
// ─────────────────────────────────────────────────────────────────────────────

/// Returns `true` if `err` represents a transient failure that is safe to retry.
///
/// Transient errors are typically caused by network instability, rate-limiting,
/// or RPC node overload. Permanent errors (bad data, configuration mistakes)
/// are **not** retried.
///
/// | Error variant            | Retried | Reason                                    |
/// |--------------------------|---------|-------------------------------------------|
/// | `RpcError`               | ✅      | Network blip or HTTP 429 / 503            |
/// | `RpcClientError`         | ✅      | Low-level `ClientError` (connection reset)|
/// | `ConnectionError`        | ✅      | gRPC / WebSocket drop                     |
/// | `InternalError`          | ✅      | tokio task join noise                     |
/// | `DatabaseError`          | ❌      | Schema / constraint violations are stable |
/// | `DecodingError`          | ❌      | Bad data will not self-heal               |
/// | `ConfigError`            | ❌      | Programmer error                          |
/// | `RetryExhausted`         | ❌      | Already exhausted                         |
#[must_use]
pub fn is_transient(err: &SolanaIndexerError) -> bool {
    matches!(
        err,
        SolanaIndexerError::RpcError(_)
            | SolanaIndexerError::RpcClientError(_)
            | SolanaIndexerError::ConnectionError(_)
            | SolanaIndexerError::InternalError(_)
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// RetryingRpcProvider
// ─────────────────────────────────────────────────────────────────────────────

/// A decorator that wraps any [`RpcProvider`] with configurable retry logic.
///
/// On each method call, if the inner provider returns a transient error the
/// decorator sleeps for an exponentially increasing delay (with optional jitter)
/// and retries transparently, up to `config.max_retries` times.
///
/// Permanent errors are returned immediately without retrying.
///
/// # Example
///
/// ```no_run
/// use solana_indexer_sdk::{RetryConfig};
/// use solana_indexer_sdk::utils::retry::RetryingRpcProvider;
/// use solana_indexer_sdk::utils::rpc::DefaultRpcProvider;
/// use std::sync::Arc;
///
/// let raw = DefaultRpcProvider::new("http://127.0.0.1:8899");
/// let retrying = RetryingRpcProvider::new(raw, RetryConfig::default());
/// let rpc: Arc<dyn solana_indexer_sdk::utils::rpc::RpcProvider> = Arc::new(retrying);
/// ```
pub struct RetryingRpcProvider<P> {
    inner: P,
    config: RetryConfig,
}

impl<P: RpcProvider> RetryingRpcProvider<P> {
    /// Wraps `inner` with the given retry `config`.
    pub fn new(inner: P, config: RetryConfig) -> Self {
        Self { inner, config }
    }

    /// Central retry loop: calls `op()` repeatedly until it succeeds, the
    /// error is non-transient, or `max_retries` is reached.
    async fn with_retry<F, Fut, T>(&self, op: F) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        #[allow(unused_assignments)]
        let mut last_err: Option<SolanaIndexerError> = None;
        let mut attempt = 0u32;

        loop {
            match op().await {
                Ok(val) => return Ok(val),
                Err(err) => {
                    if !is_transient(&err) {
                        // Permanent error — propagate immediately.
                        return Err(err);
                    }

                    attempt += 1;

                    let delay = compute_backoff(&self.config, attempt);
                    tracing::warn!(
                        attempt,
                        max = self.config.max_retries,
                        delay_ms = delay.as_millis(),
                        error = %err,
                        "Transient RPC error — retrying"
                    );

                    last_err = Some(err);

                    if attempt > self.config.max_retries {
                        break;
                    }

                    sleep(delay).await;
                }
            }
        }

        Err(SolanaIndexerError::RetryExhausted {
            attempts: attempt,
            last_error: last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
        })
    }
}

#[async_trait]
impl<P: RpcProvider + Send + Sync> RpcProvider for RetryingRpcProvider<P> {
    async fn get_signatures_for_address(
        &self,
        address: &Pubkey,
        before: Option<Signature>,
        until: Option<Signature>,
        limit: usize,
        commitment: Option<CommitmentConfig>,
    ) -> Result<Vec<RpcConfirmedTransactionStatusWithSignature>> {
        self.with_retry(|| {
            self.inner
                .get_signatures_for_address(address, before, until, limit, commitment)
        })
        .await
    }

    async fn get_transaction(
        &self,
        signature: &Signature,
        commitment: Option<CommitmentConfig>,
    ) -> Result<EncodedConfirmedTransactionWithStatusMeta> {
        self.with_retry(|| self.inner.get_transaction(signature, commitment))
            .await
    }

    async fn get_multiple_accounts(
        &self,
        pubkeys: &[Pubkey],
        commitment: Option<CommitmentConfig>,
    ) -> Result<Vec<Option<Account>>> {
        self.with_retry(|| self.inner.get_multiple_accounts(pubkeys, commitment))
            .await
    }

    async fn get_block(
        &self,
        slot: u64,
        commitment: Option<CommitmentConfig>,
    ) -> Result<solana_transaction_status::UiConfirmedBlock> {
        self.with_retry(|| self.inner.get_block(slot, commitment))
            .await
    }

    async fn get_program_accounts(&self, program_id: &Pubkey) -> Result<Vec<(Pubkey, Account)>> {
        self.with_retry(|| self.inner.get_program_accounts(program_id))
            .await
    }

    async fn get_slot(&self, commitment: Option<CommitmentConfig>) -> Result<u64> {
        self.with_retry(|| self.inner.get_slot(commitment)).await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn no_jitter_cfg(max_retries: u32) -> RetryConfig {
        RetryConfig {
            max_retries,
            initial_backoff_ms: 1, // keep tests fast
            backoff_multiplier: 2.0,
            max_backoff_ms: 100,
            jitter: false,
        }
    }

    // ── backoff ───────────────────────────────────────────────────────────────

    #[test]
    fn test_compute_backoff_increases() {
        let cfg = no_jitter_cfg(5);
        let d1 = compute_backoff(&cfg, 1);
        let d2 = compute_backoff(&cfg, 2);
        let d3 = compute_backoff(&cfg, 3);
        assert!(d1 < d2, "backoff should grow: {d1:?} < {d2:?}");
        assert!(d2 < d3, "backoff should grow: {d2:?} < {d3:?}");
    }

    #[test]
    fn test_compute_backoff_capped() {
        let cfg = RetryConfig {
            max_backoff_ms: 500,
            initial_backoff_ms: 100,
            backoff_multiplier: 10.0,
            jitter: false,
            ..RetryConfig::default()
        };
        // attempt 4 → 100 * 10^3 = 100_000 ms → should be capped at 500
        let d = compute_backoff(&cfg, 4);
        assert_eq!(d.as_millis(), 500, "delay should be capped");
    }

    #[test]
    fn test_compute_backoff_attempt1_equals_initial() {
        let cfg = no_jitter_cfg(5);
        assert_eq!(
            compute_backoff(&cfg, 1),
            Duration::from_millis(cfg.initial_backoff_ms)
        );
    }

    // ── is_transient ──────────────────────────────────────────────────────────

    #[test]
    fn test_is_transient_rpc_error() {
        assert!(is_transient(&SolanaIndexerError::RpcError(
            "timeout".into()
        )));
    }

    #[test]
    fn test_is_transient_connection_error() {
        assert!(is_transient(&SolanaIndexerError::ConnectionError(
            "dropped".into()
        )));
    }

    #[test]
    fn test_is_transient_internal_error() {
        assert!(is_transient(&SolanaIndexerError::InternalError(
            "join".into()
        )));
    }

    #[test]
    fn test_is_transient_decoding_false() {
        assert!(!is_transient(&SolanaIndexerError::DecodingError(
            "bad".into()
        )));
    }

    #[test]
    fn test_is_transient_config_false() {
        assert!(!is_transient(&SolanaIndexerError::ConfigError(
            "missing".into()
        )));
    }

    // ── mock provider ─────────────────────────────────────────────────────────

    struct MockProvider {
        fail_count: Arc<AtomicU32>,
        calls: Arc<AtomicU32>,
        permanent: bool, // if true, always fails with DecodingError (non-transient)
    }

    #[async_trait]
    impl RpcProvider for MockProvider {
        async fn get_signatures_for_address(
            &self,
            _: &Pubkey,
            _: Option<Signature>,
            _: Option<Signature>,
            _: usize,
            _: Option<CommitmentConfig>,
        ) -> Result<Vec<RpcConfirmedTransactionStatusWithSignature>> {
            let call_no = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.permanent || call_no <= self.fail_count.load(Ordering::SeqCst) {
                let err = if self.permanent {
                    SolanaIndexerError::DecodingError("permanent".into())
                } else {
                    SolanaIndexerError::RpcError("transient".into())
                };
                Err(err)
            } else {
                Ok(vec![])
            }
        }

        async fn get_transaction(
            &self,
            _: &Signature,
            _: Option<CommitmentConfig>,
        ) -> Result<EncodedConfirmedTransactionWithStatusMeta> {
            unimplemented!()
        }

        async fn get_multiple_accounts(
            &self,
            _: &[Pubkey],
            _: Option<CommitmentConfig>,
        ) -> Result<Vec<Option<Account>>> {
            unimplemented!()
        }

        async fn get_block(
            &self,
            _: u64,
            _: Option<CommitmentConfig>,
        ) -> Result<solana_transaction_status::UiConfirmedBlock> {
            unimplemented!()
        }

        async fn get_program_accounts(&self, _: &Pubkey) -> Result<Vec<(Pubkey, Account)>> {
            unimplemented!()
        }

        async fn get_slot(&self, _: Option<CommitmentConfig>) -> Result<u64> {
            unimplemented!()
        }
    }

    // ── retry behaviour ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_retry_succeeds_after_transient_failures() {
        let calls = Arc::new(AtomicU32::new(0));
        let mock = MockProvider {
            fail_count: Arc::new(AtomicU32::new(2)), // fail first 2 calls
            calls: calls.clone(),
            permanent: false,
        };
        let retrying = RetryingRpcProvider::new(mock, no_jitter_cfg(5));
        let result = retrying
            .get_signatures_for_address(&Pubkey::default(), None, None, 1, None)
            .await;

        assert!(result.is_ok(), "should eventually succeed: {result:?}");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "should have called 3 times"
        );
    }

    #[tokio::test]
    async fn test_retry_permanent_error_no_retry() {
        let calls = Arc::new(AtomicU32::new(0));
        let mock = MockProvider {
            fail_count: Arc::new(AtomicU32::new(99)),
            calls: calls.clone(),
            permanent: true, // DecodingError is non-transient
        };
        let retrying = RetryingRpcProvider::new(mock, no_jitter_cfg(5));
        let result = retrying
            .get_signatures_for_address(&Pubkey::default(), None, None, 1, None)
            .await;

        assert!(result.is_err(), "should fail immediately");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "permanent error should not be retried"
        );
    }

    #[tokio::test]
    async fn test_retry_exhausts_after_max_retries() {
        let calls = Arc::new(AtomicU32::new(0));
        let max = 3u32;
        let mock = MockProvider {
            fail_count: Arc::new(AtomicU32::new(999)), // always fail (transient)
            calls: calls.clone(),
            permanent: false,
        };
        let retrying = RetryingRpcProvider::new(mock, no_jitter_cfg(max));
        let result = retrying
            .get_signatures_for_address(&Pubkey::default(), None, None, 1, None)
            .await;

        assert!(
            matches!(result, Err(SolanaIndexerError::RetryExhausted { .. })),
            "should return RetryExhausted"
        );
        // 1 initial call + max_retries retries
        assert_eq!(
            calls.load(Ordering::SeqCst),
            max + 1,
            "should have called max_retries+1 times total"
        );
    }
}
