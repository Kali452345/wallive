//! Thin wrappers for the notification-area icon, popup menu, Run-key
//! autostart, single-instance mutex, console attach, file-open dialog and
//! job object. All `unsafe` for `shell/` lives here.
#![allow(unsafe_code)]

use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, POINT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject, HGDIOBJ,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
    CoInitializeEx, CoTaskMemFree,
};
use windows::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE,
};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FileOpenDialog, IFileOpenDialog,
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, SIGDN_FILESYSPATH, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, AppendMenuW, CreateIconIndirect, CreatePopupMenu,
    DestroyIcon, DestroyMenu, FindWindowW, GetCursorPos, GetSystemMetrics, HICON, HMENU, ICONINFO,
    MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, PostMessageW, SM_CXSMICON,
    SMTO_ABORTIFHUNG, SendMessageTimeoutW, SetForegroundWindow, TPM_BOTTOMALIGN, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_CLOSE, WM_COPYDATA, WM_NULL,
};
use windows::core::{BOOL, HSTRING, PCWSTR, w};

use super::MenuItem;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn fill(dst: &mut [u16], s: &str) {
    let n = dst.len() - 1;
    for (d, c) in dst.iter_mut().zip(s.encode_utf16().take(n).chain([0])) {
        *d = c;
    }
}

/// The tray icon image. Destroyed on drop.
pub struct Icon(HICON);

impl Icon {
    /// Small-icon size for the current DPI, drawn by [`super::icon_pixels`].
    pub fn new() -> windows::core::Result<Self> {
        // SAFETY: plain metric query.
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64);
        let pixels = super::icon_pixels(size as u32);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        // SAFETY: `info` describes a 32-bpp top-down DIB; `bits` receives its
        // pixel memory of exactly size*size u32s, which we fill before use.
        // The icon copies both bitmaps, so they are deleted afterwards.
        unsafe {
            let color = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u32>(), pixels.len());
            let mask = CreateBitmap(size, size, 1, 1, None);
            let icon = CreateIconIndirect(&ICONINFO {
                fIcon: BOOL::from(true),
                xHotspot: 0,
                yHotspot: 0,
                hbmMask: mask,
                hbmColor: color,
            });
            let _ = DeleteObject(HGDIOBJ(mask.0));
            let _ = DeleteObject(HGDIOBJ(color.0));
            Ok(Self(icon?))
        }
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        // SAFETY: created by CreateIconIndirect, destroyed once.
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}

/// Notification-area icon owned by `hwnd`. Clicks arrive as `message` with
/// `NOTIFYICON_VERSION_4` semantics. Removed on drop.
pub struct Tray {
    data: NOTIFYICONDATAW,
}

impl Tray {
    pub fn add(hwnd: HWND, message: u32, icon: &Icon, tip: &str) -> Option<Self> {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: message,
            hIcon: icon.0,
            ..Default::default()
        };
        fill(&mut data.szTip, tip);
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let tray = Self { data };
        tray.show().then_some(tray)
    }

    /// (Re-)adds the icon, e.g. after Explorer restarted (`TaskbarCreated`).
    pub fn show(&self) -> bool {
        // SAFETY: `data` is fully initialised and outlives the calls.
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &self.data).as_bool()
                && Shell_NotifyIconW(NIM_SETVERSION, &self.data).as_bool()
        }
    }

    pub fn set_tip(&mut self, tip: &str) {
        fill(&mut self.data.szTip, tip);
        // SAFETY: as above.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.data);
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // SAFETY: removes the icon identified by hWnd + uID.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data);
        }
    }
}

/// Shows `items` as a popup menu at `at` (or the cursor) and returns the
/// chosen item id. Blocks in the menu's modal loop.
pub fn show_menu(hwnd: HWND, items: &[MenuItem], at: Option<(i32, i32)>) -> Option<u32> {
    // SAFETY: the menu is created, used and destroyed here (destroying it
    // also destroys its submenus).
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        fill_menu(menu, items);
        let (x, y) = at.unwrap_or_else(|| {
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            (p.x, p.y)
        });
        // Required so the menu closes when the user clicks elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let id = TrackPopupMenuEx(
            menu,
            (TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN).0,
            x,
            y,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        (id.0 > 0).then_some(id.0 as u32)
    }
}

