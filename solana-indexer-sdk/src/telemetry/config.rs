/// Configuration for the telemetry subsystem.
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// Service name for OTel resource identification.
    pub service_name: String,
    /// Minimum log level filter (e.g. "info", "debug", "solana_indexer_sdk=debug,warn").
    pub log_filter: String,
    /// Whether to enable ANSI-colored console output.
    pub enable_console_colors: bool,
    /// Whether to include target module in output.
    pub show_target: bool,
    /// Whether to include thread IDs.
    pub show_thread_ids: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            service_name: "solana-indexer".into(),
            log_filter: "info".into(),
            enable_console_colors: true,
            show_target: true,
            show_thread_ids: false,
        }
    }
}
