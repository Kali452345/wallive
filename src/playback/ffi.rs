//! Thin wrappers over the Media Foundation Source Reader, the D3D11 video
//! processor, DXGI composition swap chains and DirectComposition. All
//! `unsafe` for `playback/` lives here.
//!
//! Pipeline (ADR-003, revised): hardware decoder (DXVA, NV12 textures) ->
//! one video-processor blit (crop + scale + NV12->BGRA) into a composition
//! swap chain -> one DComp visual per wallpaper window. The video thread
//! sleeps in the kernel between frames: on DWM's compositor clock (one tick
//! per display refresh) and the swap chain's frame-latency waitable object.
//!
//! Why not `Present(n)` with a sync interval: DWM retires vsync-synced
//! presents for a visual under the full-screen icon layer only every
//! ~250 ms (it treats the window as occluded), which gave ~4 fps.
//! `IDXGIOutput::WaitForVBlank` returns immediately for a windowed swap
//! chain here. So frames are presented with sync interval 0 and held for
//! N compositor clock ticks (logs/experiments.md).
#![allow(unsafe_code)]

use std::mem::ManuallyDrop;
use std::path::Path;
use std::sync::OnceLock;

use windows::Win32::Foundation::{
    CloseHandle, FILETIME, HANDLE, HMODULE, HWND, LPARAM, RECT, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_9_3, D3D_FEATURE_LEVEL_10_0,
    D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
    D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
    D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
    D3D11_VIDEO_USAGE_OPTIMAL_SPEED, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D, D3D11CreateDevice, ID3D11Device, ID3D11Multithread,
    ID3D11Texture2D, ID3D11VideoContext1, ID3D11VideoDevice, ID3D11VideoProcessor,
    ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorInputView, ID3D11VideoProcessorOutputView,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice2, IDCompositionDesktopDevice, IDCompositionTarget,
    IDCompositionVisual2,
};
use windows::Win32::Graphics::Dwm::{DWM_TIMING_INFO, DwmGetCompositionTimingInfo};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
    DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_RATIONAL,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIAdapter, IDXGIDevice, IDXGIDevice3, IDXGIFactory2,
    IDXGISwapChain2,
};
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFDXGIBuffer, IMFDXGIDeviceManager, IMFSample, IMFSourceReader,
    MF_BYTESTREAM_CONTENT_TYPE, MF_LOW_LATENCY, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS,
    MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_D3D_MANAGER,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READERF_ENDOFSTREAM, MF_VERSION,
    MFCreateAttributes, MFCreateDXGIDeviceManager, MFCreateMFByteStreamOnStream, MFCreateMediaType,
    MFCreateSourceReaderFromByteStream, MFMediaType_Video, MFSTARTUP_LITE, MFShutdown, MFStartup,
    MFVideoFormat_NV12,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{
    COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize, STGM_READ, STGM_SHARE_DENY_WRITE,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, GetProcessTimes, INFINITE, SetEvent, WaitForMultipleObjects,
    WaitForSingleObject,
};
use windows::Win32::System::Variant::VT_I8;
use windows::Win32::UI::Shell::SHCreateStreamOnFileEx;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use windows::core::{GUID, HSTRING, IUnknown, Interface, Result, s, w};
use windows_numerics::Matrix3x2;

/// Message the video thread posts to the host window:
/// `wParam` = one of the `MEDIA_*` codes in `playback`, `lParam` = detail.
pub const WM_MEDIA_EVENT: u32 = WM_APP + 2;

/// Posts a playback event to the host window. Thread-safe.
pub fn post(host: isize, event: u32, param: isize) {
    // SAFETY: PostMessage is thread-safe and fails cleanly if the host
    // window no longer exists.
    unsafe {
        let _ = PostMessageW(
            Some(HWND(host as *mut _)),
            WM_MEDIA_EVENT,
            WPARAM(event as usize),
            LPARAM(param),
        );
    }
}

/// COM (MTA) + Media Foundation for the video thread's lifetime.
pub struct MfThread(());