fn fill_menu(menu: HMENU, items: &[MenuItem]) {
    for item in items {
        // SAFETY: `menu` is a live popup menu; labels are null-terminated
        // buffers that outlive AppendMenuW, which copies them. A submenu
        // appended with MF_POPUP is owned (and destroyed) by its parent.
        let _ = unsafe {
            match item {
                MenuItem::Separator => AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()),
                MenuItem::Item {
                    command,
                    label,
                    checked,
                    enabled,
                } => {
                    let mut flags = MF_STRING;
                    if *checked {
                        flags |= MF_CHECKED;
                    }
                    if !*enabled {
                        flags |= MF_GRAYED;
                    }
                    let text = wide(label);
                    AppendMenuW(menu, flags, command.id() as usize, PCWSTR(text.as_ptr()))
                }
                MenuItem::Submenu { label, items } => match CreatePopupMenu() {
                    Ok(sub) => {
                        fill_menu(sub, items);
                        let text = wide(label);
                        AppendMenuW(
                            menu,
                            MF_STRING | MF_POPUP,
                            sub.0 as usize,
                            PCWSTR(text.as_ptr()),
                        )
                    }
                    Err(e) => Err(e),
                },
            }
        };
    }
}

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const RUN_VALUE: PCWSTR = w!("Wallive");

/// Start with Windows through the per-user Run key (no admin rights).
pub struct Autostart;

impl Autostart {
    fn command(exe: &Path) -> String {
        format!("\"{}\"", exe.display())
    }

    /// True when the Run value points at `exe`.
    pub fn is_enabled(exe: &Path) -> bool {
        let mut buf = [0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        // SAFETY: `buf` / `len` describe a writable buffer in bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                RUN_VALUE,
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut len),
            )
        };
        if status.is_err() {
            return false;
        }
        let chars = (len as usize / 2).saturating_sub(1).min(buf.len());
        String::from_utf16_lossy(&buf[..chars]).eq_ignore_ascii_case(&Self::command(exe))
    }

    pub fn set(exe: &Path, enabled: bool) -> windows::core::Result<()> {
        // SAFETY: static key/value names; the data buffer is a
        // null-terminated UTF-16 string whose byte length is passed.
        let status = unsafe {
            if enabled {
                let data = wide(&Self::command(exe));
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    RUN_KEY,
                    RUN_VALUE,
                    REG_SZ.0,
                    Some(data.as_ptr().cast()),
                    (data.len() * 2) as u32,
                )
            } else {
                RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE)
            }
        };
        status.ok()
    }
}

/// Named mutex held for the process lifetime; a second instance sees it.
pub struct SingleInstance(HANDLE);

impl SingleInstance {
    /// `None` when another instance in this session already holds it.
    pub fn acquire() -> Option<Self> {
        // SAFETY: static name; the handle is closed on drop.
        let handle =
            unsafe { CreateMutexW(None, false, w!(r"Local\Wallive.SingleInstance")) }.ok()?;
        // SAFETY: reads the error left by CreateMutexW just above.
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            // SAFETY: our handle to the existing mutex, closed once.
            unsafe {
                let _ = CloseHandle(handle);
            }
            return None;
        }
        Some(Self(handle))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        // SAFETY: closed once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Asks a running instance to exit. Returns whether one was found.
pub fn close_running() -> bool {
    // SAFETY: static class name; posting to another process's window is
    // allowed for WM_CLOSE at the same integrity level.
    unsafe {
        match FindWindowW(w!("WalliveHost"), PCWSTR::null()) {
            Ok(h) if !h.is_invalid() => {
                PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok()
            }
            _ => false,
        }
    }
}

/// Hands `videos` to a running instance (`WM_COPYDATA`, one path per line).
/// Returns whether one accepted them.
pub fn send_to_running(videos: &[PathBuf], tag: usize) -> bool {
    let mut units: Vec<u16> = Vec::new();
    for (i, video) in videos.iter().enumerate() {
        if i > 0 {
            units.push(u16::from(b'\n'));
        }
        units.extend(video.as_os_str().encode_wide());
    }
    let data = COPYDATASTRUCT {
        dwData: tag,
        cbData: (units.len() * 2) as u32,
        lpData: units.as_ptr() as *mut _,
    };
    let mut result = 0usize;
    // SAFETY: `data` and `units` outlive the synchronous send; the receiver
    // only reads cbData bytes.
    unsafe {
        let Ok(host) = FindWindowW(w!("WalliveHost"), PCWSTR::null()) else {
            return false;
        };
        SendMessageTimeoutW(
            host,
            WM_COPYDATA,
            WPARAM(0),
            LPARAM(&data as *const COPYDATASTRUCT as isize),
            SMTO_ABORTIFHUNG,
            5000,
            Some(&mut result),
        );
    }
    result == 1
}

/// Lets a child process we are about to start (the picker) take the
/// foreground, so its dialog does not open behind other windows.
pub fn allow_foreground() {
    // SAFETY: plain call; failure only means the dialog may open unfocused.
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
    }
}

