//! Video playback: one Media Engine (one hardware decoder) whose windowless
//! swap chain is shown in every wallpaper window through DirectComposition
//! (ADR-003, ADR-007).
//!
//! Runs on the UI thread. Engine callbacks arrive as `WM_MEDIA_EVENT` posts
//! and are handled by [`Player::on_event`]; nothing here polls.

mod ffi;

use std::path::Path;

use windows::Win32::Foundation::HWND;
use windows::Win32::Media::MediaFoundation::IMFMediaEngineEx;

pub use ffi::{WM_MEDIA_EVENT, create_device};

// MF_MEDIA_ENGINE_EVENT values handled here.
const EVENT_ERROR: u32 = 5;
const EVENT_LOADEDMETADATA: u32 = 10;
const EVENT_PLAYING: u32 = 13;
const EVENT_PAUSE: u32 = 9;

/// Media Engine events worth waking the UI thread for. Everything else
/// (notably TIMEUPDATE, several per second) is dropped on the MF thread.
pub fn is_interesting(event: u32) -> bool {
    matches!(
        event,
        EVENT_ERROR | EVENT_LOADEDMETADATA | EVENT_PLAYING | EVENT_PAUSE
    )
}

/// A wallpaper window to show the video in, with its size in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub hwnd: HWND,
    pub width: u32,
    pub height: u32,
}

pub struct Player {
    engine: IMFMediaEngineEx,
    compositor: ffi::Compositor,
    targets: Vec<ffi::Target>,
    surfaces: Vec<Surface>,
    /// Swap-chain size (largest surface), once metadata is loaded.
    output: Option<(u32, u32)>,
    paused: bool,
    // Declared last: Media Foundation must shut down after the engine drops.
    _platform: ffi::Platform,
}

impl Player {
    /// Creates the engine. `host` receives `WM_MEDIA_EVENT` posts.
    pub fn new(host: HWND) -> windows::core::Result<Self> {
        let platform = ffi::Platform::start()?;
        let device = ffi::create_device()?;
        let engine = ffi::create_engine(&device, host)?;
        ffi::configure(&engine)?;
        let compositor = ffi::Compositor::new(&device)?;
        Ok(Self {
            engine,
            compositor,
            targets: Vec::new(),
            surfaces: Vec::new(),
            output: None,
            paused: false,
            _platform: platform,
        })
    }

    pub fn open(&mut self, path: &Path) -> windows::core::Result<()> {
        self.output = None;
        self.targets.clear();
        ffi::set_source(&self.engine, &file_url(path))
    }

    /// Replaces the set of wallpaper windows (after attach / re-attach).
    pub fn set_surfaces(&mut self, surfaces: Vec<Surface>) {
        if surfaces == self.surfaces && !self.targets.is_empty() {
            return;
        }
        let largest = largest(&surfaces);
        self.surfaces = surfaces;
        // A bigger monitor appeared: resize the swap chain first.
        if self.output.is_some() && largest != self.output {
            self.setup_output();
        }
        self.rebuild_targets();
    }

    #[expect(dead_code, reason = "wired up by the pause policy (power / occlusion)")]
    pub fn set_paused(&mut self, paused: bool) {
        if paused == self.paused {
            return;
        }
        self.paused = paused;
        // Before metadata loads, the flag alone decides whether we start.
        if self.output.is_none() {
            return;
        }
        let result = if paused {
            ffi::pause(&self.engine)
        } else {
            ffi::play(&self.engine)
        };
        if let Err(e) = result {
            crate::log!(
                "playback: {} failed: {e}",
                if paused { "pause" } else { "play" }
            );
        }
    }

    /// Handles one engine event posted to the host window.
    pub fn on_event(&mut self, event: u32, _param1: usize) {
        match event {
            EVENT_LOADEDMETADATA => {
                crate::log!(
                    "playback: metadata loaded, native size {:?}",
                    ffi::native_size(&self.engine)
                );
                self.setup_output();
                self.rebuild_targets();
                if !self.paused
                    && let Err(e) = ffi::play(&self.engine)
                {
                    crate::log!("playback: play failed: {e}");
                }
            }
            EVENT_PLAYING => crate::log!("playback: playing"),
            EVENT_PAUSE => crate::log!("playback: paused"),
            EVENT_ERROR => crate::log!("playback: error {:?}", ffi::last_error(&self.engine)),
            _ => {}
        }
    }