impl MfThread {
    pub fn start() -> Result<Self> {
        // SAFETY: first COM call on this (new) thread.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
        // SAFETY: MF startup with the bindings' SDK version; lite = no
        // network sources, which we never use.
        if let Err(e) = unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE) } {
            // SAFETY: balances the successful CoInitializeEx above.
            unsafe { CoUninitialize() };
            return Err(e);
        }
        Ok(Self(()))
    }
}

impl Drop for MfThread {
    fn drop(&mut self) {
        // SAFETY: balances MFStartup / CoInitializeEx on this thread; every
        // MF object created by the thread has been dropped before this.
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}

/// D3D11 device with video support, shared by decoder, video processor,
/// swap chain and DComp. Multithread-protected: the UI thread and the video
/// thread both use it.
pub fn create_device() -> Result<ID3D11Device> {
    let levels: [D3D_FEATURE_LEVEL; 5] = [
        D3D_FEATURE_LEVEL_11_1,
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
        D3D_FEATURE_LEVEL_9_3,
    ];
    let mut device = None;
    // SAFETY: out-pointer is a valid Option; feature level slice outlives
    // the call; no software module.
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )
    }?;
    let device = device.ok_or_else(windows::core::Error::empty)?;
    let mt: ID3D11Multithread = device.cast()?;
    // SAFETY: plain setter on a live interface.
    unsafe {
        let _ = mt.SetMultithreadProtected(true);
    }
    Ok(device)
}

/// Display refresh rate DWM composes at, in Hz.
pub fn refresh_hz() -> Option<f64> {
    let mut info = DWM_TIMING_INFO {
        cbSize: size_of::<DWM_TIMING_INFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a valid, sized out-struct; a null HWND asks for the
    // global composition timing (required on Windows 8.1+).
    unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut info) }.ok()?;
    let r = info.rateRefresh;
    (r.uiNumerator > 0 && r.uiDenominator > 0)
        .then(|| f64::from(r.uiNumerator) / f64::from(r.uiDenominator))
}

/// CPU time (user + kernel) this process has used, in milliseconds.
pub fn process_cpu_ms() -> f64 {
    let (mut create, mut exit, mut kernel, mut user) = Default::default();
    // SAFETY: pseudo-handle of this process; all out-pointers are valid.
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut create,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok.is_err() {
        return 0.0;
    }
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    (ticks(kernel) + ticks(user)) as f64 / 10_000.0
}

/// Frees GPU memory of released resources now: D3D11 destroys objects only
/// when the context is flushed, and `Trim` returns the driver's internal
/// allocations (meant for apps going idle).
pub fn trim(device: &ID3D11Device) {
    // SAFETY: immediate-context calls on the video thread, the only thread
    // using the context; no resources are bound afterwards.
    unsafe {
        if let Ok(context) = device.GetImmediateContext() {
            context.ClearState();
            context.Flush();
        }
        if let Ok(dxgi) = device.cast::<IDXGIDevice3>() {
            dxgi.Trim();
        }
    }
}

/// Auto-reset kernel event used to wake the video thread.
pub struct Signal(HANDLE);

// SAFETY: an event handle may be set and waited on from any thread; the
// handle is closed only in Drop, when no other reference exists.
unsafe impl Send for Signal {}
// SAFETY: as above; `set` and waits do not mutate the Rust value.
unsafe impl Sync for Signal {}

impl Signal {
    pub fn new() -> Result<Self> {
        // SAFETY: unnamed auto-reset event, initially non-signalled.
        Ok(Self(unsafe { CreateEventW(None, false, false, None) }?))
    }

    pub fn set(&self) {
        // SAFETY: the handle is a live event owned by `self`.
        unsafe {
            let _ = SetEvent(self.0);
        }
    }

    /// Blocks until the event is set.
    pub fn wait(&self) {
        // SAFETY: the handle is a live event owned by `self`.
        unsafe {
            WaitForSingleObject(self.0, INFINITE);
        }
    }

    /// Blocks until the event is set (`true`) or `ms` pass (`false`).
    pub fn wait_for(&self, ms: u32) -> bool {
        // SAFETY: the handle is a live event owned by `self`.
        unsafe { WaitForSingleObject(self.0, ms) == WAIT_OBJECT_0 }
    }
}