/// GUI-subsystem builds have no console. When started from a terminal
/// without redirection, attach to the terminal so log lines show up there.
pub fn attach_parent_console() {
    // SAFETY: plain handle query and console attach.
    unsafe {
        let redirected =
            GetStdHandle(STD_ERROR_HANDLE).is_ok_and(|h| !h.is_invalid() && !h.0.is_null());
        if !redirected {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

/// Shows the Windows file-open dialog for one or more videos (empty when
/// cancelled). Runs in the short-lived `wallive --pick` child so the shell's
/// dialog DLLs never load into the resident process.
pub fn pick_videos() -> windows::core::Result<Vec<PathBuf>> {
    // SAFETY: first COM call on this thread of the picker process; the
    // dialog needs an STA.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.ok()?;
    let filters = [
        COMDLG_FILTERSPEC {
            pszName: w!("Videos"),
            pszSpec: w!("*.mp4;*.m4v;*.mov;*.mkv;*.webm;*.wmv;*.avi;*.mpg;*.mpeg;*.ts;*.m2ts"),
        },
        COMDLG_FILTERSPEC {
            pszName: w!("All files"),
            pszSpec: w!("*.*"),
        },
    ];
    // SAFETY: standard IFileOpenDialog use; each returned path string is
    // copied and then freed with CoTaskMemFree as documented.
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetFileTypes(&filters)?;
        dialog.SetTitle(&HSTRING::from(
            "Choose videos for the wallpaper (several take turns)",
        ))?;
        dialog.SetOptions(
            dialog.GetOptions()? | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM | FOS_ALLOWMULTISELECT,
        )?;
        if dialog.Show(None).is_err() {
            return Ok(Vec::new()); // cancelled
        }
        let results = dialog.GetResults()?;
        let mut paths = Vec::new();
        for i in 0..results.GetCount()? {
            let name = results.GetItemAt(i)?.GetDisplayName(SIGDN_FILESYSPATH)?;
            if let Ok(path) = name.to_string() {
                paths.push(PathBuf::from(path));
            }
            CoTaskMemFree(Some(name.0 as *const _));
        }
        Ok(paths)
    }
}

/// Job object that kills its child processes (import / picker) when this
/// process exits, so an import never outlives the app.
pub struct ChildJob(HANDLE);

// SAFETY: a job handle may be used from any thread.
unsafe impl Send for ChildJob {}
// SAFETY: AssignProcessToJobObject is thread-safe.
unsafe impl Sync for ChildJob {}

impl ChildJob {
    pub fn new() -> windows::core::Result<Self> {
        // SAFETY: anonymous job; the limit structure matches the class.
        unsafe {
            let job = CreateJobObjectW(None, PCWSTR::null())?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )?;
            Ok(Self(job))
        }
    }

    pub fn assign(&self, child: &std::process::Child) {
        // SAFETY: the child's process handle is valid while `child` lives.
        unsafe {
            let _ = AssignProcessToJobObject(self.0, HANDLE(child.as_raw_handle()));
        }
    }
}

impl Drop for ChildJob {
    fn drop(&mut self) {
        // SAFETY: closed once; closing kills remaining children by design.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
