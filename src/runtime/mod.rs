//! Event wiring: the hidden host window, Explorer event subscription, and
//! the reactions that keep the wallpaper attached and playing.
//!
//! Wakes only for OS notifications:
//! - `TaskbarCreated` (Explorer restart): re-hook the new Explorer, re-attach.
//! - `WM_DISPLAYCHANGE`: re-attach to the new monitor layout.
//! - Explorer window events: cheap re-check (z-order, lost windows).
//! - Global window events, debounced: occlusion / fullscreen re-check.
//! - Power-setting and session notifications: pause reasons.
//! - Events posted from the video thread.

mod ffi;
mod pause;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::desktop::{self, Wallpaper};
use crate::playback::Player;
use crate::{log, occlusion, power};
use ffi::{Event, ExplorerHook, Host, WindowHooks};
use pause::PauseReasons;

pub struct Options {
    /// Video to play; `None` attaches without playback.
    pub video: Option<PathBuf>,
    /// Also pause while running on battery (savers always pause).
    pub pause_on_battery: bool,
}

pub fn run(options: Options) -> windows::core::Result<()> {
    let host = Host::create()?;
    host.close_on_ctrl_c();

    let mut wallpaper = Wallpaper::default();
    let mut hook = hook_explorer();
    let mut last = wallpaper.attach();
    log!("attach: {last:?}");

    let mut player = match &options.video {
        Some(path) => match open_player(&host, path) {
            Ok(p) => Some(p),
            Err(e) => {
                log!("playback unavailable: {e}");
                None
            }
        },
        None => None,
    };
    if let Some(p) = player.as_mut() {
        p.set_surfaces(wallpaper.surfaces());
    }

    // The pause policy only matters while there is something to pause.
    let mut reasons = PauseReasons {
        pause_on_battery: options.pause_on_battery,
        remote: power::is_remote_session(),
        ..Default::default()
    };
    let mut checks = CheckStats::default();
    let watch = player.is_some().then(|| {
        if power::enable_eco_qos() {
            log!("EcoQoS enabled");
        }
        let hooks = WindowHooks::install();
        log!("pause policy: {} window event hooks", hooks.count());
        // Registration makes Windows send the current power state at once.
        (hooks, power::Registration::new(host.hwnd()))
    });
    if watch.is_some() {
        check_windows(&wallpaper, &mut reasons, &mut checks);
        apply(&reasons, player.as_mut());
    }

    ffi::run_loop(|event| {
        let status = match event {
            Event::TaskbarCreated => {
                log!("Explorer restarted");
                hook = hook_explorer();
                Some(wallpaper.attach())
            }
            Event::DisplayChanged => {
                log!("display configuration changed");
                Some(wallpaper.attach())
            }
            Event::DesktopChanged => wallpaper.on_desktop_changed(),
            Event::Media { event, param } => {
                if let Some(p) = player.as_mut() {
                    p.on_event(event, param);
                }
                None
            }
            Event::WindowsSettled => {
                check_windows(&wallpaper, &mut reasons, &mut checks);
                None
            }
            Event::Power(change) => {
                log!("power: {change:?}");
                reasons.apply_power(change);
                None
            }
            Event::Session(change) => {
                log!("session: {change:?}");
                reasons.apply_session(change);
                reasons.remote = power::is_remote_session();
                None
            }
        };
        if let Some(status) = status {
            if status != last || event != Event::DesktopChanged {
                log!("attach: {status:?}");
                last = status;
            }
            // New wallpaper windows need new composition targets.
            if let Some(p) = player.as_mut() {
                p.set_surfaces(wallpaper.surfaces());
            }
            // The monitor layout may have changed what is covered.
            if watch.is_some() {
                check_windows(&wallpaper, &mut reasons, &mut checks);
            }
        }
        apply(&reasons, player.as_mut());
    });

    log!("message loop ended; shutting down");
    drop(player);
    drop(watch);
    drop(hook);
    wallpaper.detach();
    let (received, used) = ffi::window_event_counts();
    log!(
        "exit; Explorer window events: {}; global window events: {received} ({used} top-level); \
         occlusion checks: {} taking {:.2} ms total, {:.3} ms max",
        ffi::win_event_count(),
        checks.count,
        checks.total.as_secs_f64() * 1000.0,
        checks.max.as_secs_f64() * 1000.0,
    );
    Ok(())
}

#[derive(Default)]
struct CheckStats {
    count: u32,
    total: Duration,
    max: Duration,
}

/// Re-reads what covers the wallpaper and whether a fullscreen app runs.
fn check_windows(wallpaper: &Wallpaper, reasons: &mut PauseReasons, stats: &mut CheckStats) {
    let started = Instant::now();
    reasons.covered = occlusion::all_covered(&wallpaper.monitors(), &occlusion::occluders());
    reasons.fullscreen = occlusion::fullscreen_app();
    let took = started.elapsed();
    stats.count += 1;
    stats.total += took;
    stats.max = stats.max.max(took);
}

/// Pauses or resumes playback to match `reasons`, logging why on change.
fn apply(reasons: &PauseReasons, player: Option<&mut Player>) {
    let Some(player) = player else { return };
    let paused = reasons.paused();
    if paused != player.is_paused() {
        if paused {
            log!("pause: {}", reasons.active().join(", "));
        }
        player.set_paused(paused);
    }
}

fn open_player(host: &Host, path: &std::path::Path) -> windows::core::Result<Player> {
    let mut player = Player::new(host.hwnd())?;
    log!("playback: opening {}", path.display());
    player.open(path);
    Ok(player)
}

fn hook_explorer() -> Option<ExplorerHook> {
    let pid = desktop::explorer_pid();
    let hook = pid.and_then(ExplorerHook::install);
    match (pid, &hook) {
        (Some(pid), Some(_)) => log!("watching Explorer pid {pid}"),
        (Some(pid), None) => log!("could not hook Explorer pid {pid}"),
        (None, _) => log!("Explorer not found; waiting for TaskbarCreated"),
    }
    hook
}