impl Drop for Signal {
    fn drop(&mut self) {
        // SAFETY: closing the handle this value owns, once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Why [`SwapChain::wait_ready`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// The swap chain can take a new frame.
    Ready,
    /// The control signal was set.
    Control,
}

/// `DCompositionWaitForCompositorClock` (Windows 11+): waits for the next
/// compositor clock tick or one of `handles`.
type WaitForCompositorClock =
    unsafe extern "system" fn(count: u32, handles: *const HANDLE, timeout_ms: u32) -> u32;

/// Resolved at run time: a static import would stop the exe loading on
/// Windows 10, where the export does not exist.
fn compositor_clock() -> Option<WaitForCompositorClock> {
    static CLOCK: OnceLock<Option<WaitForCompositorClock>> = OnceLock::new();
    *CLOCK.get_or_init(|| {
        // SAFETY: dcomp.dll is a system DLL (already loaded for DComp); the
        // export, when present, has exactly the documented signature above.
        unsafe {
            let module = LoadLibraryW(w!("dcomp.dll")).ok()?;
            let f = GetProcAddress(module, s!("DCompositionWaitForCompositorClock"))?;
            Some(std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                WaitForCompositorClock,
            >(f))
        }
    })
}

/// Flip-model composition swap chain, B8G8R8A8, frame latency 1.
pub struct SwapChain {
    chain: IDXGISwapChain2,
    waitable: HANDLE,
    size: (u32, u32),
}

// SAFETY: DXGI swap chains are free-threaded (`IDXGISwapChain2` is `Send`
// in the bindings); the waitable handle may be waited on from any thread and
// is closed only in Drop.
unsafe impl Send for SwapChain {}
// SAFETY: as above; the video thread is the only one that presents.
unsafe impl Sync for SwapChain {}

impl SwapChain {
    pub fn create(device: &ID3D11Device, width: u32, height: u32) -> Result<Self> {
        let dxgi: IDXGIDevice = device.cast()?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
            ..Default::default()
        };
        // SAFETY: the factory is the device's own; `desc` is fully set and
        // outlives the call; the waitable handle is owned by `Self`.
        unsafe {
            let adapter: IDXGIAdapter = dxgi.GetAdapter()?;
            let factory: IDXGIFactory2 = adapter.GetParent()?;
            let chain: IDXGISwapChain2 = factory
                .CreateSwapChainForComposition(device, &desc, None)?
                .cast()?;
            chain.SetMaximumFrameLatency(1)?;
            let waitable = chain.GetFrameLatencyWaitableObject();
            Ok(Self {
                chain,
                waitable,
                size: (width, height),
            })
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The swap chain as DComp visual content.
    pub fn content(&self) -> Result<IUnknown> {
        self.chain.cast()
    }

    /// Sleeps until the compositor can take another frame or `control` is
    /// set, whichever comes first (`control` wins ties).
    pub fn wait_ready(&self, control: &Signal) -> Wake {
        let handles = [control.0, self.waitable];
        // SAFETY: both handles are live for the duration of the call.
        let r = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
        if r == WAIT_OBJECT_0 {
            Wake::Control
        } else {
            Wake::Ready
        }
    }

    /// Shows the back buffer and keeps it on screen for `refreshes` display
    /// refreshes, blocking until then or until `control` is set.
    pub fn present(&self, refreshes: u32, control: &Signal) -> Result<Wake> {
        let Some(wait_tick) = compositor_clock() else {
            // Windows 10: no compositor clock; let DXGI pace by sync interval.
            // SAFETY: plain call on a live swap chain from the presenting
            // thread.
            unsafe { self.chain.Present(refreshes.clamp(1, 4), DXGI_PRESENT(0)) }.ok()?;
            return Ok(Wake::Ready);
        };
        // SAFETY: as above; sync interval 0 hands the frame to DWM, which
        // shows it at its next composition.
        unsafe { self.chain.Present(0, DXGI_PRESENT(0)) }.ok()?;
        for _ in 0..refreshes {
            // SAFETY: one live handle; the function blocks until the next
            // compositor tick (return value 1 = count) or the handle is set
            // (return value 0), never on a timer.
            let r = unsafe { wait_tick(1, &control.0, INFINITE) };
            if r == 0 {
                return Ok(Wake::Control);
            }
        }
        Ok(Wake::Ready)
    }

    fn back_buffer(&self) -> Result<ID3D11Texture2D> {
        // SAFETY: buffer 0 of a D3D11 flip-model chain is always the current
        // back buffer (the runtime rotates the allocation behind it).
        unsafe { self.chain.GetBuffer(0) }
    }
}

impl Drop for SwapChain {
    fn drop(&mut self) {
        // SAFETY: the waitable handle from GetFrameLatencyWaitableObject is
        // owned by the caller and closed once.
        unsafe {
            let _ = CloseHandle(self.waitable);
        }
    }
}

/// Video stream properties reported by the container.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamInfo {
    pub width: u32,
    pub height: u32,
    /// Frames per second as numerator / denominator (0/0 if unknown).
    pub fps: (u32, u32),
}

const FIRST_VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

fn unpack(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}

/// Hardware (DXVA) decoding reader for the first video stream. Samples carry
/// D3D11 textures in the decoder's native NV12; audio is never decoded.
pub struct Reader {
    reader: IMFSourceReader,
}

impl Reader {
    pub fn open(path: &Path, device: &ID3D11Device) -> Result<Self> {
        let mut token = 0u32;
        let mut manager: Option<IMFDXGIDeviceManager> = None;
        // SAFETY: both out-pointers are valid for the call.
        unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager) }?;
        let manager = manager.ok_or_else(windows::core::Error::empty)?;
        // SAFETY: `token` came from the manager just created.
        unsafe { manager.ResetDevice(device, token) }?;

