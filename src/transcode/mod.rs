//! Import pipeline (ADR-004): decode any video Media Foundation can open,
//! resize to cover the largest monitor (never upscale), cap the frame rate,
//! drop audio, and encode H.264 8-bit 4:2:0 into the cache.
//!
//! Runs in a short-lived child process (`wallive --import ...`) so encoder
//! DLLs and buffers never live in the resident wallpaper process.

mod ffi;
mod mp4;

use std::path::{Path, PathBuf};

use ffi::{Decoder, Encoder, VideoFormat};
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Media::MediaFoundation::IMFSample;

pub const DEFAULT_FPS_CAP: u32 = 30;

/// Output format for a source: cover `monitor` preserving aspect, never
/// larger than the source, even dimensions, frame rate capped at `fps_cap`.
pub fn plan(src: VideoFormat, monitor: (u32, u32), fps_cap: u32) -> VideoFormat {
    let (sw, sh) = (f64::from(src.width.max(2)), f64::from(src.height.max(2)));
    let (mw, mh) = (f64::from(monitor.0.max(2)), f64::from(monitor.1.max(2)));
    let scale = (mw / sw).max(mh / sh).min(1.0);
    let even = |v: f64| ((v.round() as u32) & !1).max(2);
    let src_fps = f64::from(src.fps.0) / f64::from(src.fps.1.max(1));
    let fps = if src.fps.0 == 0 || src_fps > f64::from(fps_cap) {
        (fps_cap, 1)
    } else {
        src.fps
    };
    VideoFormat {
        width: even(sw * scale),
        height: even(sh * scale),
        fps,
    }
}

/// Target bitrate: ~0.13 bits per pixel per frame (about 8 Mbit/s at
/// 1080p30), clamped to a sane range.
pub fn bitrate(f: VideoFormat) -> u32 {
    let fps = f64::from(f.fps.0) / f64::from(f.fps.1.max(1));
    let bits = f64::from(f.width) * f64::from(f.height) * fps * 0.13;
    bits.clamp(1_000_000.0, 40_000_000.0) as u32
}

fn frame_duration_hns(fps: (u32, u32)) -> i64 {
    10_000_000 * i64::from(fps.1.max(1)) / i64::from(fps.0.max(1))
}

/// Imports `src` into `dst` (MP4, H.264). Returns the output format.
pub fn import(src: &Path, dst: &Path, monitor: (u32, u32)) -> windows::core::Result<VideoFormat> {
    let _platform = ffi::Platform::start()?;
    if let Ok(device) = crate::playback::create_device() {
        match ffi::probe_decoders(&device) {
            Ok(support) => crate::log!("import: hardware decode {support:?}"),
            Err(e) => crate::log!("import: decoder probe failed: {e}"),
        }
    }

    // Held until the import ends; deletes the patched copy, if any.
    let mut _copy = None;
    let (decoder, out, first) = match open_source(src, monitor)? {
        (decoder, out, Some(first)) => (decoder, out, first),
        (_, _, None) => {
            let path = dst.with_extension("src.mp4");
            let patched = mp4::copy_without_fragment_edit_lists(src, &path)
                .map_err(|e| windows::core::Error::new(E_FAIL, e.to_string()))?;
            if !patched {
                return Err(no_frames());
            }
            crate::log!("import: no frames; fragmented MP4 with an edit list, retrying without it");
            let copy = _copy.insert(TempFile(path));
            match open_source(&copy.0, monitor)? {
                (decoder, out, Some(first)) => (decoder, out, first),
                (_, _, None) => return Err(no_frames()),
            }
        }
    };

    let tmp = dst.with_extension("part.mp4");
    let mut encoder = Encoder::create(&tmp, out, bitrate(out))?;
    let duration = frame_duration_hns(out.fps);
    let start = first.1;
    let mut frames = 0u64;
    let mut next = Some(first);
    while let Some((sample, time)) = next {
        encoder.write_sample(&sample, time - start, duration)?;
        frames += 1;
        next = decoder.next()?;
    }
    encoder.finish()?;
    std::fs::rename(&tmp, dst).map_err(|e| windows::core::Error::new(E_FAIL, e.to_string()))?;
    crate::log!("import: wrote {frames} frames to {}", dst.display());
    Ok(out)
}

/// A decoded frame and its timestamp (100 ns).
type Frame = (IMFSample, i64);

/// Opens `src` with NV12 output planned for `monitor` and reads the first
/// frame (`None` if the source yields no frames at all).
fn open_source(
    src: &Path,
    monitor: (u32, u32),
) -> windows::core::Result<(Decoder, VideoFormat, Option<Frame>)> {
    let decoder = Decoder::open(src)?;
    let native = decoder.native_format()?;
    let out = plan(native, monitor, DEFAULT_FPS_CAP);
    crate::log!("import: {native:?} -> {out:?}, {} bit/s", bitrate(out));
    decoder.set_output(out)?;
    let first = decoder.next()?;
    Ok((decoder, out, first))
}

