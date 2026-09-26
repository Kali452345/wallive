//! Wallive: low-resource live video wallpaper for Windows.
//!
//! Current stage: desktop-attach spike. Shows a solid colour behind the
//! desktop icons on every monitor and keeps it attached across Explorer
//! restarts and display changes. Console output is the log; Ctrl+C exits.

mod desktop;
mod runtime;

use std::sync::OnceLock;
use std::time::Instant;

/// Milliseconds since start, for log lines.
pub fn uptime_ms() -> u128 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis()
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        eprintln!("[{:>8} ms] {}", $crate::uptime_ms(), format_args!($($arg)*))
    };
}

fn main() {
    uptime_ms();
    if let Err(e) = runtime::run() {
        log!("fatal: {e}");
        std::process::exit(1);
    }
}
