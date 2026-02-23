use super::config::TelemetryConfig;
use std::sync::OnceLock;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

// ── Shared state ─────────────────────────────────────────────────────────────

/// Ensures the global subscriber is installed exactly once.
static TELEMETRY_INIT: OnceLock<()> = OnceLock::new();

/// Holds the `SdkTracerProvider` so we can shut it down gracefully.
///
/// Populated only when the `opentelemetry` feature is active and
/// `init_telemetry_with_otel` succeeds.
#[cfg(feature = "opentelemetry")]
static OTEL_PROVIDER: OnceLock<opentelemetry_sdk::trace::SdkTracerProvider> = OnceLock::new();

// ── Guard ─────────────────────────────────────────────────────────────────────

/// RAII guard that keeps the telemetry subsystem alive.
///
/// Dropping this value calls [`shutdown_telemetry`], which flushes any
/// buffered spans. Hold the guard for the lifetime of `main`.
pub struct TelemetryGuard {
    _private: (),
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        shutdown_telemetry();
    }
}

// ── Phase 1: console-only init ────────────────────────────────────────────────

/// Initialize the global tracing subscriber with console (fmt) output only.
///
/// Reads `RUST_LOG` for the level filter; falls back to `config.log_filter`.
/// Safe to call multiple times — subsequent calls are no-ops and return a new
/// guard that is a no-op on drop.
pub fn init_telemetry(config: TelemetryConfig) -> TelemetryGuard {
    TELEMETRY_INIT.get_or_init(|| {
        let env_filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(&config.log_filter));

        let fmt_layer = fmt::layer()
            .with_target(config.show_target)
            .with_thread_ids(config.show_thread_ids)
            .with_ansi(config.enable_console_colors);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .init();
    });

    TelemetryGuard { _private: () }
}

// ── Phase 2: OTLP init ────────────────────────────────────────────────────────

/// Initialize the global tracing subscriber with console output **and** OTLP
/// span export.
///
/// Reads `RUST_LOG` for the level filter; falls back to `config.log_filter`.
/// The OTel layer is only added when `config.otel` is `Some`.  
/// Safe to call multiple times — subsequent calls are no-ops.
///
/// If the OTLP exporter cannot be built (e.g. bad endpoint, TLS error), the
/// function logs a warning to `stderr` and falls back to console-only output.
#[cfg(feature = "opentelemetry")]
pub fn init_telemetry_with_otel(config: TelemetryConfig) -> TelemetryGuard {
    use opentelemetry::global;

    TELEMETRY_INIT.get_or_init(|| {
        let env_filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(&config.log_filter));

        let fmt_layer = fmt::layer()
            .with_target(config.show_target)
            .with_thread_ids(config.show_thread_ids)
            .with_ansi(config.enable_console_colors);

        // Only add the OTel layer when an OtelConfig is provided.
        if let Some(ref otel_cfg) = config.otel {
            match super::otel::build_otel_pipeline(&config.service_name, otel_cfg) {
                Ok((otel_layer, provider)) => {
                    // Register as global provider so code using the OTel API
                    // directly (e.g. `opentelemetry::global::tracer(...)`) works.
                    global::set_tracer_provider(provider.clone());

                    // Store for graceful shutdown.
                    let _ = OTEL_PROVIDER.set(provider);

                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(fmt_layer)
                        .with(otel_layer)
                        .init();
                }
                Err(e) => {
                    eprintln!(
                        "[solana-indexer-sdk] WARNING: failed to build OTLP pipeline \
                         (falling back to console-only): {e}"
                    );
                    tracing_subscriber::registry()
                        .with(env_filter)
                        .with(fmt_layer)
                        .init();
                }
            }
        } else {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .init();
        }
    });

    TelemetryGuard { _private: () }
}

// ── Shutdown ──────────────────────────────────────────────────────────────────

