//! Event wiring: the hidden host window, Explorer event subscription, and
//! the reactions that keep the wallpaper attached and playing.
//!
//! Wakes only for OS notifications:
//! - `TaskbarCreated` (Explorer restart): re-hook the new Explorer, re-attach,
//!   re-add the tray icon.
//! - `WM_DISPLAYCHANGE`: re-attach to the new monitor layout.
//! - Explorer window events: cheap re-check (z-order, lost windows).
//! - Global window events, debounced: occlusion / fullscreen re-check.
//! - Power-setting and session notifications: pause reasons.
//! - Tray clicks, finished child-process tasks, `wallive <video>` requests.
//! - Events posted from the video thread.

mod ffi;
mod pause;
mod tasks;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::config::{self, Config};
use crate::desktop::{self, Status, Wallpaper};
use crate::playback::Player;
use crate::shell::{self, Command, MenuState};
use crate::{log, occlusion, power};
use ffi::{Event, ExplorerHook, Host, WindowHooks};
use pause::PauseReasons;
use tasks::{Done, Tasks};

pub use ffi::COPYDATA_OPEN;

pub struct Options {
    /// `--play <video>`: play this file as-is and never write settings.
    /// `None`: the normal app, driven by `config.txt`.
    pub video: Option<PathBuf>,
    /// `wallive <video>`: choose this video at start, as if picked.
    pub open: Option<PathBuf>,
}

pub fn run(options: Options) -> windows::core::Result<()> {
    let host = Host::create()?;
    host.close_on_ctrl_c();
    let mut app = App::new(host, options);
    app.start();
    ffi::run_loop(|event| app.handle(event));
    app.shutdown();
    Ok(())
}

struct App {
    host: Host,
    wallpaper: Wallpaper,
    explorer: Option<ExplorerHook>,
    last: Status,
    player: Option<Player>,
    reasons: PauseReasons,
    checks: CheckStats,
    /// Pause-policy subscriptions; installed with the first player.
    watch: Option<(WindowHooks, power::Registration)>,
    icon: Option<shell::Icon>,
    tray: Option<shell::Tray>,
    tip: String,
    config: Config,
    /// False for `--play`: settings are neither read nor written.
    persist: bool,
    first_run: bool,
    fixed_video: Option<PathBuf>,
    open_at_start: Option<PathBuf>,
    tasks: Tasks,
    exe: PathBuf,
}

