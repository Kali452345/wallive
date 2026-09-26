//! Thin Win32 wrappers for the host window, the message loop, the Explorer
//! WinEvent hook and console Ctrl+C. All `unsafe` for `runtime/` lives here.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicIsize, AtomicU32, AtomicU64, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, SetConsoleCtrlHandler,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    EVENT_OBJECT_CREATE, EVENT_OBJECT_REORDER, GetMessageW, MSG, OBJID_WINDOW, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, TranslateMessage,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP, WM_CLOSE, WM_DESTROY, WM_DISPLAYCHANGE,
    WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{BOOL, PCWSTR, w};

/// Things the safe runtime reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Explorer (re)created the taskbar, i.e. Explorer restarted.
    TaskbarCreated,
    /// Monitor added/removed or resolution changed.
    DisplayChanged,
    /// Explorer created/destroyed/showed/hid/reordered a window.
    DesktopChanged,
}

const WM_WAKE: u32 = WM_APP + 1;

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
/// Host window handle, for the console Ctrl+C handler thread.
static HOST: AtomicIsize = AtomicIsize::new(0);
/// Explorer window events received (for the spike's cost measurement).
static WIN_EVENTS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static QUEUE: RefCell<VecDeque<Event>> = const { RefCell::new(VecDeque::new()) };
}

/// Queues an event once; repeated events of the same kind coalesce until the
/// loop drains them.
///
/// `TaskbarCreated`, `WM_DISPLAYCHANGE` and out-of-context WinEvents are all
/// delivered *inside* `GetMessageW`, which then keeps waiting for a posted
/// message. So when the queue goes from empty to non-empty, post a no-op
/// `WM_WAKE` to make the loop return and drain it. Without this an Explorer
/// restart went unhandled until some unrelated message arrived (logs/errors.md).
fn push(event: Event) {
    let was_empty = QUEUE.with_borrow_mut(|q| {
        let was_empty = q.is_empty();
        if !q.contains(&event) {
            q.push_back(event);
        }
        was_empty
    });
    if was_empty {
        let host = HWND(HOST.load(Ordering::Relaxed) as *mut _);
        // SAFETY: posting to our own window; fails cleanly if it is gone.
        unsafe {
            let _ = PostMessageW(Some(host), WM_WAKE, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn win_event_count() -> u64 {
    WIN_EVENTS.load(Ordering::Relaxed)
}

unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let taskbar_created = TASKBAR_CREATED.load(Ordering::Relaxed);
    match msg {
        WM_DISPLAYCHANGE => push(Event::DisplayChanged),
        WM_WAKE => {}
        WM_CLOSE => {
            // SAFETY: destroying our own host window on its own thread.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return LRESULT(0);
        }
        WM_DESTROY => {
            // SAFETY: posts WM_QUIT to this thread's queue.
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        m if m != 0 && m == taskbar_created => push(Event::TaskbarCreated),
        _ => {}
    }
    // SAFETY: forwarding the unchanged arguments Windows gave us.
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// Hidden top-level window. It must be top-level (not message-only) because
/// `TaskbarCreated` and `WM_DISPLAYCHANGE` are broadcast to top-level windows.
pub struct Host(());

impl Host {
    pub fn create() -> windows::core::Result<Self> {
        // SAFETY: static null-terminated string.
        let msg = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
        TASKBAR_CREATED.store(msg, Ordering::Relaxed);

        // SAFETY: null name returns this exe's module handle.
        let module = unsafe { GetModuleHandleW(PCWSTR::null()) }?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(host_proc),
            hInstance: module.into(),
            lpszClassName: w!("WalliveHost"),
            ..Default::default()
        };
        // SAFETY: `class` is fully initialised with static strings.
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err(windows::core::Error::from_thread());
        }
        // SAFETY: registered class, static strings, no parent; never shown.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("WalliveHost"),
                w!("Wallive"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(module.into()),
                None,
            )
        }?;
        HOST.store(hwnd.0 as isize, Ordering::Relaxed);
        Ok(Self(()))
    }

    /// Makes Ctrl+C / console close post `WM_CLOSE` to the host so the loop
    /// exits cleanly and wallpaper windows are destroyed.
    pub fn close_on_ctrl_c(&self) {
        unsafe extern "system" fn handler(kind: u32) -> BOOL {
            if kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT || kind == CTRL_CLOSE_EVENT {
                let host = HWND(HOST.load(Ordering::Relaxed) as *mut _);
                // SAFETY: PostMessage is thread-safe and fails cleanly if the
                // host window is gone.
                unsafe {
                    let _ = PostMessageW(Some(host), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                return BOOL::from(true);
            }
            BOOL::from(false)
        }
        // SAFETY: registering a handler with the correct signature.
        unsafe {
            let _ = SetConsoleCtrlHandler(Some(handler), true);
        }
    }
}

/// Out-of-context WinEvent hook limited to one process. Unhooks on drop.
pub struct ExplorerHook(HWINEVENTHOOK);

impl ExplorerHook {
    /// Observe-only: CREATE, DESTROY, SHOW, HIDE and REORDER for windows owned
    /// by `pid`. Callbacks run on this thread during message retrieval.
    pub fn install(pid: u32) -> Option<Self> {
        // SAFETY: out-of-context hook, so no module handle; the callback has
        // the WINEVENTPROC signature and lives for the whole program.
        let hook = unsafe {
            SetWinEventHook(
                EVENT_OBJECT_CREATE,
                EVENT_OBJECT_REORDER,
                None,
                Some(on_win_event),
                pid,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        (!hook.is_invalid()).then_some(Self(hook))
    }
}

impl Drop for ExplorerHook {
    fn drop(&mut self) {
        // SAFETY: the handle came from SetWinEventHook and is unhooked once.
        unsafe {
            let _ = UnhookWinEvent(self.0);
        }
    }
}

unsafe extern "system" fn on_win_event(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    // Only whole windows matter, not accessible sub-objects (e.g. taskbar
    // buttons), which Explorer reports in large numbers.
    if id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 {
        return;
    }
    WIN_EVENTS.fetch_add(1, Ordering::Relaxed);
    push(Event::DesktopChanged);
}

/// Runs the message loop, handing queued events to `handle` after each
/// dispatched message. Returns when the host window is destroyed.
pub fn run_loop(mut handle: impl FnMut(Event)) {
    let mut msg = MSG::default();
    loop {
        // SAFETY: `msg` is a valid out-parameter for this thread's queue.
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if got.0 <= 0 {
            break;
        }
        // SAFETY: `msg` was just filled by GetMessageW.
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        // Handled outside the window procedure, so handlers can make
        // re-entrant Win32 calls freely.
        while let Some(event) = QUEUE.with_borrow_mut(VecDeque::pop_front) {
            handle(event);
        }
    }
}