    fn setup_output(&mut self) {
        let Some(out) = largest(&self.surfaces) else {
            return;
        };
        let crop = ffi::native_size(&self.engine)
            .map(|video| cover_crop(video, out))
            .unwrap_or([0.0, 0.0, 1.0, 1.0]);
        match ffi::windowless_output(&self.engine, crop, out.0, out.1)
            .and_then(|handle| self.compositor.set_swapchain(handle))
        {
            Ok(()) => self.output = Some(out),
            Err(e) => crate::log!("playback: windowless output failed: {e}"),
        }
    }

    fn rebuild_targets(&mut self) {
        self.targets.clear();
        let Some(out) = self.output else {
            return;
        };
        if !self.compositor.has_surface() {
            return;
        }
        for s in &self.surfaces {
            match self.compositor.target(s.hwnd, out, (s.width, s.height)) {
                Ok(t) => self.targets.push(t),
                Err(e) => crate::log!("playback: DComp target for {:?} failed: {e}", s.hwnd),
            }
        }
        if let Err(e) = self.compositor.commit() {
            crate::log!("playback: DComp commit failed: {e}");
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.targets.clear();
        ffi::shutdown(&self.engine);
        crate::log!("playback: engine shut down");
    }
}

fn largest(surfaces: &[Surface]) -> Option<(u32, u32)> {
    surfaces
        .iter()
        .max_by_key(|s| u64::from(s.width) * u64::from(s.height))
        .map(|s| (s.width, s.height))
}

/// Normalised source rectangle that fills `out` without distortion,
/// cropping the video's excess width or height equally on both sides.
pub fn cover_crop(video: (u32, u32), out: (u32, u32)) -> [f32; 4] {
    let (vw, vh) = (video.0.max(1) as f64, video.1.max(1) as f64);
    let (ow, oh) = (out.0.max(1) as f64, out.1.max(1) as f64);
    let (video_aspect, out_aspect) = (vw / vh, ow / oh);
    if (video_aspect - out_aspect).abs() < 1e-3 {
        return [0.0, 0.0, 1.0, 1.0];
    }
    if video_aspect > out_aspect {
        // Video is wider: keep full height, crop the sides.
        let keep = out_aspect / video_aspect;
        let side = ((1.0 - keep) / 2.0) as f32;
        [side, 0.0, 1.0 - side, 1.0]
    } else {
        let keep = video_aspect / out_aspect;
        let edge = ((1.0 - keep) / 2.0) as f32;
        [0.0, edge, 1.0, 1.0 - edge]
    }
}

/// `file:///` URL for a local path, percent-encoding everything that is not
/// an unreserved URL character or a path separator.
pub fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut url = String::from("file:///");
    for byte in text.trim_start_matches('/').bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                url.push(byte as char)
            }
            _ => url.push_str(&format!("%{byte:02X}")),
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_aspect_is_uncropped() {
        assert_eq!(cover_crop((1920, 1080), (1280, 720)), [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn wider_video_crops_sides() {
        // 21:9-ish video on a 16:9 screen.
        let [l, t, r, b] = cover_crop((2560, 1080), (1920, 1080));
        assert!(
            (l - 0.125).abs() < 1e-4 && (r - 0.875).abs() < 1e-4,
            "{l} {r}"
        );
        assert_eq!((t, b), (0.0, 1.0));
    }

    #[test]
    fn taller_video_crops_top_and_bottom() {
        let [l, t, r, b] = cover_crop((1080, 1080), (1920, 1080));
        assert_eq!((l, r), (0.0, 1.0));
        assert!(
            (t - 0.21875).abs() < 1e-4 && (b - 0.78125).abs() < 1e-4,
            "{t} {b}"
        );
    }

    #[test]
    fn file_url_encodes_spaces_and_unicode() {
        let url = file_url(Path::new(r"C:\My Videos\café #1.mp4"));
        assert_eq!(url, "file:///C:/My%20Videos/caf%C3%A9%20%231.mp4");
    }

    #[test]
    fn largest_by_area() {
        let s = |w, h| Surface {
            hwnd: HWND::default(),
            width: w,
            height: h,
        };
        assert_eq!(
            largest(&[s(1920, 1080), s(2560, 1440), s(1080, 1920)]),
            Some((2560, 1440))
        );
        assert_eq!(largest(&[]), None);
    }
}
