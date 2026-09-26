//! Why playback is paused (ADR-005). Every trigger sets or clears one flag;
//! playback runs only when none is set.

use crate::power::{PowerChange, SessionChange};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PauseReasons {
    /// Every monitor is (nearly) hidden by windows.
    pub covered: bool,
    /// Fullscreen game / app or presentation mode.
    pub fullscreen: bool,
    pub display_off: bool,
    pub battery_saver: bool,
    pub energy_saver: bool,
    pub on_battery: bool,
    pub locked: bool,
    pub disconnected: bool,
    pub remote: bool,
    /// Pausing on battery is a user choice; savers always pause.
    pub pause_on_battery: bool,
    /// Paused from the tray menu.
    pub user: bool,
}

impl PauseReasons {
    pub fn apply_power(&mut self, change: PowerChange) {
        match change {
            PowerChange::DisplayOn(on) => self.display_off = !on,
            PowerChange::BatterySaver(on) => self.battery_saver = on,
            PowerChange::EnergySaver(on) => self.energy_saver = on,
            PowerChange::OnBattery(on) => self.on_battery = on,
        }
    }

    pub fn apply_session(&mut self, change: SessionChange) {
        match change {
            SessionChange::Locked(locked) => self.locked = locked,
            SessionChange::Connected(connected) => self.disconnected = !connected,
        }
    }

    /// Names of the active reasons, for the log. Empty means play.
    pub fn active(&self) -> Vec<&'static str> {
        [
            (self.user, "paused by user"),
            (self.covered, "desktop covered"),
            (self.fullscreen, "fullscreen app"),
            (self.display_off, "display off"),
            (self.battery_saver, "battery saver"),
            (self.energy_saver, "energy saver"),
            (self.on_battery && self.pause_on_battery, "on battery"),
            (self.locked, "session locked"),
            (self.disconnected, "session disconnected"),
            (self.remote, "remote session"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect()
    }

    pub fn paused(&self) -> bool {
        !self.active().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plays_by_default() {
        assert!(!PauseReasons::default().paused());
    }

    #[test]
    fn battery_pauses_only_when_chosen() {
        let mut r = PauseReasons::default();
        r.apply_power(PowerChange::OnBattery(true));
        assert!(!r.paused());
        r.pause_on_battery = true;
        assert_eq!(r.active(), ["on battery"]);
        r.apply_power(PowerChange::OnBattery(false));
        assert!(!r.paused());
    }

    #[test]
    fn reasons_combine_and_clear_independently() {
        let mut r = PauseReasons::default();
        r.apply_power(PowerChange::DisplayOn(false));
        r.apply_session(SessionChange::Locked(true));
        r.covered = true;
        assert_eq!(
            r.active(),
            ["desktop covered", "display off", "session locked"]
        );
        r.apply_power(PowerChange::DisplayOn(true));
        r.apply_session(SessionChange::Locked(false));
        assert!(r.paused());
        r.covered = false;
        assert!(!r.paused());
    }

    #[test]
    fn savers_and_disconnect() {
        let mut r = PauseReasons::default();
        r.apply_power(PowerChange::EnergySaver(true));
        assert!(r.paused());
        r.apply_power(PowerChange::EnergySaver(false));
        r.apply_session(SessionChange::Connected(false));
        assert_eq!(r.active(), ["session disconnected"]);
        r.apply_session(SessionChange::Connected(true));
        assert!(!r.paused());
    }
}
