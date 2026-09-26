//! Power and session state that pauses playback (ADR-005): display off,
//! Battery Saver / Energy Saver, optionally running on battery, session lock
//! and disconnected or remote sessions. All of it arrives as window messages
//! on the host window; nothing polls.

mod ffi;

pub use ffi::{Registration, decode, enable_eco_qos, is_remote_session};

use windows::core::GUID;

/// One `PBT_POWERSETTINGCHANGE` notification, decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerChange {
    DisplayOn(bool),
    /// Battery Saver (Windows 10, Windows 11 before 24H2).
    BatterySaver(bool),
    /// Energy Saver (Windows 11 24H2+), any level.
    EnergySaver(bool),
    OnBattery(bool),
}

/// `GUID_ENERGY_SAVER_STATUS`; not in the `windows` crate metadata yet.
pub const GUID_ENERGY_SAVER_STATUS: GUID = GUID::from_u128(0x550e8400_e29b_41d4_a716_446655440000);

/// Maps a power setting and its DWORD value to a change we act on.
pub fn power_change(setting: &GUID, value: u32) -> Option<PowerChange> {
    use windows::Win32::System::SystemServices::{
        GUID_ACDC_POWER_SOURCE, GUID_POWER_SAVING_STATUS, GUID_SESSION_DISPLAY_STATUS,
    };
    match *setting {
        // 0 off, 1 on, 2 dimmed. A dimmed display still shows the wallpaper.
        s if s == GUID_SESSION_DISPLAY_STATUS => Some(PowerChange::DisplayOn(value != 0)),
        s if s == GUID_POWER_SAVING_STATUS => Some(PowerChange::BatterySaver(value != 0)),
        // 0 off, 1 standard, 2 high savings.
        s if s == GUID_ENERGY_SAVER_STATUS => Some(PowerChange::EnergySaver(value != 0)),
        // 0 AC, 1 DC (battery), 2 short-term source such as a UPS.
        s if s == GUID_ACDC_POWER_SOURCE => Some(PowerChange::OnBattery(value != 0)),
        _ => None,
    }
}

/// One `WM_WTSSESSION_CHANGE` notification that matters to playback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionChange {
    Locked(bool),
    /// The session lost / regained its display (fast user switching, RDP
    /// disconnect).
    Connected(bool),
}

pub fn session_change(code: u32) -> Option<SessionChange> {
    use windows::Win32::UI::WindowsAndMessaging::{
        WTS_CONSOLE_CONNECT, WTS_CONSOLE_DISCONNECT, WTS_REMOTE_CONNECT, WTS_REMOTE_DISCONNECT,
        WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
    };
    match code {
        WTS_SESSION_LOCK => Some(SessionChange::Locked(true)),
        WTS_SESSION_UNLOCK => Some(SessionChange::Locked(false)),
        WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT => Some(SessionChange::Connected(true)),
        WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT => Some(SessionChange::Connected(false)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::SystemServices::{
        GUID_ACDC_POWER_SOURCE, GUID_POWER_SAVING_STATUS, GUID_SESSION_DISPLAY_STATUS,
    };

    #[test]
    fn display_dimmed_still_counts_as_on() {
        let g = GUID_SESSION_DISPLAY_STATUS;
        assert_eq!(power_change(&g, 0), Some(PowerChange::DisplayOn(false)));
        assert_eq!(power_change(&g, 1), Some(PowerChange::DisplayOn(true)));
        assert_eq!(power_change(&g, 2), Some(PowerChange::DisplayOn(true)));
    }

    #[test]
    fn savers_and_power_source() {
        assert_eq!(
            power_change(&GUID_POWER_SAVING_STATUS, 1),
            Some(PowerChange::BatterySaver(true))
        );
        assert_eq!(
            power_change(&GUID_ENERGY_SAVER_STATUS, 2),
            Some(PowerChange::EnergySaver(true))
        );
        assert_eq!(
            power_change(&GUID_ENERGY_SAVER_STATUS, 0),
            Some(PowerChange::EnergySaver(false))
        );
        assert_eq!(
            power_change(&GUID_ACDC_POWER_SOURCE, 0),
            Some(PowerChange::OnBattery(false))
        );
        assert_eq!(
            power_change(&GUID_ACDC_POWER_SOURCE, 1),
            Some(PowerChange::OnBattery(true))
        );
        assert_eq!(power_change(&GUID::zeroed(), 1), None);
    }

    #[test]
    fn session_codes() {
        assert_eq!(session_change(7), Some(SessionChange::Locked(true)));
        assert_eq!(session_change(8), Some(SessionChange::Locked(false)));
        assert_eq!(session_change(2), Some(SessionChange::Connected(false)));
        assert_eq!(session_change(3), Some(SessionChange::Connected(true)));
        assert_eq!(session_change(5), None); // logon
    }
}
