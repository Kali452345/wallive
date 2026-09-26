//! Is the wallpaper hidden? (ADR-005)
//!
//! Every visible top-level window sits above the wallpaper, so z-order among
//! them does not matter: a monitor is covered when the union of the windows
//! that qualify as occluders covers enough of it. The check runs only after
//! window events settle (see `runtime`), never on a timer.

mod ffi;

pub use ffi::{fullscreen_app, occluders};

/// Screen rectangle, right/bottom exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn area(&self) -> i64 {
        i64::from((self.right - self.left).max(0)) * i64::from((self.bottom - self.top).max(0))
    }

    fn clip(&self, to: &Rect) -> Option<Rect> {
        let r = Rect {
            left: self.left.max(to.left),
            top: self.top.max(to.top),
            right: self.right.min(to.right),
            bottom: self.bottom.min(to.bottom),
        };
        (r.left < r.right && r.top < r.bottom).then_some(r)
    }
}

/// Fraction of a monitor that must be covered before it counts as hidden.
/// Just below 1 so a thin gap (taskbar auto-hide strip, rounding in window
/// bounds) does not keep the video running.
pub const THRESHOLD: f64 = 0.95;

/// What a top-level window looks like to the occlusion check.
#[derive(Clone, Debug, Default)]
pub struct WindowInfo {
    pub rect: Rect,
    pub visible: bool,
    pub minimized: bool,
    /// Cloaked by DWM: on another virtual desktop, or a suspended UWP app.
    pub cloaked: bool,
    /// `WS_EX_TRANSPARENT` + `WS_EX_LAYERED`: a click-through overlay that the
    /// wallpaper shows through.
    pub click_through: bool,
    /// `WS_EX_TOOLWINDOW`: palettes, popups, notification toasts.
    pub tool: bool,
    pub class: String,
}

