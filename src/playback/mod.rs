//! Video playback (ADR-003, ADR-007): one hardware decoder on a video
//! thread, one composition swap chain shown in every wallpaper window.
//!
//! - UI thread ([`Player`]): owns the DComp targets, reacts to attach changes
//!   and pause requests, and receives `WM_MEDIA_EVENT` posts.
//! - Video thread: decode -> video-processor blit -> present, paced by vsync.
//!   It sleeps on the swap chain's waitable object between frames and on a
//!   kernel event while paused; nothing polls.

mod ffi;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU8, Ordering};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D11::ID3D11Device;

pub use ffi::{WM_MEDIA_EVENT, create_device};

/// `WM_MEDIA_EVENT` codes (`wParam`).
pub const MEDIA_PLAYING: u32 = 1;
/// `lParam` is the failing HRESULT; the video thread has stopped.
pub const MEDIA_ERROR: u32 = 2;

/// A wallpaper window to show the video in, with its size in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub hwnd: HWND,
    pub width: u32,
    pub height: u32,
}

const RUN: u8 = 0;
const PAUSE: u8 = 1;
const STOP: u8 = 2;

/// Shared between the UI thread and one video thread.
struct Control {
    state: AtomicU8,
    /// Set after every `state` change so a sleeping video thread wakes.
    signal: ffi::Signal,
    /// Timestamp of the last decoded frame (100 ns), to resume after restart.
    position: AtomicI64,
}

impl Control {
    fn command(&self, state: u8) {
        self.state.store(state, Ordering::Release);
        self.signal.set();
    }
}

struct Worker {
    control: Arc<Control>,
    thread: JoinHandle<()>,
}

pub struct Player {
    host: HWND,
    device: ID3D11Device,
    compositor: ffi::Compositor,
    chain: Option<Arc<ffi::SwapChain>>,
    targets: Vec<ffi::Target>,
    surfaces: Vec<Surface>,
    video: Option<PathBuf>,
    worker: Option<Worker>,
    paused: bool,
    /// Where the next video thread starts (100 ns).
    position: i64,
}

impl Player {
    /// `host` receives `WM_MEDIA_EVENT` posts.
    pub fn new(host: HWND) -> windows::core::Result<Self> {
        let device = ffi::create_device()?;
        let compositor = ffi::Compositor::new(&device)?;
        Ok(Self {
            host,
            device,
            compositor,
            chain: None,
            targets: Vec::new(),
            surfaces: Vec::new(),
            video: None,
            worker: None,
            paused: false,
            position: 0,
        })
    }

    pub fn open(&mut self, path: &Path) {
        self.video = Some(path.to_path_buf());
        self.position = 0;
        self.restart();
    }

    /// Replaces the set of wallpaper windows (after attach / re-attach).
    pub fn set_surfaces(&mut self, surfaces: Vec<Surface>) {
        if surfaces == self.surfaces && self.chain.is_some() {
            return;
        }
        self.surfaces = surfaces;
        self.restart();
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Pauses or resumes the video thread. Paused, it sleeps on a kernel
    /// event and the last frame stays on screen.
    pub fn set_paused(&mut self, paused: bool) {
        if paused == self.paused {
            return;
        }
        self.paused = paused;
        if let Some(worker) = &self.worker {
            worker.control.command(if paused { PAUSE } else { RUN });
        }
        crate::log!("playback: {}", if paused { "paused" } else { "resumed" });
    }

    /// Handles one event posted by the video thread.
    pub fn on_event(&mut self, event: u32, param: usize) {
        match event {
            MEDIA_PLAYING => crate::log!("playback: first frame presented"),
            MEDIA_ERROR => crate::log!("playback: stopped on error 0x{:08X}", param as u32),
            _ => {}
        }
    }

    /// Stops the video thread, rebuilds the swap chain and targets for the
    /// current surfaces, and starts a new video thread where the last left off.
    fn restart(&mut self) {
        self.stop_worker();
        self.targets.clear();
        let Some(out) = largest(&self.surfaces) else {
            return;
        };
        let chain = match &self.chain {
            Some(chain) if chain.size() == out => chain.clone(),
            _ => {
                self.chain = None;
                match ffi::SwapChain::create(&self.device, out.0, out.1) {
                    Ok(chain) => self.chain.insert(Arc::new(chain)).clone(),
                    Err(e) => {
                        crate::log!("playback: swap chain {out:?} failed: {e}");
                        return;
                    }
                }
            }
        };
        match chain.content() {
            Ok(content) => {
                for s in &self.surfaces {
                    match self
                        .compositor
                        .target(s.hwnd, &content, out, (s.width, s.height))
                    {
                        Ok(t) => self.targets.push(t),
                        Err(e) => {
                            crate::log!("playback: DComp target for {:?} failed: {e}", s.hwnd)
                        }
                    }
                }
            }
            Err(e) => crate::log!("playback: swap chain content failed: {e}"),
        }
        if let Err(e) = self.compositor.commit() {
            crate::log!("playback: DComp commit failed: {e}");
        }
        if let Some(video) = self.video.clone() {
            self.worker = self.spawn(video, chain);
        }
    }

    fn spawn(&self, video: PathBuf, chain: Arc<ffi::SwapChain>) -> Option<Worker> {
        let signal = match ffi::Signal::new() {
            Ok(s) => s,
            Err(e) => {
                crate::log!("playback: event creation failed: {e}");
                return None;
            }
        };
        let control = Arc::new(Control {
            state: AtomicU8::new(if self.paused { PAUSE } else { RUN }),
            signal,
            position: AtomicI64::new(self.position),
        });
        let job = Job {
            device: self.device.clone(),
            chain,
            video,
            host: self.host.0 as isize,
            control: control.clone(),
            refresh_hz: ffi::refresh_hz().unwrap_or(60.0),
        };
        match std::thread::Builder::new()
            .name("wallive-video".into())
            .spawn(move || job.run())
        {
            Ok(thread) => Some(Worker { control, thread }),
            Err(e) => {
                crate::log!("playback: video thread failed to start: {e}");
                None
            }
        }
    }

    fn stop_worker(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.control.command(STOP);
            let _ = worker.thread.join();
            self.position = worker.control.position.load(Ordering::Acquire);
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop_worker();
        self.targets.clear();
    }
}

/// Everything one video thread needs; lives on that thread.
struct Job {
    device: ID3D11Device,
    chain: Arc<ffi::SwapChain>,
    video: PathBuf,
    host: isize,
    control: Arc<Control>,
    refresh_hz: f64,
}

impl Job {
    fn run(self) {
        let result = ffi::MfThread::start().and_then(|_mf| self.play());
        if let Err(e) = result {
            crate::log!("playback: {}: {e}", self.video.display());
            ffi::post(self.host, MEDIA_ERROR, e.code().0 as isize);
        }
    }

