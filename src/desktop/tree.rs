//! Deciding where the wallpaper goes in Explorer's desktop window tree.
//!
//! Pure logic over a [`Snapshot`], so it is unit-testable on any machine. `W` is
//! an `HWND` at runtime and a plain integer in the tests. See ADR-006.
//!
//! Two layouts exist:
//!
//! - **Classic** (Windows 10, Windows 11 before 24H2): message `0x052C` makes
//!   Progman spawn top-level `WorkerW` windows. One holds `SHELLDLL_DefView`
//!   (the icons); the next `WorkerW` behind it in z-order is empty and is where
//!   the wallpaper goes.
//! - **Raised** (Windows 11 24H2+): Progman has `WS_EX_NOREDIRECTIONBITMAP`,
//!   `SHELLDLL_DefView` stays a direct child of Progman, and `0x052C` makes a
//!   `WorkerW` *child* of Progman that draws Explorer's own wallpaper. Ours is a
//!   layered child of Progman placed below the icons and above that `WorkerW`.

/// Class of the window that holds the desktop icons.
pub const ICONS_CLASS: &str = "SHELLDLL_DefView";

/// Class of the windows Progman creates in answer to `0x052C`.
pub const WORKER_CLASS: &str = "WorkerW";

/// One window, reduced to what the layout decision needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window<W> {
    pub handle: W,
    pub class: String,
    /// Has a direct `SHELLDLL_DefView` child.
    pub holds_icons: bool,
}

/// Explorer's desktop windows at one moment.
#[derive(Clone, Debug)]
pub struct Snapshot<W> {
    pub progman: W,
    /// Progman carries `WS_EX_NOREDIRECTIONBITMAP`. This marks the raised
    /// desktop even before Explorer has created its wallpaper `WorkerW`.
    pub progman_no_redirection: bool,
    /// Progman's direct children, topmost first.
    pub progman_children: Vec<Window<W>>,
    /// Top-level `WorkerW` windows, topmost first.
    pub top_level_workers: Vec<Window<W>>,
}

/// Where wallpaper windows are parented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout<W> {
    /// Classic: children of the empty `WorkerW` behind the icons.
    Classic { worker: W },
    /// Raised: layered children of Progman, below `icons` and above `layer`
    /// (Explorer's wallpaper `WorkerW`). `icons` is `None` if the user hid the
    /// desktop icons window; `layer` is `None` until Explorer creates it.
    Raised {
        progman: W,
        icons: Option<W>,
        layer: Option<W>,
    },
    /// Classic desktop that has not been split yet. There is no slot behind the
    /// icons, so nothing is attached until Explorer answers `0x052C`.
    Unsplit { progman: W },
}

impl<W: Copy> Layout<W> {
    /// Whether sending `0x052C` could still improve the result.
    pub fn needs_split(&self) -> bool {
        matches!(
            self,
            Self::Unsplit { .. } | Self::Raised { layer: None, .. }
        )
    }

    /// Parent for wallpaper windows, or `None` when there is no safe slot.
    pub fn parent(&self) -> Option<W> {
        match *self {
            Self::Classic { worker } => Some(worker),
            Self::Raised { progman, .. } => Some(progman),
            Self::Unsplit { .. } => None,
        }
    }
}

/// Picks the layout from a snapshot.
pub fn detect<W: Copy + Eq>(snapshot: &Snapshot<W>) -> Layout<W> {
    let (icons, layer) = raised_parts(&snapshot.progman_children);

    // Icons and a WorkerW both inside Progman is the raised layout even on
    // early 24H2 builds that did not set the style bit yet.
    if snapshot.progman_no_redirection || (icons.is_some() && layer.is_some()) {
        return Layout::Raised {
            progman: snapshot.progman,
            icons,
            layer,
        };
    }

    // Only the WorkerW holding the icons anchors the search. Any other window
    // with a DefView (an old-style file dialog) is not the desktop.
    let workers = &snapshot.top_level_workers;
    if let Some(icons_at) = workers.iter().position(|w| w.holds_icons)
        && let Some(behind) = workers[icons_at + 1..].iter().find(|w| !w.holds_icons)
    {
        return Layout::Classic {
            worker: behind.handle,
        };
    }

    Layout::Unsplit {
        progman: snapshot.progman,
    }
}

/// First `SHELLDLL_DefView` and first `WorkerW` among Progman's children.
pub fn raised_parts<W: Copy>(children: &[Window<W>]) -> (Option<W>, Option<W>) {
    let first = |class: &str| children.iter().find(|w| w.class == class).map(|w| w.handle);
    (first(ICONS_CLASS), first(WORKER_CLASS))
}

/// Z-order corrections needed on the raised desktop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZFix<W> {
    /// Our windows that must be moved to directly below the icons (or to the
    /// top of Progman's children when there are no icons).
    pub raise: Vec<W>,
    /// Explorer's layer is above the icons and must go to the bottom.
    pub sink_layer: bool,
}

impl<W> ZFix<W> {
    pub fn is_needed(&self) -> bool {
        !self.raise.is_empty() || self.sink_layer
    }
}

