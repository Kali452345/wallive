//! Wallive: low-resource live video wallpaper for Windows.
//!
//! Usage:
//!   wallive                                  run from the tray (settings in config.txt)
//!   wallive <video> [<video> ...]            use these videos (several take turns); hands them to a running instance
//!   wallive --quit                           ask a running instance to exit
//!   wallive --version                        print the version
//!   wallive --play <video>                   play <video> as-is, settings untouched (testing)
//!   wallive --import <src> <dst> <W>x<H>     import child process (ADR-004)
//!   wallive --pick                           file-dialog child process; prints the path
//!   wallive --make-test-clip <dst> [<W>x<H>] [fps] [secs]
//!   wallive --bench-decode <video> [loops]   hardware decode cost per frame
//!
//! The tray app logs to `%LOCALAPPDATA%\Wallive\wallive.log`; the other modes
//! log to stderr (a terminal they were started from, or a redirect).
#![cfg_attr(not(test), windows_subsystem = "windows")]

mod config;
mod desktop;
mod occlusion;
mod playback;
mod power;
mod runtime;
mod shell;
mod transcode;

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Milliseconds since start, for log lines.
pub fn uptime_ms() -> u128 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis()
}

struct LogFile {
    file: std::fs::File,
    written: u64,
}

static LOG_FILE: OnceLock<Mutex<LogFile>> = OnceLock::new();
/// The log stops growing here; events are rare, so this is weeks of use.
const LOG_LIMIT: u64 = 1 << 20;

/// Writes one log line to stderr and, in the tray app, to the log file.
pub fn write_log(args: std::fmt::Arguments) {
    let line = format!("[{:>8} ms] {args}\n", uptime_ms());
    let _ = std::io::stderr().write_all(line.as_bytes());
    if let Some(log) = LOG_FILE.get()
        && let Ok(mut log) = log.lock()
        && log.written < LOG_LIMIT
    {
        let _ = log.file.write_all(line.as_bytes());
        log.written += line.len() as u64;
        if log.written >= LOG_LIMIT {
            let _ = log.file.write_all(b"(log size limit reached)\n");
        }
    }
}

/// A handle to the log file for child processes' stderr.
pub fn log_file_clone() -> Option<std::fs::File> {
    LOG_FILE.get()?.lock().ok()?.file.try_clone().ok()
}

/// `%LOCALAPPDATA%\Wallive\wallive.log`, keeping the previous run's log as
/// `wallive.old.log`.
fn open_log_file() {
    let dir = config::local_dir();
    let path = dir.join("wallive.log");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::rename(&path, dir.join("wallive.old.log"));
    if let Ok(file) = std::fs::File::create(&path) {
        let _ = LOG_FILE.set(Mutex::new(LogFile { file, written: 0 }));
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::write_log(format_args!($($arg)*))
    };
}

fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once(['x', 'X'])?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn usage() -> ! {
    eprintln!(
        "usage: wallive [<video>... | --quit | --version | --play <video> | --import <src> <dst> <W>x<H> | --pick | --make-test-clip <dst> [<W>x<H>] [fps] [secs] | --bench-decode <video> [loops]]"
    );
    std::process::exit(2);
}

/// Runs the wallpaper, one instance per session. The tray app (not
/// `--play`) also logs to the log file, opened only once this is the one
/// instance so a second launch cannot rotate the running instance's log.
fn run_resident(options: runtime::Options) -> windows::core::Result<()> {
    let Some(_instance) = shell::SingleInstance::acquire() else {
        log!("Wallive is already running");
        std::process::exit(3);
    };
    if options.video.is_none() {
        open_log_file();
    }
    log!("Wallive {}", env!("CARGO_PKG_VERSION"));
    runtime::run(options)
}

fn main() {
    uptime_ms();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let first = args.first().map(String::as_str);
    if first.is_some() {
        shell::attach_parent_console();
    }
    let result = match first {
        None => run_resident(runtime::Options {
            video: None,
            open: Vec::new(),
        }),
        Some("--version") => {
            println!("wallive {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--quit") => {
            if !shell::close_running() {
                log!("Wallive is not running");
            }
            Ok(())
        }
        Some("--play") => {
            let Some(video) = args.get(1) else { usage() };
            run_resident(runtime::Options {
                video: Some(PathBuf::from(video)),
                open: Vec::new(),
            })
        }
        Some("--pick") => match shell::pick_videos() {
            Ok(paths) if !paths.is_empty() => {
                for path in paths {
                    println!("{}", path.display());
                }
                Ok(())
            }
            Ok(_) => std::process::exit(1),
            Err(e) => Err(e),
        },
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
        Some(flag) if flag.starts_with("--") => usage(),
        // Video paths: hand them to the running instance, or start with them.
        Some(_) => {
            let videos: Vec<PathBuf> = args
                .iter()
                .map(|v| std::path::absolute(v).unwrap_or_else(|_| PathBuf::from(v)))
                .collect();
            if shell::send_to_running(&videos, runtime::COPYDATA_OPEN) {
                Ok(())
            } else {
                run_resident(runtime::Options {
                    video: None,
                    open: videos,
                })
            }
        }
    };
    if let Err(e) = result {
        log!("fatal: {e}");
        std::process::exit(1);
    }
}
