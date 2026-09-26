//! Thin wrappers over D3D11, Media Foundation's Media Engine and
//! DirectComposition. All `unsafe` for `playback/` lives here.
//!
//! Pipeline (ADR-003): hardware decoder -> Media Engine windowless swap chain
//! -> one DComp surface -> one DComp visual per wallpaper window. No render
//! pass of our own; the engine presents frames and DWM composes them.
#![allow(unsafe_code)]

use windows::Win32::Foundation::{HANDLE, HMODULE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_9_3, D3D_FEATURE_LEVEL_10_0,
    D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
    D3D11CreateDevice, ID3D11Device, ID3D11Multithread,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice2, IDCompositionDesktopDevice, IDCompositionTarget,
    IDCompositionVisual2,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Media::MediaFoundation::{
    CLSID_MFMediaEngineClassFactory, IMFAttributes, IMFDXGIDeviceManager, IMFMediaEngine,
    IMFMediaEngineClassFactory, IMFMediaEngineEx, IMFMediaEngineNotify, IMFMediaEngineNotify_Impl,
    MF_MEDIA_ENGINE_CALLBACK, MF_MEDIA_ENGINE_DXGI_MANAGER, MF_VERSION, MFARGB, MFCreateAttributes,
    MFCreateDXGIDeviceManager, MFSTARTUP_LITE, MFShutdown, MFStartup, MFVideoNormalizedRect,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{BSTR, IUnknown, Interface, Result, implement};
use windows_numerics::Matrix3x2;

/// Message the engine's callback posts to the host window:
/// `wParam` = `MF_MEDIA_ENGINE_EVENT`, `lParam` = `param1`.
pub const WM_MEDIA_EVENT: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 2;

/// Keeps COM and Media Foundation initialised for the thread's lifetime.
pub struct Platform(());

impl Platform {
    pub fn start() -> Result<Self> {
        // SAFETY: first COM call on this thread; the UI thread is an STA.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;
        // SAFETY: MF startup with the SDK version the bindings were built for.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE) }?;
        Ok(Self(()))
    }
}

impl Drop for Platform {
    fn drop(&mut self) {
        // SAFETY: balances the MFStartup above; COM stays up until exit.
        unsafe {
            let _ = MFShutdown();
        }
    }
}

/// Media Engine callback. Runs on MF worker threads, so it only posts the
/// event to the UI thread's host window and returns immediately.
#[implement(IMFMediaEngineNotify)]
struct Notify {
    host: isize,
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, param1: usize, _param2: u32) -> Result<()> {
        if !super::is_interesting(event) {
            return Ok(());
        }
        // SAFETY: PostMessage is thread-safe and fails cleanly if the host
        // window no longer exists.
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.host as *mut _)),
                WM_MEDIA_EVENT,
                WPARAM(event as usize),
                LPARAM(param1 as isize),
            );
        }
        Ok(())
    }
}

/// D3D11 device with video support, shared by the decoder and DComp.
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
    // Media Foundation uses the device from its own threads.
    let mt: ID3D11Multithread = device.cast()?;
    // SAFETY: plain setter on a live interface.
    unsafe {
        let _ = mt.SetMultithreadProtected(true);
    }
    Ok(device)
}

/// Creates a Media Engine that decodes on `device` and reports to `host`.
pub fn create_engine(device: &ID3D11Device, host: HWND) -> Result<IMFMediaEngineEx> {
    let mut token = 0u32;
    let mut manager: Option<IMFDXGIDeviceManager> = None;
    // SAFETY: both out-pointers are valid for the call.
    unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager) }?;
    let manager = manager.ok_or_else(windows::core::Error::empty)?;
    // SAFETY: `token` came from the manager just created.
    unsafe { manager.ResetDevice(device, token) }?;

    let mut attrs: Option<IMFAttributes> = None;
    // SAFETY: valid out-pointer.
    unsafe { MFCreateAttributes(&mut attrs, 2) }?;
    let attrs = attrs.ok_or_else(windows::core::Error::empty)?;
    let notify: IMFMediaEngineNotify = Notify {
        host: host.0 as isize,
    }
    .into();
    // SAFETY: GUID keys are static; the attribute store AddRefs the values.
    unsafe {
        attrs.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
        attrs.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &manager)?;
    }

    // SAFETY: standard in-proc COM activation of the documented CLSID.
    let factory: IMFMediaEngineClassFactory =
        unsafe { CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER) }?;
    // SAFETY: attributes are fully populated above.
    let engine: IMFMediaEngine = unsafe { factory.CreateInstance(0, &attrs) }?;
    engine.cast()
}

