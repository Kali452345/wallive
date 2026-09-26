//! Process shell: tray icon and menu, Start with Windows, single instance,
//! console attach, the video picker (run in a child process) and the job
//! object that ties child processes to this one.

mod ffi;

pub use ffi::{
    Autostart, ChildJob, Icon, SingleInstance, Tray, allow_foreground, attach_parent_console,
    close_running, pick_videos, send_to_running, show_menu,
};

/// Tray menu commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Choose,
    Next,
    /// Minutes between videos.
    SwitchEvery(u32),
    Shuffle,
    Pause,
    PauseOnBattery,
    Autostart,
    Quit,
}

const SWITCH_ID_BASE: u32 = 1000;

impl Command {
    /// Menu item id (non-zero; `TrackPopupMenuEx` returns 0 for "none").
    pub fn id(self) -> u32 {
        match self {
            Self::Choose => 1,
            Self::Pause => 2,
            Self::PauseOnBattery => 3,
            Self::Autostart => 4,
            Self::Quit => 5,
            Self::Next => 6,
            Self::Shuffle => 7,
            Self::SwitchEvery(minutes) => SWITCH_ID_BASE + minutes,
        }
    }

    pub fn from_id(id: u32) -> Option<Self> {
        if id > SWITCH_ID_BASE {
            return Some(Self::SwitchEvery(id - SWITCH_ID_BASE));
        }
        [
            Self::Choose,
            Self::Pause,
            Self::PauseOnBattery,
            Self::Autostart,
            Self::Quit,
            Self::Next,
            Self::Shuffle,
        ]
        .into_iter()
        .find(|c| c.id() == id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Item {
        command: Command,
        label: String,
        checked: bool,
        enabled: bool,
    },
    Submenu {
        label: String,
        items: Vec<MenuItem>,
    },
    Separator,
}

/// What the menu shows; everything the menu depends on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MenuState {
    pub paused: bool,
    pub pause_on_battery: bool,
    pub autostart: bool,
    /// The file dialog is open; choosing again is disabled.
    pub picking: bool,
    /// How many videos take turns (0 or 1: no playlist items).
    pub videos: usize,
    pub switch_minutes: u32,
    pub shuffle: bool,
}

fn minutes_label(m: u32) -> String {
    match m {
        1 => "1 minute".into(),
        60 => "1 hour".into(),
        m if m % 60 == 0 => format!("{} hours", m / 60),
        m => format!("{m} minutes"),
    }
}

pub fn menu(state: MenuState, switch_choices: &[u32]) -> Vec<MenuItem> {
    let item = |command, label: &str, checked, enabled| MenuItem::Item {
        command,
        label: label.to_string(),
        checked,
        enabled,
    };
    let mut items = vec![item(
        Command::Choose,
        if state.picking {
            "Choosing videos\u{2026}"
        } else {
            "Choose videos\u{2026}"
        },
        false,
        !state.picking,
    )];
    if state.videos >= 2 {
        items.push(item(Command::Next, "Next video", false, true));
        let mut choices: Vec<u32> = switch_choices.to_vec();
        if !choices.contains(&state.switch_minutes) {
            // A value edited into config.txt still shows as the choice.
            choices.push(state.switch_minutes);
            choices.sort_unstable();
        }
        items.push(MenuItem::Submenu {
            label: "Switch every".into(),
            items: choices
                .into_iter()
                .map(|m| MenuItem::Item {
                    command: Command::SwitchEvery(m),
                    label: minutes_label(m),
                    checked: m == state.switch_minutes,
                    enabled: true,
                })
                .collect(),
        });
        items.push(item(Command::Shuffle, "Shuffle", state.shuffle, true));
    }
    items.extend([
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
    ]);
    items
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
            Command::Next,
            Command::SwitchEvery(1),
            Command::SwitchEvery(60),
            Command::Shuffle,
            Command::Pause,
            Command::PauseOnBattery,
            Command::Autostart,
            Command::Quit,
        ] {
            assert_ne!(c.id(), 0);
            assert_eq!(Command::from_id(c.id()), Some(c));
        }
        assert_eq!(Command::from_id(0), None);
        assert_eq!(Command::from_id(999), None);
    }

    fn find(items: &[MenuItem], cmd: Command) -> Option<(bool, bool)> {
        items.iter().find_map(|i| match i {
            MenuItem::Item {
                command,
                checked,
                enabled,
                ..
            } if *command == cmd => Some((*checked, *enabled)),
            MenuItem::Submenu { items, .. } => find(items, cmd),
            _ => None,
        })
    }

    #[test]
    fn menu_reflects_state() {
        let items = menu(
            MenuState {
                paused: true,
                picking: true,
                ..Default::default()
            },
            &[1, 5],
        );
        assert_eq!(find(&items, Command::Pause), Some((true, true)));
        assert_eq!(find(&items, Command::Choose), Some((false, false)));
        assert_eq!(find(&items, Command::Autostart), Some((false, true)));
        assert_eq!(find(&items, Command::Quit), Some((false, true)));
        // One video: no playlist items.
        assert_eq!(find(&items, Command::Next), None);
        assert_eq!(find(&items, Command::Shuffle), None);
    }

    #[test]
    fn playlist_items_with_several_videos() {
        let items = menu(
            MenuState {
                videos: 3,
                switch_minutes: 5,
                shuffle: true,
                ..Default::default()
            },
            &[1, 5, 15],
        );
        assert_eq!(find(&items, Command::Next), Some((false, true)));
        assert_eq!(find(&items, Command::Shuffle), Some((true, true)));
        assert_eq!(find(&items, Command::SwitchEvery(5)), Some((true, true)));
        assert_eq!(find(&items, Command::SwitchEvery(1)), Some((false, true)));
        // A hand-edited interval is listed and checked.
        let items = menu(
            MenuState {
                videos: 2,
                switch_minutes: 7,
                ..Default::default()
            },
            &[1, 5, 15],
        );
        assert_eq!(find(&items, Command::SwitchEvery(7)), Some((true, true)));
        assert_eq!(minutes_label(7), "7 minutes");
        assert_eq!(minutes_label(60), "1 hour");
        assert_eq!(minutes_label(1), "1 minute");
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

    /// A Windows `.ico` with 32-bpp images of `icon_pixels` at `sizes`.
    fn ico(sizes: &[u32]) -> Vec<u8> {
        let mut images = Vec::new();
        for &size in sizes {
            let px = icon_pixels(size);
            let mask_row = size.div_ceil(32) * 4;
            let mut img = Vec::new();
            for v in [40, size, size * 2] {
                img.extend(v.to_le_bytes());
            }
            img.extend(1u16.to_le_bytes()); // planes
            img.extend(32u16.to_le_bytes()); // bits per pixel
            img.extend([0u8; 24]); // BI_RGB, sizes and palette unused
            // Bottom-up rows; 0xAARRGGBB little-endian is B, G, R, A.
            for row in (0..size).rev() {
                for x in 0..size {
                    img.extend(px[(row * size + x) as usize].to_le_bytes());
                }
            }
            img.resize(img.len() + (mask_row * size) as usize, 0); // AND mask
            images.push((size, img));
        }
        let mut out = Vec::new();
        out.extend(0u16.to_le_bytes());
        out.extend(1u16.to_le_bytes()); // icon
        out.extend((images.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * images.len() as u32;
        for (size, img) in &images {
            let dim = if *size >= 256 { 0 } else { *size as u8 };
            out.extend([dim, dim, 0, 0]);
            out.extend(1u16.to_le_bytes());
            out.extend(32u16.to_le_bytes());
            out.extend((img.len() as u32).to_le_bytes());
            out.extend(offset.to_le_bytes());
            offset += img.len() as u32;
        }
        for (_, img) in &images {
            out.extend(img);
        }
        out
    }

    /// Checks the `.ico` layout. With `WALLIVE_WRITE_ICO=<path>` also writes
    /// it there (`installer/wallive.ico` is made this way).
    #[test]
    fn ico_file() {
        let data = ico(&[16, 32, 48, 256]);
        assert_eq!(&data[..6], &[0, 0, 1, 0, 4, 0]);
        let first = u32::from_le_bytes(data[14..18].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(data[18..22].try_into().unwrap()) as usize;
        assert_eq!(offset, 6 + 16 * 4);
        assert_eq!(first, 40 + 16 * 16 * 4 + 16 * 4);
        assert_eq!(data[offset], 40);
        if let Some(path) = std::env::var_os("WALLIVE_WRITE_ICO") {
            std::fs::write(path, &data).unwrap();
        }
    }
}
