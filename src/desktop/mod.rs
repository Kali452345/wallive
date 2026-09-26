//! Attaching one wallpaper window per monitor behind the desktop icons, on
//! both the classic WorkerW layout and the Windows 11 24H2+ raised desktop.
//!
//! Knows nothing about video: it owns the windows and exposes one surface
//! `HWND` per monitor. Re-checks are driven by Explorer window events (see
//! `runtime`), never by a timer.

mod ffi;
pub mod tree;

use windows::Win32::Foundation::{HWND, RECT};

use tree::Layout;

/// One monitor's windows.
#[derive(Clone, Copy, Debug)]
struct Screen {
    /// Layered holder inside Progman (raised layout only).
    holder: Option<HWND>,
    /// The window that will carry the video.
    surface: HWND,
    /// Monitor rectangle in screen coordinates.
    monitor: RECT,
}

impl Screen {
    /// The window that is a direct child of the desktop parent.
    fn outer(&self) -> HWND {
        self.holder.unwrap_or(self.surface)
    }

    fn alive(&self) -> bool {
        ffi::is_alive(self.surface) && self.holder.is_none_or(ffi::is_alive)
    }

    fn destroy(self) {
        // Destroying the holder destroys the surface inside it too.
        ffi::destroy(self.outer());
    }
}

/// What an attach attempt produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Attached {
        layout: Kind,
        monitors: usize,
    },
    /// No safe slot yet; a later Explorer event will retry.
    Waiting(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Classic,
    Raised,
    /// Raised layout, but Explorer has not created its wallpaper layer yet.
    RaisedNoLayer,
}

#[derive(Default)]
pub struct Wallpaper {
    layout: Option<Layout<HWND>>,
    screens: Vec<Screen>,
    /// Progman we already sent `0x052C` to, so events do not resend it.
    spawn_requested: Option<HWND>,
}

/// Explorer's process id, used to scope the WinEvent hook.
pub fn explorer_pid() -> Option<u32> {
    ffi::find_progman()
        .or_else(ffi::find_taskbar)
        .and_then(ffi::process_id)
}

impl Wallpaper {
    /// Tears down any existing windows and attaches afresh.
    pub fn attach(&mut self) -> Status {
        self.detach();

        let Some(progman) = ffi::find_progman() else {
            return Status::Waiting("Progman not found (Explorer not running?)");
        };
        let mut layout = tree::detect(&ffi::snapshot(progman));
        if layout.needs_split() && self.spawn_requested != Some(progman) {
            ffi::request_worker(progman);
            self.spawn_requested = Some(progman);
            layout = tree::detect(&ffi::snapshot(progman));
        }

        let (parent, kind) = match layout {
            Layout::Classic { worker } => (worker, Kind::Classic),
            Layout::Raised { progman, layer, .. } => (
                progman,
                if layer.is_some() {
                    Kind::Raised
                } else {
                    Kind::RaisedNoLayer
                },
            ),
            Layout::Unsplit { .. } => return Status::Waiting("classic desktop not split yet"),
        };
        let raised = kind != Kind::Classic;

        for monitor in ffi::monitor_rects() {
            match create_screen(parent, monitor, raised) {
                Some(screen) => self.screens.push(screen),
                None => crate::log!("could not create wallpaper window for {monitor:?}"),
            }
        }
        if self.screens.is_empty() {
            return Status::Waiting("no wallpaper windows could be created");
        }

        self.layout = Some(layout);
        self.fix_z_order();
        for screen in &self.screens {
            if let Some(holder) = screen.holder {
                ffi::show(holder);
            }
            ffi::show(screen.surface);
        }

        Status::Attached {
            layout: kind,
            monitors: self.screens.len(),
        }
    }

    /// Called after Explorer created, destroyed, showed, hid or reordered a
    /// window. Cheap when nothing relevant changed. A raised desktop whose
    /// layer appears later is handled by the z-order fix, not a re-attach.
    pub fn on_desktop_changed(&mut self) -> Option<Status> {
        let broken = self.layout.is_none()
            || self
                .screens
                .iter()
                .any(|s| !s.alive() || !self.parent_ok(s));
        if broken {
            return Some(self.attach());
        }
        if self.fix_z_order() {
            crate::log!("restacked wallpaper windows under the icons");
        }
        None
    }

    /// The video windows, one per monitor, with their sizes.
    pub fn surfaces(&self) -> Vec<crate::playback::Surface> {
        self.screens
            .iter()
            .map(|s| crate::playback::Surface {
                hwnd: s.surface,
                width: (s.monitor.right - s.monitor.left).max(1) as u32,
                height: (s.monitor.bottom - s.monitor.top).max(1) as u32,
            })
            .collect()
    }

    /// Monitor rectangles that carry a wallpaper window, in screen pixels.
    pub fn monitors(&self) -> Vec<crate::occlusion::Rect> {
        self.screens
            .iter()
            .map(|s| crate::occlusion::Rect {
                left: s.monitor.left,
                top: s.monitor.top,
                right: s.monitor.right,
                bottom: s.monitor.bottom,
            })
            .collect()
    }

    pub fn detach(&mut self) {
        for screen in self.screens.drain(..) {
            screen.destroy();
        }
        self.layout = None;
    }

    fn parent_ok(&self, screen: &Screen) -> bool {
        self.layout
            .and_then(|l| l.parent())
            .is_some_and(|p| ffi::parent_of(screen.outer()) == Some(p))
    }

    /// Raised layout only: keep ours below the icons and above Explorer's
    /// layer, moving only what is out of place. Returns whether anything moved.
    fn fix_z_order(&mut self) -> bool {
        let Some(Layout::Raised {
            progman,
            icons,
            layer,
        }) = &mut self.layout
        else {
            return false;
        };
        // Explorer recreates its layer on wallpaper/slideshow changes, so
        // re-read the children every time.
        let handles = ffi::children(*progman);
        let described: Vec<tree::Window<HWND>> = handles
            .iter()
            .map(|&h| tree::Window {
                handle: h,
                class: ffi::class_of(h),
                holds_icons: false,
            })
            .collect();
        (*icons, *layer) = tree::raised_parts(&described);

        let ours: Vec<HWND> = self.screens.iter().map(Screen::outer).collect();
        let fix = tree::z_fix(&handles, &ours, *icons, *layer);
        if fix.sink_layer
            && let Some(layer) = *layer
        {
            ffi::send_to_bottom(layer);
        }
        for window in &fix.raise {
            ffi::place_below(*window, *icons);
        }
        fix.is_needed()
    }
}

impl Drop for Wallpaper {
    fn drop(&mut self) {
        self.detach();
    }
}

fn create_screen(parent: HWND, monitor: RECT, raised: bool) -> Option<Screen> {
    let at = ffi::screen_to_client(parent, monitor);
    if raised {
        if let Some(holder) = ffi::create_layered_holder(parent, at) {
            let inner = RECT {
                left: 0,
                top: 0,
                right: at.right - at.left,
                bottom: at.bottom - at.top,
            };
            return match ffi::create_surface(holder, inner) {
                Ok(surface) => Some(Screen {
                    holder: Some(holder),
                    surface,
                    monitor,
                }),
                Err(e) => {
                    crate::log!("surface window failed: {e}");
                    ffi::destroy(holder);
                    None
                }
            };
        }
        crate::log!("layered holder refused; attaching surface directly to Progman");
    }
    ffi::create_surface(parent, at)
        .map_err(|e| crate::log!("surface window failed: {e}"))
        .ok()
        .map(|surface| Screen {
            holder: None,
            surface,
            monitor,
        })
}