        let mut attrs = None;
        // SAFETY: valid out-pointer.
        unsafe { MFCreateAttributes(&mut attrs, 2) }?;
        let attrs = attrs.ok_or_else(windows::core::Error::empty)?;
        // SAFETY: static GUID keys; the store AddRefs the manager; the path
        // HSTRING outlives the call; stream selection and media type calls
        // are on the live reader just created.
        unsafe {
            attrs.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, &manager)?;
            attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
            // Low-latency decoding hands each frame out as soon as it is
            // decoded; the decoder then keeps ~21 MB fewer 1080p surfaces
            // (logs/experiments.md). The cache files have no B-frames to
            // reorder, so output order is unchanged.
            attrs.SetUINT32(&MF_LOW_LATENCY, 1)?;
            // A plain buffered file stream: from the second loop on, reads
            // come from the system file cache. Media Foundation's own file
            // stream (by URL or `MFCreateFile`) read the disk on every loop.
            let stream = SHCreateStreamOnFileEx(
                &HSTRING::from(path.as_os_str()),
                (STGM_READ | STGM_SHARE_DENY_WRITE).0,
                0,
                false,
                None,
            )?;
            let file = MFCreateMFByteStreamOnStream(&stream)?;
            // No file name to guess the container from; cache files are MP4.
            file.cast::<IMFAttributes>()?
                .SetString(&MF_BYTESTREAM_CONTENT_TYPE, w!("video/mp4"))?;
            let reader = MFCreateSourceReaderFromByteStream(&file, &attrs)?;
            reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
            reader.SetStreamSelection(FIRST_VIDEO, true)?;
            let t = MFCreateMediaType()?;
            t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            reader.SetCurrentMediaType(FIRST_VIDEO, None, &t)?;
            Ok(Self { reader })
        }
    }

    /// Display size and frame rate from the container's native type (the
    /// decoder's output type may be padded, e.g. 1088 lines).
    pub fn info(&self) -> Result<StreamInfo> {
        // SAFETY: plain getters on a live reader / media type.
        unsafe {
            let t = self.reader.GetNativeMediaType(FIRST_VIDEO, 0)?;
            let (width, height) = unpack(t.GetUINT64(&MF_MT_FRAME_SIZE)?);
            let fps = t.GetUINT64(&MF_MT_FRAME_RATE).map(unpack).unwrap_or((0, 0));
            Ok(StreamInfo { width, height, fps })
        }
    }

    /// Next decoded frame and its timestamp (100 ns units), or `None` at the
    /// end of the stream.
    pub fn next(&self) -> Result<Option<(IMFSample, i64)>> {
        loop {
            let mut flags = 0u32;
            let mut time = 0i64;
            let mut sample: Option<IMFSample> = None;
            // SAFETY: all out-pointers are valid for this synchronous call.
            unsafe {
                self.reader.ReadSample(
                    FIRST_VIDEO,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut time),
                    Some(&mut sample),
                )?;
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return Ok(None);
            }
            // Stream ticks and format changes arrive without a sample.
            if let Some(sample) = sample {
                return Ok(Some((sample, time)));
            }
        }
    }

    /// Seeks to `hns` (100 ns units); the next frame is the key frame at or
    /// before it.
    pub fn seek(&self, hns: i64) -> Result<()> {
        let position = PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                    vt: VT_I8,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: PROPVARIANT_0_0_0 { hVal: hns },
                }),
            },
        };
        // SAFETY: GUID_NULL = 100 ns time format; a VT_I8 PROPVARIANT owns no
        // memory, so not clearing it leaks nothing.
        unsafe { self.reader.SetCurrentPosition(&GUID::zeroed(), &position) }
    }
}