/// Compares Progman's children (topmost first) with the target order
/// `icons > ours > layer` and returns only the moves that are needed, so a
/// correct desktop is never touched.
pub fn z_fix<W: Copy + Eq>(
    children: &[W],
    ours: &[W],
    icons: Option<W>,
    layer: Option<W>,
) -> ZFix<W> {
    let index = |w: W| children.iter().position(|c| *c == w);
    let icons_at = icons.and_then(index);
    let layer_at = layer.and_then(index);

    let sink_layer = matches!((icons_at, layer_at), (Some(i), Some(l)) if l < i);
    let raise = ours
        .iter()
        .copied()
        .filter(|&w| match index(w) {
            None => true,
            Some(at) => {
                let above_icons = icons_at.is_some_and(|i| at < i);
                // A layer about to be sunk ends up below everything anyway.
                let below_layer = !sink_layer && layer_at.is_some_and(|l| at > l);
                above_icons || below_layer
            }
        })
        .collect();

    ZFix { raise, sink_layer }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(handle: u32, class: &str, holds_icons: bool) -> Window<u32> {
        Window {
            handle,
            class: class.to_owned(),
            holds_icons,
        }
    }

    fn snap(flag: bool, children: Vec<Window<u32>>, workers: Vec<Window<u32>>) -> Snapshot<u32> {
        Snapshot {
            progman: 1,
            progman_no_redirection: flag,
            progman_children: children,
            top_level_workers: workers,
        }
    }

    #[test]
    fn classic_split_uses_worker_behind_icons() {
        let s = snap(
            false,
            vec![],
            vec![win(10, WORKER_CLASS, true), win(11, WORKER_CLASS, false)],
        );
        assert_eq!(detect(&s), Layout::Classic { worker: 11 });
    }

    #[test]
    fn classic_ignores_empty_worker_above_icons() {
        let s = snap(
            false,
            vec![],
            vec![
                win(9, WORKER_CLASS, false),
                win(10, WORKER_CLASS, true),
                win(11, WORKER_CLASS, false),
            ],
        );
        assert_eq!(detect(&s), Layout::Classic { worker: 11 });
    }

    #[test]
    fn classic_unsplit_waits_and_has_no_parent() {
        let s = snap(false, vec![win(20, ICONS_CLASS, false)], vec![]);
        let layout = detect(&s);
        assert_eq!(layout, Layout::Unsplit { progman: 1 });
        assert!(layout.needs_split());
        assert_eq!(layout.parent(), None);
    }

    #[test]
    fn classic_icons_worker_alone_is_unsplit() {
        let s = snap(false, vec![], vec![win(10, WORKER_CLASS, true)]);
        assert_eq!(detect(&s), Layout::Unsplit { progman: 1 });
    }

    #[test]
    fn raised_with_layer() {
        let s = snap(
            true,
            vec![win(20, ICONS_CLASS, false), win(21, WORKER_CLASS, false)],
            vec![],
        );
        let layout = detect(&s);
        assert_eq!(
            layout,
            Layout::Raised {
                progman: 1,
                icons: Some(20),
                layer: Some(21)
            }
        );
        assert!(!layout.needs_split());
        assert_eq!(layout.parent(), Some(1));
    }

    #[test]
    fn raised_before_layer_exists_still_raised_but_wants_split() {
        let s = snap(true, vec![win(20, ICONS_CLASS, false)], vec![]);
        let layout = detect(&s);
        assert!(matches!(layout, Layout::Raised { layer: None, .. }));
        assert!(layout.needs_split());
    }

    #[test]
    fn raised_detected_without_style_bit() {
        let s = snap(
            false,
            vec![win(21, WORKER_CLASS, false), win(20, ICONS_CLASS, false)],
            vec![],
        );
        assert!(matches!(detect(&s), Layout::Raised { .. }));
    }

    #[test]
    fn style_bit_beats_stale_classic_workers() {
        let s = snap(
            true,
            vec![win(20, ICONS_CLASS, false)],
            vec![win(10, WORKER_CLASS, true), win(11, WORKER_CLASS, false)],
        );
        assert!(matches!(detect(&s), Layout::Raised { .. }));
    }

    #[test]
    fn raised_with_icons_hidden() {
        let s = snap(true, vec![win(21, WORKER_CLASS, false)], vec![]);
        assert_eq!(
            detect(&s),
            Layout::Raised {
                progman: 1,
                icons: None,
                layer: Some(21)
            }
        );
    }

    #[test]
    fn correct_order_needs_nothing() {
        let fix = z_fix(&[20, 30, 31, 21], &[30, 31], Some(20), Some(21));
        assert!(!fix.is_needed(), "{fix:?}");
    }

    #[test]
    fn ours_above_icons_is_raised() {
        let fix = z_fix(&[30, 20, 21], &[30], Some(20), Some(21));
        assert_eq!(fix.raise, vec![30]);
        assert!(!fix.sink_layer);
    }

    #[test]
    fn ours_below_layer_is_raised() {
        let fix = z_fix(&[20, 21, 30], &[30], Some(20), Some(21));
        assert_eq!(fix.raise, vec![30]);
        assert!(!fix.sink_layer);
    }

    #[test]
    fn layer_recreated_on_top_is_sunk_only() {
        let fix = z_fix(&[21, 20, 30], &[30], Some(20), Some(21));
        assert!(fix.raise.is_empty());
        assert!(fix.sink_layer);
    }

    #[test]
    fn layer_on_top_and_ours_above_icons_both_move() {
        let fix = z_fix(&[21, 30, 20], &[30], Some(20), Some(21));
        assert_eq!(fix.raise, vec![30]);
        assert!(fix.sink_layer);
    }

    #[test]
    fn ours_missing_from_progman_is_raised() {
        let fix = z_fix(&[20, 21], &[30], Some(20), Some(21));
        assert_eq!(fix.raise, vec![30]);
    }

    #[test]
    fn no_icons_only_needs_to_be_above_layer() {
        assert!(!z_fix(&[30, 21], &[30], None, Some(21)).is_needed());
        assert_eq!(z_fix(&[21, 30], &[30], None, Some(21)).raise, vec![30]);
    }

    #[test]
    fn no_layer_only_needs_to_be_below_icons() {
        assert!(!z_fix(&[20, 30], &[30], Some(20), None).is_needed());
        assert_eq!(z_fix(&[30, 20], &[30], Some(20), None).raise, vec![30]);
    }
}