pub fn configure(engine: &IMFMediaEngineEx) -> Result<()> {
    // SAFETY: plain setters on a live engine.
    unsafe {
        engine.SetLoop(true)?;
        engine.SetMuted(true)?;
        engine.SetAutoPlay(false)?;
    }
    Ok(())
}

pub fn set_source(engine: &IMFMediaEngineEx, url: &str) -> Result<()> {
    // SAFETY: the BSTR lives across the call; the engine copies it.
    unsafe { engine.SetSource(&BSTR::from(url)) }
}

pub fn play(engine: &IMFMediaEngineEx) -> Result<()> {
    // SAFETY: plain call on a live engine.
    unsafe { engine.Play() }
}

pub fn pause(engine: &IMFMediaEngineEx) -> Result<()> {
    // SAFETY: plain call on a live engine.
    unsafe { engine.Pause() }
}

pub fn shutdown(engine: &IMFMediaEngineEx) {
    // SAFETY: final call on the engine; it is dropped right after.
    unsafe {
        let _ = engine.Shutdown();
    }
}

pub fn native_size(engine: &IMFMediaEngineEx) -> Option<(u32, u32)> {
    let (mut w, mut h) = (0u32, 0u32);
    // SAFETY: out-pointers are valid for the call.
    unsafe { engine.GetNativeVideoSize(Some(&mut w), Some(&mut h)) }.ok()?;
    (w > 0 && h > 0).then_some((w, h))
}

/// `MF_MEDIA_ENGINE_ERR` code and extended HRESULT of the last error.
pub fn last_error(engine: &IMFMediaEngineEx) -> Option<(u16, i32)> {
    // SAFETY: plain getters on a live engine / error object.
    unsafe {
        let err = engine.GetError().ok()?;
        Some((
            err.GetErrorCode(),
            err.GetExtendedErrorCode().err().map_or(0, |e| e.code().0),
        ))
    }
}

/// Switches to windowless swap-chain mode, sizes the output and returns the
/// swap-chain handle for DComp. `src` is a normalised crop of the video.
pub fn windowless_output(
    engine: &IMFMediaEngineEx,
    src: [f32; 4],
    width: u32,
    height: u32,
) -> Result<HANDLE> {
    let src = MFVideoNormalizedRect {
        left: src[0],
        top: src[1],
        right: src[2],
        bottom: src[3],
    };
    let dst = RECT {
        left: 0,
        top: 0,
        right: width as i32,
        bottom: height as i32,
    };
    let black = MFARGB {
        rgbBlue: 0,
        rgbGreen: 0,
        rgbRed: 0,
        rgbAlpha: 255,
    };
    // SAFETY: rect/colour pointers are valid for the call; order follows the
    // Microsoft MediaEngineDCompWin32Sample (enable, size, get handle).
    unsafe {
        engine.EnableWindowlessSwapchainMode(true)?;
        engine.UpdateVideoStream(Some(&src), Some(&dst), Some(&black))?;
        engine.GetVideoSwapchainHandle()
    }
}

/// DirectComposition device plus the shared video surface.
pub struct Compositor {
    device: IDCompositionDesktopDevice,
    surface: Option<IUnknown>,
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
        Ok(Self {
            device,
            surface: None,
        })
    }

    pub fn set_swapchain(&mut self, handle: HANDLE) -> Result<()> {
        // SAFETY: `handle` is the engine's composition swap-chain handle.
        self.surface = Some(unsafe { self.device.CreateSurfaceFromHandle(handle) }?);
        Ok(())
    }

    pub fn has_surface(&self) -> bool {
        self.surface.is_some()
    }

    /// Shows the shared surface in `hwnd`, scaled from `from` to `to` pixels.
    pub fn target(&self, hwnd: HWND, from: (u32, u32), to: (u32, u32)) -> Result<Target> {
        let surface = self
            .surface
            .as_ref()
            .ok_or_else(windows::core::Error::empty)?;
        let (sx, sy) = (
            to.0 as f32 / from.0.max(1) as f32,
            to.1 as f32 / from.1.max(1) as f32,
        );
        // SAFETY: `hwnd` is a live window of this thread; all interfaces are
        // live; the matrix pointer is valid for the call.
        unsafe {
            let target = self.device.CreateTargetForHwnd(hwnd, true)?;
            let visual = self.device.CreateVisual()?;
            visual.SetContent(surface)?;
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
