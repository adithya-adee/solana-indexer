// ── Submodules ────────────────────────────────────────────────────────────────

/// Telemetry configuration.
#[cfg(feature = "telemetry")]
pub mod config;

/// Global subscriber initialisation and graceful shutdown.
#[cfg(feature = "telemetry")]
pub mod subscriber;

/// OpenTelemetry OTLP pipeline builder.
///
/// Only compiled with `--features opentelemetry`.
#[cfg(feature = "opentelemetry")]
pub mod otel;

// ── Re-exports ────────────────────────────────────────────────────────────────

#[cfg(feature = "telemetry")]
pub use config::TelemetryConfig;

#[cfg(feature = "telemetry")]
pub use subscriber::{init_telemetry, shutdown_telemetry, TelemetryGuard};

/// OTel-aware initialiser. Requires `--features opentelemetry`.
#[cfg(feature = "opentelemetry")]
pub use subscriber::init_telemetry_with_otel;

/// Typed config for the OTLP exporter. Requires `--features opentelemetry`.
#[cfg(feature = "opentelemetry")]
pub use otel::{OtelConfig, OtlpProtocol};