/// Flush and shut down the telemetry pipeline.
///
/// - **Phase 1**: no-op (nothing to flush for the fmt layer).
/// - **Phase 2**: shuts down the `SdkTracerProvider`, flushing any in-flight
///   spans from the batch exporter before process exit.
///
/// Called automatically when [`TelemetryGuard`] is dropped.
pub fn shutdown_telemetry() {
    #[cfg(feature = "opentelemetry")]
    if let Some(provider) = OTEL_PROVIDER.get() {
        if let Err(e) = provider.shutdown() {
            eprintln!("[solana-indexer-sdk] WARNING: OTel provider shutdown error: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::config::TelemetryConfig;

    // ── TelemetryConfig ───────────────────────────────────────────────────────

    /// Default config should represent the "sensible production defaults" we
    /// document: info level, ANSI colours on, target path shown.
    #[test]
    fn telemetry_config_defaults_are_sane() {
        let cfg = TelemetryConfig::default();

        assert_eq!(cfg.service_name, "solana-indexer");
        assert_eq!(cfg.log_filter, "info");
        assert!(cfg.enable_console_colors, "colours should be on by default");
        assert!(cfg.show_target, "module target should be shown by default");
        assert!(
            !cfg.show_thread_ids,
            "thread IDs off by default (low noise)"
        );

        // When the opentelemetry feature is active the otel field must
        // default to None so the caller consciously opts in to OTLP export.
        #[cfg(feature = "opentelemetry")]
        assert!(
            cfg.otel.is_none(),
            "otel export must be opt-in, not on by default"
        );
    }

    /// TelemetryConfig::clone must produce an independent copy.
    #[test]
    fn telemetry_config_is_clone() {
        let original = TelemetryConfig {
            service_name: "my-service".into(),
            log_filter: "debug".into(),
            enable_console_colors: false,
            show_target: false,
            show_thread_ids: true,
            #[cfg(feature = "opentelemetry")]
            otel: None,
        };

        let cloned = original.clone();
        assert_eq!(cloned.service_name, "my-service");
        assert_eq!(cloned.log_filter, "debug");
        assert!(!cloned.enable_console_colors);
    }

    // ── TelemetryGuard ────────────────────────────────────────────────────────

    /// TelemetryGuard must implement Send + Sync so it can be stored in
    /// thread-local state or sent across async tasks.
    #[test]
    fn telemetry_guard_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TelemetryGuard>();
    }

    // ── OtelConfig (opentelemetry feature only) ───────────────────────────────

    #[cfg(feature = "opentelemetry")]
    mod otel_tests {
        use crate::telemetry::otel::{OtelConfig, OtlpProtocol};

        /// The default endpoint must match what Jaeger / OTel Collector expose
        /// out-of-the-box on a developer machine.
        #[test]
        fn otel_config_default_endpoint_is_grpc() {
            let cfg = OtelConfig::default();
            assert_eq!(cfg.endpoint, "http://localhost:4317");
            assert!(
                matches!(cfg.protocol, OtlpProtocol::Grpc),
                "default protocol must be gRPC"
            );
        }

        /// OtelConfig must be Clone so it can be stored in TelemetryConfig.
        #[test]
        fn otel_config_is_clone() {
            let original = OtelConfig {
                endpoint: "http://collector:4317".into(),
                protocol: OtlpProtocol::Http,
            };
            let cloned = original.clone();
            assert_eq!(cloned.endpoint, "http://collector:4317");
            assert!(matches!(cloned.protocol, OtlpProtocol::Http));
        }

        /// build_otel_pipeline must succeed at the type-checking stage —
        /// verifying that all generic bounds (Layer<Registry>) are satisfied.
        ///
        /// We use a deliberately unreachable endpoint so the test does not
        /// depend on a running collector.  The important assertion is that
        /// the function *returns* (either Ok or Err) without panicking and
        /// without any compiler errors.
        #[tokio::test]
        async fn build_pipeline_with_bad_endpoint_returns_err_not_panic() {
            use crate::telemetry::otel::build_otel_pipeline;

            // Point at a port that is guaranteed to refuse connections.
            // The OTLP builder validates the URL format but does not open a
            // connection until the first span is exported, so this call should
            // succeed and return Ok — connection failures happen lazily.
            let cfg = OtelConfig {
                endpoint: "http://127.0.0.1:1".into(), // port 1 is reserved / always closed
                protocol: OtlpProtocol::Grpc,
            };

            // Either outcome is acceptable here: some OTLP builds validate the
            // URL eagerly, others defer. What must NOT happen is a panic.
            let result = build_otel_pipeline::<tracing_subscriber::Registry>("test-service", &cfg);
            match result {
                Ok((_layer, provider)) => {
                    // Got a pipeline — shut it down cleanly.
                    let _ = provider.shutdown();
                }
                Err(e) => {
                    // Validation rejected the config — that is also fine.
                    // Just ensure the error message is non-empty.
                    assert!(
                        !e.to_string().is_empty(),
                        "error from build_otel_pipeline must have a description"
                    );
                }
            }
        }

        /// init_telemetry_with_otel with otel: None must behave identically to
        /// init_telemetry (console-only mode, no OTLP layer added).
        ///
        /// We cannot install the global subscriber a second time in the same
        /// process (OnceLock prevents it), so we validate indirectly: the
        /// function must not panic and must return a TelemetryGuard.
        #[test]
        fn init_with_otel_none_does_not_panic() {
            use crate::telemetry::config::TelemetryConfig;
            use crate::telemetry::subscriber::init_telemetry_with_otel;

            let cfg = TelemetryConfig {
                otel: None, // explicitly no OTLP
                ..TelemetryConfig::default()
            };

            // If the global subscriber is already installed (possible when
            // tests run in the same process) this is a no-op — that is the
            // expected behaviour.
            let _guard = init_telemetry_with_otel(cfg);
            // Reaching here without a panic is the assertion.
        }
    }

    // ── Singleton contract ────────────────────────────────────────────────────

    /// Calling init_telemetry twice must NOT panic.
    ///
    /// The second call must be a silent no-op (the subscriber is already
    /// installed). This is the primary safety guarantee of the OnceLock design.
    #[test]
    fn init_telemetry_is_idempotent() {
        let cfg = TelemetryConfig::default();

        // First call: installs (or does nothing if already installed by
        // another test in this process).
        let _g1 = init_telemetry(cfg.clone());

        // Second call: must be a no-op, not a panic.
        let _g2 = init_telemetry(cfg);
    }

    /// shutdown_telemetry must be callable at any time — even before any
    /// subscriber has been installed — without panicking.
    #[test]
    fn shutdown_before_init_is_safe() {
        shutdown_telemetry();
        shutdown_telemetry(); // twice for good measure
    }
}
