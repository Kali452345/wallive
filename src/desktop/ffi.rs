//! Thin Win32 wrappers for finding Explorer's desktop windows and placing
//! wallpaper windows in them. All `unsafe` for `desktop/` lives here.
#![allow(unsafe_code)]

use std::sync::OnceLock;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, EnumDisplayMonitors, HDC, HMONITOR, MapWindowPoints,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowExW, FindWindowW, GW_CHILD,
    GW_HWNDNEXT, GWL_EXSTYLE, GetClassNameW, GetParent, GetWindow, GetWindowLongW,
    GetWindowThreadProcessId, HWND_BOTTOM, HWND_TOP, IsWindow, LWA_ALPHA, RegisterClassW,
    SMTO_NORMAL, SW_SHOWNA, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SendMessageTimeoutW,
    SetLayeredWindowAttributes, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WNDCLASSW, WS_CHILD,
    WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};
use windows::core::{BOOL, PCWSTR, w};

use super::tree::{Snapshot, Window};

const WALLPAPER_CLASS: PCWSTR = w!("WalliveWallpaper");
// Same strings as `tree::ICONS_CLASS` / `tree::WORKER_CLASS`, as wide literals.
const ICONS_CLASS_W: PCWSTR = w!("SHELLDLL_DefView");
const WORKER_CLASS_W: PCWSTR = w!("WorkerW");

/// Undocumented Progman message that makes Explorer create the wallpaper
/// `WorkerW`. `wParam = 0xD, lParam = 1` is what Lively and others send; unlike
/// `0, 0` it does not depend on the "animate controls" visual-effects setting.
const SPAWN_WORKER: u32 = 0x052C;
const SPAWN_TIMEOUT_MS: u32 = 1_000;

/// Guards against looping forever on a tree that changes while we walk it.
const MAX_WINDOWS: usize = 256;

/// Spike fill colour (BGR): a teal that is easy to spot in screenshots.
const SPIKE_FILL: COLORREF = COLORREF(0x0080_8000);

pub fn find_progman() -> Option<HWND> {
    // SAFETY: static null-terminated class name; no window name.
    unsafe { FindWindowW(w!("Progman"), PCWSTR::null()) }
        .ok()
        .filter(|h| !h.is_invalid())
}

pub fn find_taskbar() -> Option<HWND> {
    // SAFETY: static null-terminated class name; no window name.
    unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }
        .ok()
        .filter(|h| !h.is_invalid())
}

pub fn process_id(window: HWND) -> Option<u32> {
    let mut pid = 0u32;
    // SAFETY: `pid` outlives the call; a dead handle returns 0 and leaves it 0.
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    (pid != 0).then_some(pid)
}

pub fn is_alive(window: HWND) -> bool {
    // SAFETY: IsWindow accepts any handle value.
    unsafe { IsWindow(Some(window)) }.as_bool()
}

pub fn parent_of(window: HWND) -> Option<HWND> {
    // SAFETY: GetParent accepts any handle and fails for dead ones.
    unsafe { GetParent(window) }
        .ok()
        .filter(|h| !h.is_invalid())
}

/// Asks Progman to create the wallpaper `WorkerW`. The answer can be lazy, so
/// callers re-check on the next Explorer window event instead of sleeping.
pub fn request_worker(progman: HWND) {
    // SAFETY: plain message with integer arguments and a bounded timeout; the
    // optional result pointer is not used.
    unsafe {
        SendMessageTimeoutW(
            progman,
            SPAWN_WORKER,
            WPARAM(0xD),
            LPARAM(1),
            SMTO_NORMAL,
            SPAWN_TIMEOUT_MS,
            None,
        );
    }
}

pub fn snapshot(progman: HWND) -> Snapshot<HWND> {
    let mut top_level_workers = Vec::new();
    let mut after: Option<HWND> = None;
    while top_level_workers.len() < MAX_WINDOWS {
        // SAFETY: `after` is None or a handle returned by the previous call;
        // the class name is a static null-terminated string.
        let next = unsafe { FindWindowExW(None, after, WORKER_CLASS_W, PCWSTR::null()) };
        let Some(next) = next.ok().filter(|h| !h.is_invalid()) else {
            break;
        };
        top_level_workers.push(describe(next));
        after = Some(next);
    }

    Snapshot {
        progman,
        progman_no_redirection: ex_style(progman) & WS_EX_NOREDIRECTIONBITMAP.0 != 0,
        progman_children: children(progman).into_iter().map(describe).collect(),
        top_level_workers,
    }
}

/// Direct children of `parent`, topmost first.
pub fn children(parent: HWND) -> Vec<HWND> {
    let mut out = Vec::new();
    // SAFETY: GetWindow accepts any handle and fails once the chain ends.
    let mut next = unsafe { GetWindow(parent, GW_CHILD) };
    while let Ok(child) = next {
        if child.is_invalid() || out.len() >= MAX_WINDOWS {
            break;
        }
        out.push(child);
        // SAFETY: as above.
        next = unsafe { GetWindow(child, GW_HWNDNEXT) };
    }
    out
}

