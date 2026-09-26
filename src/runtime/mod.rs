//! Event wiring: the hidden host window, Explorer event subscription, and
//! the reactions that keep the wallpaper attached and playing.
//!
//! Wakes only for OS notifications:
//! - `TaskbarCreated` (Explorer restart): re-hook the new Explorer, re-attach.
//! - `WM_DISPLAYCHANGE`: re-attach to the new monitor layout.
//! - Explorer window events: cheap re-check (z-order, lost windows).
//! - Media Engine events posted from MF threads.

mod ffi;

use std::path::PathBuf;

use crate::desktop::{self, Wallpaper};
use crate::log;
use crate::playback::Player;
use ffi::{Event, ExplorerHook, Host};

pub struct Options {
    /// Video to play; `None` attaches without playback.
    pub video: Option<PathBuf>,
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
        }
    });

    log!("message loop ended; shutting down");
    drop(player);
    drop(hook);
    wallpaper.detach();
    log!(
        "exit; Explorer window events received: {}",
        ffi::win_event_count()
    );
    Ok(())
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
