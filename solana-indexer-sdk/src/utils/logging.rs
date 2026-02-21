//! Logging utilities (legacy wrappers for tracing)

/// Log levels for the indexer
#[derive(Clone, Copy)]
pub enum LogLevel {
    Info,
    Success,
    Warning,
    Error,
    Debug,
}

/// Logs a message
pub fn log(level: LogLevel, message: &str) {
    if std::env::var("SOLANA_INDEXER_SILENT").is_ok() {
        return;
    }

    match level {
        LogLevel::Info | LogLevel::Success => tracing::info!("{}", message),
        LogLevel::Warning => tracing::warn!("{}", message),
        LogLevel::Error => tracing::error!("{}", message),
        LogLevel::Debug => tracing::debug!("{}", message),
    }
}

/// Logs indexer startup information
pub fn log_startup(program_id: &str, rpc_url: &str, poll_interval: u64) {
    if std::env::var("SOLANA_INDEXER_SILENT").is_ok() {
        return;
    }

    // Sanitize RPC URL
    let sanitized_url = if rpc_url.contains("api-key=") {
        if let Some(pos) = rpc_url.find("api-key=") {
            let before = &rpc_url[..pos + 8];
            let after = &rpc_url[pos + 8..];
            let end_pos = after.find('&').unwrap_or(after.len());
            format!("{}[REDACTED]{}", before, &after[end_pos..])
        } else {
            rpc_url.to_string()
        }
    } else {
        rpc_url.to_string()
    };

    tracing::info!(
        program_id = program_id,
        rpc_url = sanitized_url,
        poll_interval_s = poll_interval,
        "Solana Indexer Startup"
    );
}

/// Logs a section header
pub fn log_section(title: &str) {
    tracing::info!("=== {} ===", title);
}

/// Logs transaction processing
pub fn log_transaction(signature: &str, slot: u64, events: usize) {
    tracing::debug!(
        signature = signature,
        slot = slot,
        events = events,
        "Processed Transaction"
    );
}

/// Logs batch processing summary
pub fn log_batch(processed: usize, total: usize, duration_ms: u64) {
    if std::env::var("SOLANA_INDEXER_SILENT").is_ok() {
        return;
    }
    if processed > 0 {
        tracing::info!(
            processed = processed,
            total = total,
            duration_ms = duration_ms,
            "Batch processed"
        );
    }
}

/// Logs an error with context
pub fn log_error(context: &str, error: &str) {
    tracing::error!(context = context, error = error, "Indexer Error");
}
