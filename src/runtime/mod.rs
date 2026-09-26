//! Event wiring: the hidden host window, Explorer event subscription, and
//! the reactions that keep the wallpaper attached.
//!
//! Wakes only for OS notifications:
//! - `TaskbarCreated` (Explorer restart): re-hook the new Explorer, re-attach.
//! - `WM_DISPLAYCHANGE`: re-attach to the new monitor layout.
//! - Explorer window events: cheap re-check (z-order, lost windows).

mod ffi;

use crate::desktop::{self, Wallpaper};
use crate::log;
use ffi::{Event, ExplorerHook, Host};

pub fn run() -> windows::core::Result<()> {
    let host = Host::create()?;
    host.close_on_ctrl_c();

    let mut wallpaper = Wallpaper::default();
    let mut hook = hook_explorer();
    let mut last = wallpaper.attach();
    log!("attach: {last:?}");

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
        };
        if let Some(status) = status
            && (status != last || event != Event::DesktopChanged)
        {
            log!("attach: {status:?}");
            last = status;
        }
    });

    drop(hook);
    wallpaper.detach();
    log!(
        "exit; Explorer window events received: {}",
        ffi::win_event_count()
    );
    Ok(())
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