/// Desktop windows that are top-level but are the wallpaper's own backdrop.
const DESKTOP_CLASSES: [&str; 2] = ["Progman", "WorkerW"];
/// Tool windows that still hide the wallpaper completely.
const OPAQUE_TOOL_CLASSES: [&str; 2] = ["Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

/// Whether `w` hides whatever is below it.
pub fn is_occluder(w: &WindowInfo) -> bool {
    if !w.visible || w.minimized || w.cloaked || w.click_through || w.rect.area() == 0 {
        return false;
    }
    if DESKTOP_CLASSES.contains(&w.class.as_str()) {
        return false;
    }
    !w.tool || OPAQUE_TOOL_CLASSES.contains(&w.class.as_str())
}

/// Fraction of `monitor` covered by the union of `windows`, in 0..=1.
///
/// Coordinate compression: split the monitor into a grid on every window
/// edge, then add up the cells that some window covers. O(n^3) for n
/// windows, which is microseconds for the few dozen windows a desktop has.
pub fn covered_fraction(monitor: &Rect, windows: &[Rect]) -> f64 {
    let total = monitor.area();
    if total == 0 {
        return 1.0;
    }
    let clipped: Vec<Rect> = windows.iter().filter_map(|w| w.clip(monitor)).collect();
    if clipped.is_empty() {
        return 0.0;
    }
    let edges = |a: fn(&Rect) -> i32, b: fn(&Rect) -> i32| {
        let mut v: Vec<i32> = clipped.iter().flat_map(|r| [a(r), b(r)]).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let xs = edges(|r| r.left, |r| r.right);
    let ys = edges(|r| r.top, |r| r.bottom);
    let mut covered = 0i64;
    for x in xs.windows(2) {
        for y in ys.windows(2) {
            let hit = clipped
                .iter()
                .any(|r| r.left <= x[0] && x[1] <= r.right && r.top <= y[0] && y[1] <= r.bottom);
            if hit {
                covered += i64::from(x[1] - x[0]) * i64::from(y[1] - y[0]);
            }
        }
    }
    covered as f64 / total as f64
}

/// True when every monitor is at least [`THRESHOLD`] covered. No monitors
/// means nothing to show, which also counts as hidden.
pub fn all_covered(monitors: &[Rect], windows: &[WindowInfo]) -> bool {
    let rects: Vec<Rect> = windows
        .iter()
        .filter(|w| is_occluder(w))
        .map(|w| w.rect)
        .collect();
    monitors
        .iter()
        .all(|m| covered_fraction(m, &rects) >= THRESHOLD)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    fn win(rect: Rect) -> WindowInfo {
        WindowInfo {
            rect,
            visible: true,
            class: "Notepad".into(),
            ..Default::default()
        }
    }

    const FHD: Rect = Rect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };

    #[test]
    fn empty_desktop_is_uncovered() {
        assert_eq!(covered_fraction(&FHD, &[]), 0.0);
        assert!(!all_covered(&[FHD], &[]));
    }

    #[test]
    fn overlaps_are_counted_once() {
        let halves = [r(0, 0, 1200, 1080), r(720, 0, 1920, 1080)];
        assert_eq!(covered_fraction(&FHD, &halves), 1.0);
        let quarter = [r(0, 0, 960, 540), r(0, 0, 960, 540)];
        assert_eq!(covered_fraction(&FHD, &quarter), 0.25);
    }

    #[test]
    fn windows_are_clipped_to_the_monitor() {
        // A window hanging off the left edge covers only its on-screen part.
        assert_eq!(covered_fraction(&FHD, &[r(-960, 0, 960, 1080)]), 0.5);
        // Entirely on another monitor.
        assert_eq!(covered_fraction(&FHD, &[r(1920, 0, 3840, 1080)]), 0.0);
    }

    #[test]
    fn maximized_window_above_taskbar_counts_as_covered() {
        // 1080 - 48 px taskbar = 95.6 %.
        let maximized = win(r(0, 0, 1920, 1032));
        assert!(all_covered(&[FHD], std::slice::from_ref(&maximized)));
        // Snapped to half the screen: not covered.
        let half = win(r(0, 0, 960, 1032));
        assert!(!all_covered(&[FHD], &[half]));
    }

    #[test]
    fn every_monitor_must_be_covered() {
        let second = r(1920, 0, 4480, 1440);
        let maximized = win(r(0, 0, 1920, 1080));
        assert!(!all_covered(
            &[FHD, second],
            std::slice::from_ref(&maximized)
        ));
        let other = win(r(1920, 0, 4480, 1440));
        assert!(all_covered(&[FHD, second], &[maximized, other]));
    }

    #[test]
    fn hidden_and_see_through_windows_do_not_occlude() {
        let full = r(0, 0, 1920, 1080);
        let cases = [
            WindowInfo {
                visible: false,
                ..win(full)
            },
            WindowInfo {
                minimized: true,
                ..win(full)
            },
            WindowInfo {
                cloaked: true,
                ..win(full)
            },
            WindowInfo {
                click_through: true,
                ..win(full)
            },
            WindowInfo {
                tool: true,
                ..win(full)
            },
            WindowInfo {
                class: "Progman".into(),
                ..win(full)
            },
            WindowInfo {
                class: "WorkerW".into(),
                ..win(full)
            },
        ];
        for w in cases {
            assert!(!is_occluder(&w), "{w:?}");
        }
    }

    #[test]
    fn taskbar_is_an_opaque_tool_window() {
        let taskbar = WindowInfo {
            tool: true,
            class: "Shell_TrayWnd".into(),
            ..win(r(0, 1032, 1920, 1080))
        };
        assert!(is_occluder(&taskbar));
        let two_halves = [win(r(0, 0, 960, 1032)), win(r(960, 0, 1920, 1032)), taskbar];
        assert!(all_covered(&[FHD], &two_halves));
    }

    #[test]
    fn no_monitors_counts_as_covered() {
        assert!(all_covered(&[], &[]));
    }
}