impl App {
    fn new(host: Host, options: Options) -> Self {
        let persist = options.video.is_none();
        let (config, existed) = if persist {
            Config::load(&config::config_file())
        } else {
            (Config::default(), true)
        };
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("wallive.exe"));
        let tasks = Tasks::new(exe.clone(), host.hwnd().0 as isize);
        let reasons = PauseReasons {
            pause_on_battery: config.pause_on_battery,
            user: config.paused,
            remote: power::is_remote_session(),
            ..Default::default()
        };
        Self {
            host,
            wallpaper: Wallpaper::default(),
            explorer: None,
            last: Status::Waiting("not attached yet"),
            player: None,
            reasons,
            checks: CheckStats::default(),
            watch: None,
            icon: None,
            tray: None,
            tip: String::new(),
            config,
            persist,
            first_run: !existed,
            fixed_video: options.video,
            open_at_start: options.open,
            tasks,
            exe,
        }
    }

    fn start(&mut self) {
        self.explorer = hook_explorer();
        self.last = self.wallpaper.attach();
        log!("attach: {:?}", self.last);
        self.icon = shell::Icon::new()
            .map_err(|e| log!("tray icon image failed: {e}"))
            .ok();
        self.add_tray();

        if let Some(video) = self.fixed_video.clone() {
            self.open_video(&video);
        } else if let Some(video) = self.open_at_start.take() {
            self.choose(video);
        } else if let Some(wallpaper) = self.config.wallpaper.clone().filter(|p| p.is_file()) {
            self.open_video(&wallpaper);
        } else if let Some(source) = self.config.source.clone().filter(|p| p.is_file()) {
            log!(
                "cached wallpaper missing; importing {} again",
                source.display()
            );
            self.choose(source);
        } else if self.first_run {
            log!("first run: asking for a video");
            self.tasks.pick();
        }
        self.refresh();
    }

    fn handle(&mut self, event: Event) {
        let status = match event {
            Event::TaskbarCreated => {
                log!("Explorer restarted");
                self.explorer = hook_explorer();
                if let Some(tray) = &self.tray {
                    tray.show();
                } else {
                    self.add_tray();
                }
                Some(self.wallpaper.attach())
            }
            Event::DisplayChanged => {
                log!("display configuration changed");
                Some(self.wallpaper.attach())
            }
            Event::DesktopChanged => self.wallpaper.on_desktop_changed(),
            Event::Media { event, param } => {
                if let Some(p) = self.player.as_mut() {
                    p.on_event(event, param);
                }
                None
            }
            Event::WindowsSettled => {
                self.check_windows();
                None
            }
            Event::Power(change) => {
                log!("power: {change:?}");
                self.reasons.apply_power(change);
                None
            }
            Event::Session(change) => {
                log!("session: {change:?}");
                self.reasons.apply_session(change);
                self.reasons.remote = power::is_remote_session();
                None
            }
            Event::TrayMenu { x, y } => {
                self.menu((x, y));
                None
            }
            Event::TaskDone => {
                for done in self.tasks.finished() {
                    self.on_task(done);
                }
                None
            }
            Event::OpenRequested => {
                for path in ffi::take_open_requests() {
                    log!("open requested: {}", path.display());
                    self.choose(path);
                }
                None
            }
        };
        if let Some(status) = status {
            if status != self.last || event != Event::DesktopChanged {
                log!("attach: {status:?}");
                self.last = status;
            }
            // New wallpaper windows need new composition targets.
            if let Some(p) = self.player.as_mut() {
                p.set_surfaces(self.wallpaper.surfaces());
            }
            // The monitor layout may have changed what is covered.
            if self.watch.is_some() {
                self.check_windows();
            }
        }
        self.refresh();
    }

    fn shutdown(&mut self) {
        log!("message loop ended; shutting down");
        self.player = None;
        self.watch = None;
        self.explorer = None;
        self.tray = None;
        self.wallpaper.detach();
        let (received, used) = ffi::window_event_counts();
        log!(
            "exit; Explorer window events: {}; global window events: {received} ({used} top-level); \
             occlusion checks: {} taking {:.2} ms total, {:.3} ms max",
            ffi::win_event_count(),
            self.checks.count,
            self.checks.total.as_secs_f64() * 1000.0,
            self.checks.max.as_secs_f64() * 1000.0,
        );
    }

    fn add_tray(&mut self) {
        let Some(icon) = &self.icon else { return };
        self.tip = shell::tooltip(&self.status_text());
        self.tray = shell::Tray::add(self.host.hwnd(), ffi::WM_TRAY, icon, &self.tip);
        if self.tray.is_none() {
            log!("tray icon not added (no taskbar yet?); retrying on TaskbarCreated");
        }
    }

    /// Starts or switches playback to an already playable file.
    fn open_video(&mut self, path: &Path) {
        if self.player.is_none() {
            match Player::new(self.host.hwnd()) {
                Ok(p) => self.player = Some(p),
                Err(e) => {
                    log!("playback unavailable: {e}");
                    return;
                }
            }
        }
        let Some(player) = self.player.as_mut() else {
            return;
        };
        log!("playback: opening {}", path.display());
        player.open(path);
        player.set_surfaces(self.wallpaper.surfaces());
        if self.watch.is_none() {
            // The pause policy only matters while there is something to pause.
            if power::enable_eco_qos() {
                log!("EcoQoS enabled");
            }
            let hooks = WindowHooks::install();
            log!("pause policy: {} window event hooks", hooks.count());
            // Registration makes Windows send the current power state at once.
            self.watch = Some((hooks, power::Registration::new(self.host.hwnd())));
        }
        self.check_windows();
    }

    /// The user picked `source`: play the cached import, or import it first.
    fn choose(&mut self, source: PathBuf) {
        if !source.is_file() {
            log!("not a file: {}", source.display());
            return;
        }
        let size = self.import_size();
        let Some(output) = config::cache_file(&source, size) else {
            log!("cannot read {}", source.display());
            return;
        };
        if output.is_file() {
            self.switch_to(source, output);
            return;
        }
        if let Err(e) = std::fs::create_dir_all(config::cache_dir()) {
            log!("cannot create the cache folder: {e}");
            return;
        }
        log!(
            "importing {} for {}x{} -> {}",
            source.display(),
            size.0,
            size.1,
            output.display()
        );
        self.tasks.import(&source, &output, size);
    }

    /// Largest monitor, which the import covers (ADR-004).
    fn import_size(&self) -> (u32, u32) {
        self.wallpaper
            .surfaces()
            .iter()
            .map(|s| (s.width, s.height))
            .max_by_key(|&(w, h)| u64::from(w) * u64::from(h))
            .unwrap_or((1920, 1080))
    }

    fn switch_to(&mut self, source: PathBuf, output: PathBuf) {
        self.open_video(&output);
        if !self.persist {
            return;
        }
        self.config.source = Some(source);
        self.config.wallpaper = Some(output.clone());
        self.save();
        // Keep only the current import; old ones are just disk space.
        if let Ok(entries) = std::fs::read_dir(config::cache_dir()) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path != output && path.extension().is_some_and(|e| e == "mp4") {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }

    fn on_task(&mut self, done: Done) {
        match done {
            Done::Picked(Some(source)) => {
                log!("picked {}", source.display());
                self.choose(source);
            }
            Done::Picked(None) => log!("picker closed without a choice"),
            Done::Imported {
                source,
                output,
                result: Ok(()),
            } if output.is_file() => {
                log!("import finished: {}", output.display());
                self.switch_to(source, output);
            }
            Done::Imported { source, result, .. } => {
                log!("import of {} failed: {result:?}", source.display());
            }
        }
    }

    fn menu(&mut self, at: (i32, i32)) {
        let autostart = shell::Autostart::is_enabled(&self.exe);
        let state = MenuState {
            paused: self.reasons.user,
            pause_on_battery: self.reasons.pause_on_battery,
            autostart,
            busy: self.tasks.busy(),
        };
        let chosen = shell::show_menu(self.host.hwnd(), &shell::menu(state), Some(at));
        match chosen.and_then(Command::from_id) {
            Some(Command::Choose) => self.tasks.pick(),
            Some(Command::Pause) => {
                self.reasons.user = !self.reasons.user;
                self.config.paused = self.reasons.user;
                self.save();
            }
            Some(Command::PauseOnBattery) => {
                self.reasons.pause_on_battery = !self.reasons.pause_on_battery;
                self.config.pause_on_battery = self.reasons.pause_on_battery;
                self.save();
            }
            Some(Command::Autostart) => match shell::Autostart::set(&self.exe, !autostart) {
                Ok(()) => log!("start with Windows: {}", !autostart),
                Err(e) => log!("start with Windows could not be changed: {e}"),
            },
            Some(Command::Quit) => self.host.close(),
            None => {}
        }
    }

    fn save(&self) {
        if self.persist
            && let Err(e) = self.config.save(&config::config_file())
        {
            log!("settings not saved: {e}");
        }
    }

    /// Re-reads what covers the wallpaper and whether a fullscreen app runs.
    fn check_windows(&mut self) {
        let started = Instant::now();
        self.reasons.covered =
            occlusion::all_covered(&self.wallpaper.monitors(), &occlusion::occluders());
        self.reasons.fullscreen = occlusion::fullscreen_app();
        let took = started.elapsed();
        self.checks.count += 1;
        self.checks.total += took;
        self.checks.max = self.checks.max.max(took);
    }

    /// Applies the pause reasons to the player and updates the tooltip.
    fn refresh(&mut self) {
        if let Some(player) = self.player.as_mut() {
            let paused = self.reasons.paused();
            if paused != player.is_paused() {
                if paused {
                    log!("pause: {}", self.reasons.active().join(", "));
                }
                player.set_paused(paused);
            }
        }
        let tip = shell::tooltip(&self.status_text());
        if tip != self.tip {
            if let Some(tray) = self.tray.as_mut() {
                tray.set_tip(&tip);
            }
            self.tip = tip;
        }
    }

    fn status_text(&self) -> String {
        if self.tasks.busy() {
            return "importing video\u{2026}".into();
        }
        if self.player.is_none() {
            return "no video \u{2013} right-click to choose one".into();
        }
        let reasons = self.reasons.active();
        if reasons.is_empty() {
            let name = self
                .config
                .source
                .as_deref()
                .or(self.fixed_video.as_deref())
                .and_then(Path::file_name)
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            format!("playing {name}")
        } else {
            format!("paused ({})", reasons.join(", "))
        }
    }
}

#[derive(Default)]
struct CheckStats {
    count: u32,
    total: Duration,
    max: Duration,
}

fn hook_explorer() -> Option<ExplorerHook> {
    let pid = desktop::explorer_pid();
    let hook = pid.and_then(ExplorerHook::install);
    match (pid, &hook) {
        (Some(pid), Some(_)) => log!("watching Explorer pid {pid}"),
        (Some(pid), None) => log!("could not hook Explorer pid {pid}"),
        (None, _) => log!("Explorer not found; waiting for TaskbarCreated"),
    }
    hook
}