/// D3D11 video processor that crops, scales and converts one decoded NV12
/// frame into the swap chain's back buffer in a single fixed-function pass.
pub struct Processor {
    context: ID3D11VideoContext1,
    video: ID3D11VideoDevice,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    output: ID3D11VideoProcessorOutputView,
    /// Input views by (texture pointer, array slice). Decoders use a fixed
    /// texture pool, so this stays small; each view keeps its texture alive,
    /// so a cached pointer is never reused by another texture.
    inputs: Vec<(usize, u32, ID3D11VideoProcessorInputView)>,
}

impl Processor {
    /// `src` is the crop rectangle in video pixels.
    pub fn new(
        device: &ID3D11Device,
        video: StreamInfo,
        src: RECT,
        chain: &SwapChain,
    ) -> Result<Self> {
        let (out_w, out_h) = chain.size();
        let rate = DXGI_RATIONAL {
            Numerator: video.fps.0.max(1),
            Denominator: video.fps.1.max(1),
        };
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: rate,
            InputWidth: video.width,
            InputHeight: video.height,
            OutputFrameRate: rate,
            OutputWidth: out_w,
            OutputHeight: out_h,
            Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
        };
        let dst = RECT {
            left: 0,
            top: 0,
            right: out_w as i32,
            bottom: out_h as i32,
        };
        let video_device: ID3D11VideoDevice = device.cast()?;
        // SAFETY: descriptors are fully initialised and outlive each call;
        // the back buffer belongs to a swap chain on this same device; all
        // setters target stream 0 of the processor created here.
        unsafe {
            let context: ID3D11VideoContext1 = device.GetImmediateContext()?.cast()?;
            let enumerator = video_device.CreateVideoProcessorEnumerator(&desc)?;
            let processor = video_device.CreateVideoProcessor(&enumerator, 0)?;
            let view_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            };
            let mut output = None;
            video_device.CreateVideoProcessorOutputView(
                &chain.back_buffer()?,
                &enumerator,
                &view_desc,
                Some(&mut output),
            )?;
            let output = output.ok_or_else(windows::core::Error::empty)?;

