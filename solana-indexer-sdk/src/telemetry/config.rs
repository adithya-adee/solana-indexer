/// Configuration for the telemetry subsystem.
///
/// Used with [`crate::telemetry::init_telemetry`] (Phase 1 — console only) or
/// [`crate::telemetry::init_telemetry_with_otel`] (Phase 2 — OTLP export).
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// Service name reported to OpenTelemetry backends.
    pub service_name: String,
    /// Minimum log level filter.
    ///
    /// Accepts the same syntax as `RUST_LOG`, e.g. `"info"`,
    /// `"solana_indexer_sdk=debug,warn"`. Overridden by the `RUST_LOG`
    /// environment variable if set.
    pub log_filter: String,
    /// Enable ANSI color escape codes in console output.
    pub enable_console_colors: bool,
    /// Include the Rust module target path in console output.
    pub show_target: bool,
    /// Include OS thread IDs in console output.
    pub show_thread_ids: bool,
    /// OpenTelemetry OTLP config.
    ///
    /// When `Some`, `init_telemetry_with_otel` will export spans to the
    /// configured OTLP endpoint. When `None` the OTel layer is skipped.
    ///
    /// Only available with the `opentelemetry` feature.
    #[cfg(feature = "opentelemetry")]
    pub otel: Option<super::otel::OtelConfig>,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            service_name: "solana-indexer".into(),
            log_filter: "info".into(),
            enable_console_colors: true,
            show_target: true,
            show_thread_ids: false,
            #[cfg(feature = "opentelemetry")]
            otel: None,
        }
    }
}
