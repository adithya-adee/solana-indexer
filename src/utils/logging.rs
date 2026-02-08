//! Logging utilities for production-ready colorful output

use colored::Colorize;

/// Log levels for the indexer
#[derive(Clone, Copy)]
pub enum LogLevel {
    Info,
    Success,
    Warning,
    Error,
    Debug,
}

/// Logs a message with color and formatting
pub fn log(level: LogLevel, message: &str) {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");

    match level {
        LogLevel::Info => {
            println!(
                "{} {} {}",
                format!("[{timestamp}]").bright_black(),
                "ℹ".bright_blue(),
                message
            );
        }
        LogLevel::Success => {
            println!(
                "{} {} {}",
                format!("[{timestamp}]").bright_black(),
                "✓".bright_green(),
                message.green()
            );
        }
        LogLevel::Warning => {
            println!(
                "{} {} {}",
                format!("[{timestamp}]").bright_black(),
                "⚠".bright_yellow(),
                message.yellow()
            );
        }
        LogLevel::Error => {
            eprintln!(
                "{} {} {}",
                format!("[{timestamp}]").bright_black(),
                "✗".bright_red(),
                message.red()
            );
        }
        LogLevel::Debug => {
            println!(
                "{} {} {}",
                format!("[{timestamp}]").bright_black(),
                "🔍".bright_magenta(),
                message.bright_black()
            );
        }
    }
}

/// Logs indexer startup information
pub fn log_startup(program_id: &str, rpc_url: &str, poll_interval: u64) {
    println!("\n{}", "═".repeat(80).bright_blue());
    println!("{}", "  Solana Indexer".bright_cyan().bold());
    println!("{}", "═".repeat(80).bright_blue());
    println!("  {} {}", "Program ID:".bright_white(), program_id.cyan());
    println!("  {} {}", "RPC URL:   ".bright_white(), rpc_url.cyan());
    println!(
        "  {} {}s",
        "Poll Interval:".bright_white(),
        poll_interval.to_string().cyan()
    );
    println!("{}\n", "═".repeat(80).bright_blue());
}

/// Logs transaction processing
pub fn log_transaction(signature: &str, slot: u64, events: usize) {
    println!(
        "{} {} {} {} {} {} {} {}",
        "✓".bright_green(),
        "Tx".bright_white(),
        signature[..8].bright_cyan(),
        "│".bright_black(),
        format!("slot {slot}").bright_black(),
        "│".bright_black(),
        format!("{events} events").bright_green(),
        if events > 0 {
            "✓".green()
        } else {
            "".normal()
        }
    );
}

/// Logs batch processing summary
pub fn log_batch(processed: usize, total: usize, duration_ms: u64) {
    if processed > 0 {
        println!(
            "{} {} {} {} {} {}ms",
            "📦".bright_blue(),
            "Batch:".bright_white(),
            format!("{processed}/{total}").bright_cyan(),
            "processed".bright_white(),
            "in".bright_black(),
            duration_ms.to_string().bright_yellow()
        );
    }
}

/// Logs an error with context
pub fn log_error(context: &str, error: &str) {
    eprintln!(
        "{} {} {} {}",
        "✗".bright_red(),
        context.red().bold(),
        "│".bright_black(),
        error.bright_red()
    );
}
