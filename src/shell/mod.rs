//! Process shell: tray icon and menu, Start with Windows, single instance,
//! console attach, the video picker (run in a child process) and the job
//! object that ties child processes to this one.

mod ffi;

pub use ffi::{
    Autostart, ChildJob, Icon, SingleInstance, Tray, allow_foreground, attach_parent_console,
    close_running, pick_video, send_to_running, show_menu,
};

/// Tray menu commands (menu item ids).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Command {
    Choose = 1,
    Pause = 2,
    PauseOnBattery = 3,
    Autostart = 4,
    Quit = 5,
}

impl Command {
    pub fn from_id(id: u32) -> Option<Self> {
        [
            Self::Choose,
            Self::Pause,
            Self::PauseOnBattery,
            Self::Autostart,
            Self::Quit,
        ]
        .into_iter()
        .find(|c| *c as u32 == id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Item {
        command: Command,
        label: &'static str,
        checked: bool,
        enabled: bool,
    },
    Separator,
}

/// What the menu shows; everything the menu depends on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MenuState {
    pub paused: bool,
    pub pause_on_battery: bool,
    pub autostart: bool,
    /// A pick or import is running; choosing again is disabled.
    pub busy: bool,
}

pub fn menu(state: MenuState) -> Vec<MenuItem> {
    let item = |command, label, checked, enabled| MenuItem::Item {
        command,
        label,
        checked,
        enabled,
    };
    vec![
        item(
            Command::Choose,
            if state.busy {
                "Importing video\u{2026}"
            } else {
                "Choose video\u{2026}"
            },
            false,
            !state.busy,
        ),
        item(Command::Pause, "Pause", state.paused, true),
        MenuItem::Separator,
        item(
            Command::PauseOnBattery,
            "Pause on battery",
            state.pause_on_battery,
            true,
        ),
        item(
            Command::Autostart,
            "Start with Windows",
            state.autostart,
            true,
        ),
        MenuItem::Separator,
        item(Command::Quit, "Quit", false, true),
    ]
}

/// Tray tooltip, within the shell's 127-character limit.
pub fn tooltip(status: &str) -> String {
    let mut text = format!("Wallive \u{2013} {status}");
    if text.chars().count() > 127 {
        text = text.chars().take(126).collect::<String>() + "\u{2026}";
    }
    text
}

/// Tray icon pixels, `size` x `size`, straight-alpha BGRA as `0xAARRGGBB`,
/// top row first: a rounded square with a teal-to-blue gradient and a white
/// play triangle. Drawn at run time so the binary needs no icon resource
/// tooling; 4x4 supersampling for smooth edges.
pub fn icon_pixels(size: u32) -> Vec<u32> {
    let s = size.max(1) as f32;
    let radius = s * 0.22;
    let inside_square = |x: f32, y: f32| {
        let cx = x.clamp(radius, s - radius);
        let cy = y.clamp(radius, s - radius);
        (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius
    };
    // Play triangle, optically centred (shifted right a little).
    let (ax, ay) = (s * 0.38, s * 0.28);
    let (bx, by) = (s * 0.38, s * 0.72);
    let (cx, cy) = (s * 0.74, s * 0.5);
    let edge = |px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32| {
        (x1 - x0) * (py - y0) - (y1 - y0) * (px - x0)
    };
    let inside_triangle = |x: f32, y: f32| {
        let d0 = edge(x, y, ax, ay, bx, by);
        let d1 = edge(x, y, bx, by, cx, cy);
        let d2 = edge(x, y, cx, cy, ax, ay);
        (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0) || (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0)
    };

    const N: u32 = 4;
    let mut out = Vec::with_capacity((size * size) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut cover, mut white) = (0u32, 0u32);
            for sy in 0..N {
                for sx in 0..N {
                    let x = px as f32 + (sx as f32 + 0.5) / N as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / N as f32;
                    if inside_square(x, y) {
                        cover += 1;
                        if inside_triangle(x, y) {
                            white += 1;
                        }
                    }
                }
            }
            if cover == 0 {
                out.push(0);
                continue;
            }
            // Gradient from teal (top left) to blue (bottom right).
            let t = (px + py) as f32 / (2.0 * s);
            let lerp = |a: f32, b: f32| a + (b - a) * t;
            let w = white as f32 / cover as f32;
            let mix = |c: f32| (c + (255.0 - c) * w).round() as u32;
            let (r, g, b) = (
                mix(lerp(0.0, 40.0)),
                mix(lerp(180.0, 90.0)),
                mix(lerp(170.0, 220.0)),
            );
            let a = (255 * cover / (N * N)).min(255);
            out.push((a << 24) | (r << 16) | (g << 8) | b);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_ids_round_trip() {
        for c in [
            Command::Choose,
            Command::Pause,
            Command::PauseOnBattery,
            Command::Autostart,
            Command::Quit,
        ] {
            assert_eq!(Command::from_id(c as u32), Some(c));
        }
        assert_eq!(Command::from_id(0), None);
    }

    #[test]
    fn menu_reflects_state() {
        let items = menu(MenuState {
            paused: true,
            busy: true,
            ..Default::default()
        });
        let find = |cmd| {
            items.iter().find_map(|i| match i {
                MenuItem::Item {
                    command,
                    checked,
                    enabled,
                    ..
                } if *command == cmd => Some((*checked, *enabled)),
                _ => None,
            })
        };
        assert_eq!(find(Command::Pause), Some((true, true)));
        assert_eq!(find(Command::Choose), Some((false, false)));
        assert_eq!(find(Command::Autostart), Some((false, true)));
        assert_eq!(find(Command::Quit), Some((false, true)));
    }

    #[test]
    fn tooltip_is_capped() {
        assert_eq!(tooltip("playing"), "Wallive \u{2013} playing");
        let long = tooltip(&"x".repeat(300));
        assert_eq!(long.chars().count(), 127);
        assert!(long.ends_with('\u{2026}'));
    }

    #[test]
    fn icon_has_transparent_corners_and_white_centre() {
        let size = 32;
        let px = icon_pixels(size);
        assert_eq!(px.len(), 32 * 32);
        assert_eq!(px[0] >> 24, 0, "corner must be transparent");
        let centre = px[(16 * size + 16) as usize];
        assert_eq!(centre >> 24, 255);
        assert_eq!(centre & 0x00ff_ffff, 0x00ff_ffff, "triangle is white");
        let edge = px[(16 * size + 3) as usize];
        assert_eq!(edge >> 24, 255);
        assert_ne!(edge & 0x00ff_ffff, 0x00ff_ffff, "background is coloured");
    }
}
