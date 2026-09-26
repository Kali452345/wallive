//! Wallive: low-resource live video wallpaper for Windows.
//!
//! Usage:
//!   wallive                                  run with the configured video
//!   wallive --play <video>                   run with <video> (no import)
//!   wallive --import <src> <dst> <W>x<H>     import child process (ADR-004)
//!   wallive --make-test-clip <dst> [<W>x<H>] [fps] [secs]
//!   wallive --bench-decode <video> [loops]   hardware decode cost per frame
//!
//! Console output is the log; Ctrl+C exits.

mod desktop;
mod playback;
mod runtime;
mod transcode;

use std::path::PathBuf;
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

fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once(['x', 'X'])?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn usage() -> ! {
    eprintln!(
        "usage: wallive [--play <video> | --import <src> <dst> <W>x<H> | --make-test-clip <dst> [<W>x<H>] [fps] [secs]]"
    );
    std::process::exit(2);
}

fn main() {
    uptime_ms();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None => runtime::run(runtime::Options { video: None }),
        Some("--play") => {
            let Some(video) = args.get(1) else { usage() };
            runtime::run(runtime::Options {
                video: Some(PathBuf::from(video)),
            })
        }
        Some("--import") => {
            let (Some(src), Some(dst), Some(size)) = (
                args.get(1),
                args.get(2),
                args.get(3).and_then(|s| parse_size(s)),
            ) else {
                usage()
            };
            transcode::import(src.as_ref(), dst.as_ref(), size).map(|_| ())
        }
        Some("--bench-decode") => {
            let Some(src) = args.get(1) else { usage() };
            let loops = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3);
            playback::bench_decode(src.as_ref(), loops)
        }
        Some("--make-test-clip") => {
            let Some(dst) = args.get(1) else { usage() };
            let (w, h) = args
                .get(2)
                .and_then(|s| parse_size(s))
                .unwrap_or((1920, 1080));
            let fps = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(30);
            let secs = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(10);
            transcode::make_test_clip(dst.as_ref(), w, h, fps, secs)
        }
        Some(_) => usage(),
    };
    if let Err(e) = result {
        log!("fatal: {e}");
        std::process::exit(1);
    }
}
