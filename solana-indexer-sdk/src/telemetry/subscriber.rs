use super::config::TelemetryConfig;
use std::sync::OnceLock;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Guard that keeps the telemetry subsystem alive.
/// Drop triggers graceful shutdown.
pub struct TelemetryGuard {
    _private: (),
}

static TELEMETRY_INIT: OnceLock<()> = OnceLock::new();

/// Initialize the global tracing subscriber (singleton, called once).
///
/// Uses `RUST_LOG` env var if set, otherwise falls back to `config.log_filter`.
/// Safe to call multiple times — subsequent calls are no-ops.
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

/// Explicit shutdown (flushes any pending spans).
pub fn shutdown_telemetry() {
    // In Phase 1, this is a no-op. Phase 2 adds OTel provider shutdown here.
}
