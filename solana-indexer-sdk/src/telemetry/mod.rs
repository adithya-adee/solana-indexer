#[cfg(feature = "telemetry")]
pub mod config;
#[cfg(feature = "telemetry")]
pub mod subscriber;

#[cfg(feature = "telemetry")]
pub use config::TelemetryConfig;
#[cfg(feature = "telemetry")]
pub use subscriber::{init_telemetry, shutdown_telemetry};
