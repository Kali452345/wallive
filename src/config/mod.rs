//! Settings in `%APPDATA%\Wallive\config.txt`: one `key=value` per line,
//! `#` comments. Small and human-editable; unknown keys are ignored so older
//! builds can read newer files.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// The video the user chose.
    pub source: Option<PathBuf>,
    /// The imported, playable copy of `source` in the cache.
    pub wallpaper: Option<PathBuf>,
    /// Paused from the tray menu.
    pub paused: bool,
    pub pause_on_battery: bool,
}

fn flag(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn path(v: &str) -> Option<PathBuf> {
    let v = v.trim();
    (!v.is_empty()).then(|| PathBuf::from(v))
}

impl Config {
    pub fn parse(text: &str) -> Self {
        let mut c = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "source" => c.source = path(value),
                "wallpaper" => c.wallpaper = path(value),
                "paused" => c.paused = flag(value),
                "pause_on_battery" => c.pause_on_battery = flag(value),
                _ => {}
            }
        }
        c
    }

    pub fn serialize(&self) -> String {
        let p = |v: &Option<PathBuf>| {
            v.as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        };
        format!(
            "# Wallive settings\n\
             source={}\n\
             wallpaper={}\n\
             paused={}\n\
             pause_on_battery={}\n",
            p(&self.source),
            p(&self.wallpaper),
            self.paused,
            self.pause_on_battery
        )
    }

    /// Reads the config; a missing or unreadable file gives the defaults.
    /// Returns whether the file existed (first run otherwise).
    pub fn load(file: &Path) -> (Self, bool) {
        match std::fs::read_to_string(file) {
            Ok(text) => (Self::parse(&text), true),
            Err(_) => (Self::default(), false),
        }
    }

    /// Writes via a temporary file and rename, so a crash never leaves a
    /// half-written config.
    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = file.with_extension("tmp");
        std::fs::write(&tmp, self.serialize())?;
        std::fs::rename(&tmp, file)
    }
}

fn env_dir(var: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Wallive")
}

/// `%APPDATA%\Wallive\config.txt` (roams with the user profile).
pub fn config_file() -> PathBuf {
    env_dir("APPDATA").join("config.txt")
}

/// `%LOCALAPPDATA%\Wallive` for the cache and the log (machine-local).
pub fn local_dir() -> PathBuf {
    env_dir("LOCALAPPDATA")
}

pub fn cache_dir() -> PathBuf {
    local_dir().join("cache")
}

/// Cache file name for `source` imported for a `size` screen: a hash of the
/// path, file size, modification time and target size, so editing or
/// replacing the source, or a bigger monitor, gives a new import.
pub fn cache_name(source: &Path, len: u64, modified_secs: u64, size: (u32, u32)) -> String {
    // FNV-1a 64; stable across builds, unlike `DefaultHasher`.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    eat(source.to_string_lossy().to_lowercase().as_bytes());
    eat(&len.to_le_bytes());
    eat(&modified_secs.to_le_bytes());
    eat(&size.0.to_le_bytes());
    eat(&size.1.to_le_bytes());
    format!("{h:016x}.mp4")
}

/// [`cache_name`] from the file's metadata; `None` if it cannot be read.
pub fn cache_file(source: &Path, size: (u32, u32)) -> Option<PathBuf> {
    let meta = std::fs::metadata(source).ok()?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    Some(cache_dir().join(cache_name(source, meta.len(), modified, size)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let c = Config {
            source: Some(PathBuf::from(r"C:\Videos\a=b.mp4")),
            wallpaper: Some(PathBuf::from(r"C:\cache\0123.mp4")),
            paused: true,
            pause_on_battery: false,
        };
        assert_eq!(Config::parse(&c.serialize()), c);
    }

    #[test]
    fn tolerant_parse() {
        let c = Config::parse(
            "# comment\n\n  paused = yes \nunknown=1\nno equals sign\nsource=\npause_on_battery=ON\r\n",
        );
        assert_eq!(
            c,
            Config {
                source: None,
                wallpaper: None,
                paused: true,
                pause_on_battery: true,
            }
        );
    }

    #[test]
    fn empty_is_default() {
        assert_eq!(Config::parse(""), Config::default());
    }

    #[test]
    fn cache_name_changes_with_inputs() {
        let p = Path::new(r"C:\v.mp4");
        let base = cache_name(p, 10, 20, (1920, 1080));
        assert_eq!(base.len(), 20);
        assert!(base.ends_with(".mp4"));
        assert_eq!(
            base,
            cache_name(Path::new(r"c:\V.MP4"), 10, 20, (1920, 1080))
        );
        assert_ne!(base, cache_name(p, 11, 20, (1920, 1080)));
        assert_ne!(base, cache_name(p, 10, 21, (1920, 1080)));
        assert_ne!(base, cache_name(p, 10, 20, (2560, 1440)));
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("wallive-test-{}", std::process::id()));
        let file = dir.join("config.txt");
        let c = Config {
            paused: true,
            ..Default::default()
        };
        c.save(&file).unwrap();
        assert_eq!(Config::load(&file), (c, true));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(Config::load(&file), (Config::default(), false));
    }
}
