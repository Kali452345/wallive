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
/// top row first: a rounded square holding a dusk scene - violet sky, coral
/// sun and a teal-to-blue wave under a white crest (a wallpaper that moves).
/// Drawn at run time so the binary needs no icon resource tooling; 4x4
/// supersampling for smooth edges. Small sizes get a thicker crest and a
/// bigger sun so they stay legible in the tray.
pub fn icon_pixels(size: u32) -> Vec<u32> {
    type Rgb = (f32, f32, f32);
    let s = size.max(1) as f32;
    let (crest_width, sun_radius) = if size > 24 {
        (0.09, 0.10)
    } else {
        (0.12, 0.12)
    };
    let lerp = |a: Rgb, b: Rgb, t: f32| {
        let t = t.clamp(0.0, 1.0);
        (
            a.0 + (b.0 - a.0) * t,
            a.1 + (b.1 - a.1) * t,
            a.2 + (b.2 - a.2) * t,
        )
    };
    // Colour at (u, v) in 0..1 square coordinates, `None` outside the square.
    let sample = |u: f32, v: f32| -> Option<Rgb> {
        const RADIUS: f32 = 0.22;
        let (cu, cv) = (u.clamp(RADIUS, 1.0 - RADIUS), v.clamp(RADIUS, 1.0 - RADIUS));
        if (u - cu).powi(2) + (v - cv).powi(2) > RADIUS * RADIUS {
            return None;
        }
        let crest = 0.55 + 0.09 * ((u - 0.08) * std::f32::consts::TAU).sin();
        Some(if (v - crest).abs() <= crest_width / 2.0 {
            (255.0, 255.0, 255.0)
        } else if v > crest {
            lerp((45.0, 212.0, 191.0), (37.0, 99.0, 235.0), (v - 0.45) / 0.55)
        } else if (u - 0.73).hypot(v - 0.27) <= sun_radius {
            (255.0, 140.0, 100.0)
        } else {
            lerp((60.0, 50.0, 160.0), (130.0, 70.0, 210.0), v / 0.6)
        })
    };

    const N: u32 = 4;
    let mut out = Vec::with_capacity((size * size) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut cover, mut sum) = (0u32, (0.0, 0.0, 0.0));
            for sy in 0..N {
                for sx in 0..N {
                    let u = (px as f32 + (sx as f32 + 0.5) / N as f32) / s;
                    let v = (py as f32 + (sy as f32 + 0.5) / N as f32) / s;
                    if let Some((r, g, b)) = sample(u, v) {
                        cover += 1;
                        sum = (sum.0 + r, sum.1 + g, sum.2 + b);
                    }
                }
            }
            if cover == 0 {
                out.push(0);
                continue;
            }
            // Straight alpha: the colour is the mean of the covered samples.
            let channel = |c: f32| (c / cover as f32).round() as u32;
            let (r, g, b) = (channel(sum.0), channel(sum.1), channel(sum.2));
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
    fn icon_draws_the_dusk_wave() {
        let size = 32;
        let px = icon_pixels(size);
        assert_eq!(px.len(), 32 * 32);
        let at = |x: u32, y: u32| px[(y * size + x) as usize];
        assert_eq!(at(0, 0) >> 24, 0, "corner must be transparent");
        for (x, y) in [(10, 20), (16, 4), (23, 8), (16, 28)] {
            assert_eq!(at(x, y) >> 24, 255, "({x}, {y}) is opaque");
        }
        assert_eq!(at(10, 20) & 0x00ff_ffff, 0x00ff_ffff, "wave crest is white");
        assert_eq!(at(23, 8) & 0x00ff_ffff, 0x00ff_8c64, "sun is coral");
        let (sky, water) = (at(16, 4), at(16, 28));
        assert!(sky & 0xff > (sky >> 8) & 0xff, "sky is violet");
        assert!(water & 0xff > (water >> 16) & 0xff, "water is blue");
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