    /// Decode/present loop. Returns on STOP or error. All MF objects are
    /// local, so they are released before `MfThread` shuts MF down.
    fn play(&self) -> windows::core::Result<()> {
        let reader = ffi::Reader::open(&self.video, &self.device)?;
        let info = reader.info()?;
        let start = self.control.position.load(Ordering::Acquire);
        if start > 0 {
            reader.seek(start)?;
        }
        let out = self.chain.size();
        let crop = crop_rect((info.width, info.height), out);
        let mut processor = ffi::Processor::new(&self.device, info, crop, &self.chain)?;
        let fps = match info.fps {
            (n, d) if n > 0 && d > 0 => f64::from(n) / f64::from(d),
            _ => 30.0,
        };
        let mut cadence = Cadence::new(self.refresh_hz, fps);
        crate::log!(
            "playback: {}x{} @ {fps:.2} fps -> {}x{}, display {:.2} Hz, {:.2} refreshes/frame",
            info.width,
            info.height,
            out.0,
            out.1,
            self.refresh_hz,
            cadence.per_frame
        );

        let mut announced = false;
        let mut frames_since_rewind = 0u64;
        let mut rate = RateLog::default();
        loop {
            match self.control.state.load(Ordering::Acquire) {
                STOP => return Ok(()),
                // A thread started while paused still shows one frame, so a
                // re-attach during a pause does not leave the wallpaper empty.
                PAUSE if announced => {
                    self.control.signal.wait();
                    rate = RateLog {
                        reports: rate.reports,
                        ..RateLog::default()
                    };
                    continue;
                }
                _ => {}
            }
            let Some((sample, time)) = reader.next()? else {
                if frames_since_rewind == 0 {
                    return Err(windows::core::Error::new(
                        windows::Win32::Foundation::E_FAIL,
                        "video has no decodable frames",
                    ));
                }
                frames_since_rewind = 0;
                reader.seek(0)?;
                continue;
            };
            frames_since_rewind += 1;
            self.control.position.store(time, Ordering::Release);

            let refreshes = cadence.next();
            if refreshes == 0 {
                continue;
            }
            if self.chain.wait_ready(&self.control.signal) == ffi::Wake::Control {
                continue;
            }
            processor.blit(&sample)?;
            // A control wake-up is handled at the top of the loop.
            self.chain.present(refreshes, &self.control.signal)?;
            if !announced {
                announced = true;
                ffi::post(self.host, MEDIA_PLAYING, 0);
            }
            rate.frame();
        }
    }
}

/// Logs the achieved frame rate a few times after start, as a cheap check
/// that pacing works on this machine.
#[derive(Default)]
struct RateLog {
    since: Option<std::time::Instant>,
    frames: u32,
    reports: u32,
}

impl RateLog {
    const EVERY: u32 = 150;
    const REPORTS: u32 = 3;