pub fn class_of(window: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: the slice length bounds how much GetClassNameW writes.
    let len = unsafe { GetClassNameW(window, &mut buf) };
    let len = usize::try_from(len).unwrap_or(0).min(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

fn describe(window: HWND) -> Window<HWND> {
    // SAFETY: static null-terminated class name; searching a dead window's
    // children just finds nothing.
    let icons = unsafe { FindWindowExW(Some(window), None, ICONS_CLASS_W, PCWSTR::null()) };
    Window {
        handle: window,
        class: class_of(window),
        holds_icons: icons.is_ok_and(|h| !h.is_invalid()),
    }
}

fn ex_style(window: HWND) -> u32 {
    // SAFETY: reading a window long is valid for any handle (0 when dead).
    (unsafe { GetWindowLongW(window, GWL_EXSTYLE) }) as u32
}

/// Monitor rectangles in physical screen coordinates (the manifest makes the
/// process per-monitor DPI aware).
pub fn monitor_rects() -> Vec<RECT> {
    unsafe extern "system" fn collect(_: HMONITOR, _: HDC, rect: *mut RECT, out: LPARAM) -> BOOL {
        // SAFETY: `out` is the `&mut Vec<RECT>` passed below, alive for the
        // whole EnumDisplayMonitors call; `rect` is valid for this callback.
        unsafe {
            let out = &mut *(out.0 as *mut Vec<RECT>);
            out.push(*rect);
        }
        BOOL::from(true)
    }
    let mut rects: Vec<RECT> = Vec::new();
    // SAFETY: the callback matches MONITORENUMPROC and only touches `rects`,
    // which outlives the call.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut rects as *mut Vec<RECT> as isize),
        );
    }
    rects
}

/// Converts a screen rectangle into `parent`'s client coordinates.
pub fn screen_to_client(parent: HWND, rect: RECT) -> RECT {
    let mut pts = [
        POINT {
            x: rect.left,
            y: rect.top,
        },
        POINT {
            x: rect.right,
            y: rect.bottom,
        },
    ];
    // SAFETY: `pts` is a valid mutable slice for the duration of the call.
    unsafe { MapWindowPoints(None, Some(parent), &mut pts) };
    RECT {
        left: pts[0].x,
        top: pts[0].y,
        right: pts[1].x,
        bottom: pts[1].y,
    }
}

unsafe extern "system" fn wallpaper_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: forwarding the unchanged arguments Windows gave us.
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn register_class() -> windows::core::Result<()> {
    static REGISTERED: OnceLock<bool> = OnceLock::new();
    let ok = *REGISTERED.get_or_init(|| {
        // SAFETY: null name returns this exe's module handle.
        let Ok(module) = (unsafe { GetModuleHandleW(PCWSTR::null()) }) else {
            return false;
        };
        // SAFETY: creating a GDI brush; it lives for the process lifetime as
        // the class background, which is intended.
        let brush = unsafe { CreateSolidBrush(SPIKE_FILL) };
        let class = WNDCLASSW {
            lpfnWndProc: Some(wallpaper_proc),
            hInstance: module.into(),
            hbrBackground: brush,
            lpszClassName: WALLPAPER_CLASS,
            ..Default::default()
        };
        // SAFETY: `class` is fully initialised and its strings are static.
        unsafe { RegisterClassW(&class) != 0 }
    });
    if ok {
        Ok(())
    } else {
        Err(windows::core::Error::from_thread())
    }
}

fn create(
    ex: WINDOW_EX_STYLE,
    extra_style: windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE,
    parent: HWND,
    at: RECT,
) -> windows::core::Result<HWND> {
    register_class()?;
    // SAFETY: null name returns this exe's module handle.
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }?;
    // SAFETY: the class is registered, `parent` is a window handle (creation
    // fails cleanly if it died), and every string is static.
    unsafe {
        CreateWindowExW(
            ex | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            WALLPAPER_CLASS,
            w!("Wallive"),
            WS_CHILD | WS_CLIPSIBLINGS | extra_style,
            at.left,
            at.top,
            at.right - at.left,
            at.bottom - at.top,
            Some(parent),
            None,
            Some(module.into()),
            None,
        )
    }
}

/// Hidden, opaque, layered, click-through child of Progman (raised layout).
/// Progman has no redirection surface, so this layered window gives our
/// content somewhere to be composed. Returns `None` if Windows refuses the
/// layered style (e.g. no Windows 8+ manifest entry).
pub fn create_layered_holder(progman: HWND, at: RECT) -> Option<HWND> {
    let holder = create(
        WS_EX_LAYERED | WS_EX_TRANSPARENT,
        WS_CLIPCHILDREN,
        progman,
        at,
    )
    .ok()?;
    let layered = ex_style(holder) & WS_EX_LAYERED.0 != 0;
    // SAFETY: `holder` is a live window we just created; a layered window is
    // invisible until its alpha is set.
    let opaque = layered
        && unsafe { SetLayeredWindowAttributes(holder, COLORREF(0), 255, LWA_ALPHA) }.is_ok();
    if !opaque {
        destroy(holder);
        return None;
    }
    Some(holder)
}

/// Hidden child that will carry the video (the DComp target later).
pub fn create_surface(parent: HWND, at: RECT) -> windows::core::Result<HWND> {
    create(WINDOW_EX_STYLE(0), Default::default(), parent, at)
}

pub fn show(window: HWND) {
    // SAFETY: showing without activation; harmless on a dead handle.
    unsafe {
        let _ = ShowWindow(window, SW_SHOWNA);
    }
}

pub fn destroy(window: HWND) {
    // SAFETY: we only destroy windows this thread created; failure on an
    // already-destroyed window is ignored.
    unsafe {
        let _ = DestroyWindow(window);
    }
}

/// Puts `window` directly below `above` among its siblings, or at the top of
/// them when `above` is `None`.
pub fn place_below(window: HWND, above: Option<HWND>) {
    // SAFETY: z-order change only; no pointers. Dead handles make it fail and
    // the next Explorer event retries.
    unsafe {
        let _ = SetWindowPos(
            window,
            Some(above.unwrap_or(HWND_TOP)),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

pub fn send_to_bottom(window: HWND) {
    // SAFETY: z-order change only; no pointers.
    unsafe {
        let _ = SetWindowPos(
            window,
            Some(HWND_BOTTOM),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}
