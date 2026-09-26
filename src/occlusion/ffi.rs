//! Window enumeration and shell state for the occlusion check. All `unsafe`
//! for `occlusion/` lives here.
#![allow(unsafe_code)]

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetClassNameW, GetWindowLongW, GetWindowRect, IsIconic,
    IsWindowVisible, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};
use windows::core::BOOL;

use super::{Rect, WindowInfo};

/// Visible top-level windows, top of the z-order first. Invisible windows
/// are skipped here already because most top-level windows are hidden and
/// reading their details would be wasted work.
pub fn occluders() -> Vec<WindowInfo> {
    unsafe extern "system" fn collect(hwnd: HWND, out: LPARAM) -> BOOL {
        // SAFETY: `out` is the `&mut Vec<WindowInfo>` passed below, alive for
        // the whole EnumWindows call.
        let out = unsafe { &mut *(out.0 as *mut Vec<WindowInfo>) };
        // SAFETY: `hwnd` comes from EnumWindows.
        if unsafe { IsWindowVisible(hwnd) }.as_bool() {
            out.push(describe(hwnd));
        }
        BOOL::from(true)
    }
    let mut windows: Vec<WindowInfo> = Vec::new();
    // SAFETY: the callback matches WNDENUMPROC and only touches `windows`,
    // which outlives the call.
    unsafe {
        let _ = EnumWindows(
            Some(collect),
            LPARAM(&mut windows as *mut Vec<WindowInfo> as isize),
        );
    }
    windows
}

fn describe(hwnd: HWND) -> WindowInfo {
    // SAFETY: plain window queries; a window destroyed meanwhile makes them
    // fail or return zero, which reads as "not an occluder".
    let (minimized, ex) = unsafe {
        (
            IsIconic(hwnd).as_bool(),
            GetWindowLongW(hwnd, GWL_EXSTYLE) as u32,
        )
    };
    let transparent = WS_EX_TRANSPARENT.0 | WS_EX_LAYERED.0;
    WindowInfo {
        rect: bounds(hwnd),
        visible: true,
        minimized,
        cloaked: cloaked(hwnd),
        click_through: ex & transparent == transparent,
        tool: ex & WS_EX_TOOLWINDOW.0 != 0,
        class: class_of(hwnd),
    }
}

/// Visible bounds without the invisible resize borders, falling back to the
/// window rect for windows DWM does not frame.
fn bounds(hwnd: HWND) -> Rect {
    let mut r = RECT::default();
    // SAFETY: `r` is a RECT-sized out-buffer, as the attribute requires.
    let framed = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut r as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
    };
    if framed.is_err() {
        // SAFETY: valid out-pointer.
        unsafe {
            let _ = GetWindowRect(hwnd, &mut r);
        }
    }
    Rect {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}

fn cloaked(hwnd: HWND) -> bool {
    let mut value = 0u32;
    // SAFETY: DWMWA_CLOAKED writes one DWORD into `value`.
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&mut value as *mut u32).cast(),
            size_of::<u32>() as u32,
        )
    };
    ok.is_ok() && value != 0
}

fn class_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    // SAFETY: the buffer length is passed via the slice.
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// A fullscreen game / D3D app, presentation mode, or a fullscreen window on
/// the primary monitor, as the shell sees it for notification suppression.
pub fn fullscreen_app() -> bool {
    // SAFETY: no arguments; returns a state value.
    match unsafe { SHQueryUserNotificationState() } {
        Ok(s) => s == QUNS_BUSY || s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE,
        Err(_) => false,
    }
}