    fn frame(&mut self) {
        if self.reports >= Self::REPORTS {
            return;
        }
        let since = *self.since.get_or_insert_with(std::time::Instant::now);
        self.frames += 1;
        if self.frames == Self::EVERY {
            let secs = since.elapsed().as_secs_f64();
            crate::log!(
                "playback: {:.2} frames/s over {secs:.1} s",
                f64::from(self.frames) / secs
            );
            self.frames = 0;
            self.since = Some(std::time::Instant::now());
            self.reports += 1;
        }
    }
}

/// Spreads video frames over display refreshes.
#[derive(Clone, Copy, Debug)]
pub struct Cadence {
    per_frame: f64,
    owed: f64,
}

impl Cadence {
    pub fn new(refresh_hz: f64, fps: f64) -> Self {
        let mut per_frame = if refresh_hz > 0.0 && fps > 0.0 {
            refresh_hz / fps
        } else {
            1.0
        };
        // Within 0.5% of a whole number of refreshes (30 fps on 59.94 Hz):
        // use it exactly. The speed change is invisible and every frame gets
        // equal screen time instead of an occasional short frame.
        let whole = per_frame.round();
        if whole >= 1.0 && (per_frame - whole).abs() <= 0.005 * whole {
            per_frame = whole;
        }
        Self {
            per_frame,
            owed: 0.0,
        }
    }

    /// Refreshes the next frame stays on screen; 0 = skip it (video faster
    /// than the display).
    pub fn next(&mut self) -> u32 {
        self.owed += self.per_frame;
        let n = self.owed.floor();
        self.owed -= n;
        n as u32
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

/// [`cover_crop`] in video pixels.
fn crop_rect(video: (u32, u32), out: (u32, u32)) -> RECT {
    let [l, t, r, b] = cover_crop(video, out);
    let (w, h) = (video.0 as f32, video.1 as f32);
    RECT {
        left: (l * w).round() as i32,
        top: (t * h).round() as i32,
        right: (r * w).round() as i32,
        bottom: (b * h).round() as i32,
    }
}

/// Benchmark (`--bench-decode`): hardware-decodes every frame of `src` as
/// fast as possible, `loops` times, and reports CPU milliseconds per frame -
/// the floor any playback pipeline on this machine pays per frame.
pub fn bench_decode(src: &Path, loops: u32) -> windows::core::Result<()> {
    let _mf = ffi::MfThread::start()?;
    let device = ffi::create_device()?;
    let mut frames = 0u64;
    let wall = std::time::Instant::now();
    let cpu0 = ffi::process_cpu_ms();
    for _ in 0..loops.max(1) {
        let reader = ffi::Reader::open(src, &device)?;
        while reader.next()?.is_some() {
            frames += 1;
        }
    }
    let cpu = ffi::process_cpu_ms() - cpu0;
    let secs = wall.elapsed().as_secs_f64();
    crate::log!(
        "bench-decode: {frames} frames in {secs:.2} s ({:.0} fps), CPU {cpu:.0} ms = {:.2} ms/frame",
        frames as f64 / secs,
        cpu / frames.max(1) as f64
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(mut c: Cadence, n: usize) -> Vec<u32> {
        (0..n).map(|_| c.next()).collect()
    }

    #[test]
    fn cadence_30_on_60_is_even() {
        assert_eq!(run(Cadence::new(60.0, 30.0), 4), [2, 2, 2, 2]);
    }

    #[test]
    fn cadence_24_on_60_is_3_2_pulldown() {
        let v = run(Cadence::new(60.0, 24.0), 4);
        assert_eq!(v.iter().sum::<u32>(), 10);
        assert!(v.iter().all(|&n| n == 2 || n == 3), "{v:?}");
    }

    #[test]
    fn cadence_snaps_ntsc_rates() {
        assert_eq!(run(Cadence::new(59.94, 30.0), 3), [2, 2, 2]);
        assert_eq!(run(Cadence::new(60.0, 29.97), 3), [2, 2, 2]);
    }

    #[test]
    fn cadence_drops_frames_when_video_is_faster() {
        assert_eq!(run(Cadence::new(30.0, 60.0), 4), [0, 1, 0, 1]);
    }

    #[test]
    fn cadence_high_refresh_exceeds_present_limit() {
        // 24 fps on 144 Hz: 6 refreshes per frame, presented as 4 + 2.
        assert_eq!(run(Cadence::new(144.0, 24.0), 2), [6, 6]);
    }

    #[test]
    fn cadence_unknown_rates_show_every_frame_once() {
        assert_eq!(run(Cadence::new(0.0, 30.0), 2), [1, 1]);
    }

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
    fn crop_rect_in_pixels() {
        let r = crop_rect((2560, 1080), (1920, 1080));
        assert_eq!((r.left, r.top, r.right, r.bottom), (320, 0, 2240, 1080));
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