fn no_frames() -> windows::core::Error {
    windows::core::Error::new(E_FAIL, "source has no decodable video frames")
}

/// Deletes the file when dropped.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Writes a synthetic, seamlessly looping H.264 test clip: a scrolling colour
/// gradient with a white square circling the centre. Motion is visible in
/// any two screenshots, and frame N wraps back to frame 0 exactly.
pub fn make_test_clip(
    dst: &Path,
    width: u32,
    height: u32,
    fps: u32,
    secs: u32,
) -> windows::core::Result<()> {
    let _platform = ffi::Platform::start()?;
    let format = VideoFormat {
        width: width & !1,
        height: height & !1,
        fps: (fps, 1),
    };
    let mut encoder = Encoder::create(dst, format, bitrate(format))?;
    let total = u64::from(fps * secs);
    let duration = frame_duration_hns(format.fps);
    let mut frame = vec![0u8; (format.width * format.height * 3 / 2) as usize];
    for i in 0..total {
        fill_test_frame(
            &mut frame,
            format.width,
            format.height,
            i as f32 / total as f32,
        );
        encoder.write_nv12(&frame, i as i64 * duration, duration)?;
    }
    encoder.finish()?;
    crate::log!("test clip: {total} frames {format:?} -> {}", dst.display());
    Ok(())
}

/// Fills an NV12 frame for loop phase `t` in `[0, 1)`.
pub fn fill_test_frame(frame: &mut [u8], w: u32, h: u32, t: f32) {
    let (w, h) = (w as usize, h as usize);
    let tau = std::f32::consts::TAU;
    let (y_plane, uv_plane) = frame.split_at_mut(w * h);
    let side = h / 5;
    let cx = w as f32 / 2.0 + (w as f32 / 4.0) * (tau * t).cos();
    let cy = h as f32 / 2.0 + (h as f32 / 4.0) * (tau * t).sin();
    let (x0, y0) = (
        (cx as usize).saturating_sub(side / 2),
        (cy as usize).saturating_sub(side / 2),
    );
    for y in 0..h {
        for x in 0..w {
            let inside = x >= x0 && x < x0 + side && y >= y0 && y < y0 + side;
            let phase = x as f32 / w as f32 + t;
            y_plane[y * w + x] = if inside {
                235
            } else {
                (80.0 + 60.0 * (tau * phase).sin()) as u8
            };
        }
    }
    for y in 0..h / 2 {
        for x in 0..w / 2 {
            let phase = (2 * x) as f32 / w as f32 + t;
            let i = y * w + 2 * x;
            uv_plane[i] = (128.0 + 90.0 * (tau * phase).cos()) as u8;
            uv_plane[i + 1] = (128.0 + 90.0 * (tau * (phase + 0.33)).sin()) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(width: u32, height: u32, fps: (u32, u32)) -> VideoFormat {
        VideoFormat { width, height, fps }
    }

    #[test]
    fn uhd60_to_1080p30() {
        assert_eq!(
            plan(fmt(3840, 2160, (60, 1)), (1920, 1080), 30),
            fmt(1920, 1080, (30, 1))
        );
    }

    #[test]
    fn never_upscales() {
        assert_eq!(
            plan(fmt(1280, 720, (24, 1)), (1920, 1080), 30),
            fmt(1280, 720, (24, 1))
        );
    }

    #[test]
    fn keeps_aspect_and_covers_monitor() {
        // Ultra-wide source on a 16:9 monitor: height must cover 1080.
        let out = plan(fmt(5120, 2160, (30, 1)), (1920, 1080), 30);
        assert_eq!(out.height, 1080);
        assert_eq!(out.width, 2560);
    }

    #[test]
    fn dimensions_are_even() {
        let out = plan(fmt(1921, 1081, (30, 1)), (1000, 563), 30);
        assert_eq!(out.width % 2, 0);
        assert_eq!(out.height % 2, 0);
    }

    #[test]
    fn ntsc_rate_under_cap_is_kept() {
        assert_eq!(
            plan(fmt(1920, 1080, (30000, 1001)), (1920, 1080), 30).fps,
            (30000, 1001)
        );
    }

    #[test]
    fn bitrate_1080p30_is_about_8_mbit() {
        let b = bitrate(fmt(1920, 1080, (30, 1)));
        assert!((7_500_000..=8_500_000).contains(&b), "{b}");
    }

    #[test]
    fn test_frame_loops_seamlessly() {
        let (w, h) = (64, 36);
        let mut a = vec![0u8; w * h * 3 / 2];
        let mut b = a.clone();
        fill_test_frame(&mut a, w as u32, h as u32, 0.0);
        fill_test_frame(&mut b, w as u32, h as u32, 1.0);
        let diff = a
            .iter()
            .zip(&b)
            .filter(|(x, y)| x.abs_diff(**y) > 1)
            .count();
        assert_eq!(diff, 0, "phase 1.0 must equal phase 0.0");
    }
}
