//! Power-setting and session notification registration, and EcoQoS. All
//! `unsafe` for `power/` lives here.
#![allow(unsafe_code)]

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM};
use windows::Win32::System::Power::{
    HPOWERNOTIFY, POWERBROADCAST_SETTING, RegisterPowerSettingNotification,
    UnregisterPowerSettingNotification,
};
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows::Win32::System::SystemServices::{
    GUID_ACDC_POWER_SOURCE, GUID_POWER_SAVING_STATUS, GUID_SESSION_DISPLAY_STATUS,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
    ProcessPowerThrottling, SetProcessInformation,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DEVICE_NOTIFY_WINDOW_HANDLE, GetSystemMetrics, SM_REMOTESESSION,
};

use super::{GUID_ENERGY_SAVER_STATUS, PowerChange};

/// Power-setting and session notifications for one window. Windows sends
/// each setting's current value right after registration, so the initial
/// state needs no separate query. Unregisters on drop.
pub struct Registration {
    hwnd: HWND,
    power: Vec<HPOWERNOTIFY>,
    session: bool,
}

impl Registration {
    pub fn new(hwnd: HWND) -> Self {
        let settings = [
            GUID_SESSION_DISPLAY_STATUS,
            GUID_POWER_SAVING_STATUS,
            GUID_ENERGY_SAVER_STATUS,
            GUID_ACDC_POWER_SOURCE,
        ];
        let power = settings
            .iter()
            // SAFETY: `hwnd` is our live window, passed as the recipient
            // handle as DEVICE_NOTIFY_WINDOW_HANDLE requires; the GUID is
            // only read during the call. An unknown GUID (Energy Saver
            // before 24H2) just fails.
            .filter_map(|g| unsafe {
                RegisterPowerSettingNotification(HANDLE(hwnd.0), g, DEVICE_NOTIFY_WINDOW_HANDLE)
                    .ok()
            })
            .collect();
        // SAFETY: registering our own live window.
        let session = unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) };
        Self {
            hwnd,
            power,
            session: session.is_ok(),
        }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        for h in self.power.drain(..) {
            // SAFETY: each handle came from RegisterPowerSettingNotification
            // and is unregistered once.
            unsafe {
                let _ = UnregisterPowerSettingNotification(h);
            }
        }
        if self.session {
            // SAFETY: balances the successful registration above; fails
            // cleanly if the window is already gone.
            unsafe {
                let _ = WTSUnRegisterSessionNotification(self.hwnd);
            }
        }
    }
}

/// Decodes the `lParam` of `WM_POWERBROADCAST` / `PBT_POWERSETTINGCHANGE`.
///
/// # Safety
/// `lp` must be the `lParam` of a `PBT_POWERSETTINGCHANGE` message that is
/// still being handled (it points at a `POWERBROADCAST_SETTING`).
pub unsafe fn decode(lp: LPARAM) -> Option<PowerChange> {
    let setting = lp.0 as *const POWERBROADCAST_SETTING;
    if setting.is_null() {
        return None;
    }
    // SAFETY: per the contract, `setting` points at a valid header followed
    // by `DataLength` bytes of data; we read at most 4 of them, unaligned.
    let (guid, value) = unsafe {
        let s = &*setting;
        if s.DataLength < 4 {
            return None;
        }
        let data = std::ptr::addr_of!(s.Data).cast::<u32>();
        (s.PowerSetting, data.read_unaligned())
    };
    super::power_change(&guid, value)
}

/// Whether this session is a remote-desktop session.
pub fn is_remote_session() -> bool {
    // SAFETY: plain metric query.
    unsafe { GetSystemMetrics(SM_REMOTESESSION) != 0 }
}

/// Opts the whole process into EcoQoS (ADR-005): the scheduler may run it
/// at the most power-efficient frequency / on efficiency cores. Harmless
/// where unsupported.
pub fn enable_eco_qos() -> bool {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    };
    // SAFETY: `state` is the structure this information class expects, and
    // the size matches.
    unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&state as *const PROCESS_POWER_THROTTLING_STATE).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
        .is_ok()
    }
}