            context.VideoProcessorSetStreamFrameFormat(
                &processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            context.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&src));
            context.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&dst));
            context.VideoProcessorSetOutputTargetRect(&processor, true, Some(&dst));
            // No driver "enhancements" (denoise, edge, colour boost): they
            // cost GPU time and the source was encoded for plain display.
            context.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
            context.VideoProcessorSetStreamColorSpace1(
                &processor,
                0,
                DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
            );
            context.VideoProcessorSetOutputColorSpace1(
                &processor,
                DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
            );
            Ok(Self {
                context,
                video: video_device,
                enumerator,
                processor,
                output,
                inputs: Vec::new(),
            })
        }
    }

    /// Draws `sample` (a decoder output) into the back buffer.
    pub fn blit(&mut self, sample: &IMFSample) -> Result<()> {
        let view = self.input_view(sample)?;
        let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            pInputSurface: ManuallyDrop::new(Some(view)),
            ..Default::default()
        };
        // SAFETY: processor, output view and input view are live and belong
        // to this device; decode and blit are ordered on the one immediate
        // context, so the frame is complete before the processor reads it.
        let result = unsafe {
            self.context.VideoProcessorBlt(
                &self.processor,
                &self.output,
                0,
                std::slice::from_ref(&stream),
            )
        };
        // SAFETY: releases the reference moved into the stream struct above,
        // exactly once.
        unsafe { ManuallyDrop::drop(&mut stream.pInputSurface) };
        result
    }

    fn input_view(&mut self, sample: &IMFSample) -> Result<ID3D11VideoProcessorInputView> {
        // SAFETY: the sample's first buffer is a DXGI buffer (the reader has
        // a D3D manager); GetResource writes an AddRef'd texture pointer that
        // `from_raw` takes ownership of.
        let (texture, slice) = unsafe {
            let buffer: IMFDXGIBuffer = sample.GetBufferByIndex(0)?.cast()?;
            let mut raw = std::ptr::null_mut();
            buffer.GetResource(&ID3D11Texture2D::IID, &mut raw)?;
            (
                ID3D11Texture2D::from_raw(raw),
                buffer.GetSubresourceIndex()?,
            )
        };
        let key = texture.as_raw() as usize;
        if let Some((_, _, view)) = self
            .inputs
            .iter()
            .find(|(k, s, _)| *k == key && *s == slice)
        {
            return Ok(view.clone());
        }
        let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: slice,
                },
            },
        };
        let mut view = None;
        // SAFETY: texture and enumerator are live on this device; `desc`
        // outlives the call.
        unsafe {
            self.video.CreateVideoProcessorInputView(
                &texture,
                &self.enumerator,
                &desc,
                Some(&mut view),
            )
        }?;
        let view = view.ok_or_else(windows::core::Error::empty)?;
        // A decoder that reallocated its pool would grow this without bound.
        if self.inputs.len() >= 64 {
            self.inputs.clear();
        }
        self.inputs.push((key, slice, view.clone()));
        Ok(view)
    }
}

/// DirectComposition device for the wallpaper windows (UI thread).
pub struct Compositor {
    device: IDCompositionDesktopDevice,
}

/// One wallpaper window's composition target. Dropping it detaches.
pub struct Target {
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual2,
}

impl Compositor {
    pub fn new(d3d: &ID3D11Device) -> Result<Self> {
        let dxgi: IDXGIDevice = d3d.cast()?;
        // SAFETY: `dxgi` is a live rendering device for DComp.
        let device: IDCompositionDesktopDevice = unsafe { DCompositionCreateDevice2(&dxgi) }?;
        Ok(Self { device })
    }

    /// Shows `content` (`from` pixels) in `hwnd`, scaled to `to` pixels.
    pub fn target(
        &self,
        hwnd: HWND,
        content: &IUnknown,
        from: (u32, u32),
        to: (u32, u32),
    ) -> Result<Target> {
        let (sx, sy) = (
            to.0 as f32 / from.0.max(1) as f32,
            to.1 as f32 / from.1.max(1) as f32,
        );
        // SAFETY: `hwnd` is a live window of this thread; all interfaces are
        // live; the matrix pointer is valid for the call.
        unsafe {
            let target = self.device.CreateTargetForHwnd(hwnd, true)?;
            let visual = self.device.CreateVisual()?;
            visual.SetContent(content)?;
            if (sx - 1.0).abs() > f32::EPSILON || (sy - 1.0).abs() > f32::EPSILON {
                let m = Matrix3x2 {
                    M11: sx,
                    M12: 0.0,
                    M21: 0.0,
                    M22: sy,
                    M31: 0.0,
                    M32: 0.0,
                };
                visual.SetTransform2(&m)?;
            }
            target.SetRoot(&visual)?;
            Ok(Target {
                _target: target,
                _visual: visual,
            })
        }
    }

    pub fn commit(&self) -> Result<()> {
        // SAFETY: plain call on a live device.
        unsafe { self.device.Commit() }
    }
}
