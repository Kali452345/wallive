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
//! - One one-shot timer per playlist switch when several videos take turns
//!   (ADR-012).

mod ffi;
mod pause;
mod playlist;
mod tasks;

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::config::{self, Config};
use crate::desktop::{self, Status, Wallpaper};
use crate::playback::{self, Player};
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
    /// `wallive <video>...`: choose these videos at start, as if picked.
    pub open: Vec<PathBuf>,
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
    open_at_start: Vec<PathBuf>,
    tasks: Tasks,
    exe: PathBuf,
    /// Screen size imports are made for. Set once the desktop is attached
    /// and changed only when videos are chosen, so a game switching the
    /// display mode does not start re-imports.
    size: Option<(u32, u32)>,
    rng: playlist::Rng,
    /// Sources waiting for an import, in order; one import runs at a time.
    import_queue: VecDeque<PathBuf>,
    importing: Option<PathBuf>,
    /// Sources whose import failed; not retried until chosen again.
    failed: HashSet<PathBuf>,
    /// The playable file on screen.
    playing: Option<PathBuf>,
    /// The switch timer fired: switch when the video next reaches its end.
    switch_due: bool,
    timer_armed: bool,
    /// The video thread stopped on an error; switch without a loop end.
    media_failed: bool,
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
            size: None,
            rng: playlist::Rng::seeded(),
            import_queue: VecDeque::new(),
            importing: None,
            failed: HashSet::new(),
            playing: None,
            switch_due: false,
            timer_armed: false,
            media_failed: false,
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

        self.size = self.attached_size();
        if let Some(video) = self.fixed_video.clone() {
            self.open_video(&video);
        } else if !self.open_at_start.is_empty() {
            let videos = std::mem::take(&mut self.open_at_start);
            self.choose(videos);
        } else if !self.config.sources.is_empty() {
            self.resume();
        } else if self.first_run {
            log!("first run: asking for videos");
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
                match event {
                    playback::MEDIA_LOOPED if self.switch_due => self.next_video(),
                    playback::MEDIA_LOOPED => {
                        if let Some(p) = &self.player {
                            p.continue_after_loop();
                        }
                    }
                    playback::MEDIA_ERROR => self.media_failed = true,
                    _ => {}
                }
                None
            }
            Event::SwitchDue => {
                self.timer_armed = false;
                if self.media_failed {
                    self.next_video();
                } else if let Some(p) = self.player.as_mut() {
                    log!("playlist: switching at the end of this loop");
                    self.switch_due = true;
                    p.notify_at_loop_end();
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
                let videos = ffi::take_open_requests();
                log!("open requested: {} video(s)", videos.len());
                self.choose(videos);
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
            // Started before the desktop was ready (sign-in): imports can
            // now target the real screen size.
            if self.size.is_none() {
                self.size = self.attached_size();
                if self.size.is_some() {
                    self.ensure_imports();
                    self.ensure_timer();
                }
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
        self.media_failed = false;
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

    /// Start-up with saved videos: show the last one straight from the
    /// cache, then import whatever is missing.
    fn resume(&mut self) {
        if let Some(wallpaper) = self.config.wallpaper.clone().filter(|p| p.is_file()) {
            self.open_video(&wallpaper);
            self.playing = Some(wallpaper);
        } else {
            log!("cached wallpaper missing");
            let current = self.config.current;
            let other = playlist::next(current, &self.ready_list(), false, 0);
            if !self.play_index(current)
                && let Some(i) = other
            {
                self.play_index(i);
            }
        }
        self.ensure_imports();
        self.ensure_timer();
    }

    /// The user chose these videos (picker, `wallive <video>...`). The first
    /// plays as soon as it is imported; the others are imported after it.
    fn choose(&mut self, videos: Vec<PathBuf>) {
        if self.fixed_video.is_some() {
            return;
        }
        let mut sources: Vec<PathBuf> = Vec::new();
        for video in videos {
            if !video.is_file() {
                log!("not a file: {}", video.display());
            } else if !sources.contains(&video) {
                sources.push(video);
            }
        }
        if sources.is_empty() {
            return;
        }
        log!("chose {} video(s)", sources.len());
        for source in &sources {
            self.failed.remove(source);
        }
        self.config.sources = sources;
        self.config.current = 0;
        self.import_queue.clear();
        // A monitor bigger than at the last choice gets imports to match.
        self.size = Some(self.attached_size().unwrap_or((1920, 1080)));
        if !self.play_index(0) {
            // Nothing on screen yet: show a video that is already imported.
            // Otherwise the old video stays until the first new one is ready.
            let other = playlist::next(0, &self.ready_list(), false, 0);
            if self.playing.is_some() || !other.is_some_and(|i| self.play_index(i)) {
                self.save();
                self.clean_cache();
            }
        }
        self.ensure_imports();
        self.ensure_timer();
    }

    /// Largest monitor, which the import covers (ADR-004); `None` before
    /// the desktop is attached.
    fn attached_size(&self) -> Option<(u32, u32)> {
        self.wallpaper
            .surfaces()
            .iter()
            .map(|s| (s.width, s.height))
            .max_by_key(|&(w, h)| u64::from(w) * u64::from(h))
    }

    /// The imported copy of video `i`, if it exists.
    fn ready(&self, i: usize) -> Option<PathBuf> {
        let source = self.config.sources.get(i)?;
        config::cache_file(source, self.size?).filter(|p| p.is_file())
    }

    fn ready_list(&self) -> Vec<bool> {
        (0..self.config.sources.len())
            .map(|i| self.ready(i).is_some())
            .collect()
    }

    /// Shows video `i` if it is imported. Returns whether it did.
    fn play_index(&mut self, i: usize) -> bool {
        let Some(output) = self.ready(i) else {
            return false;
        };
        let n = self.config.sources.len();
        if n > 1 {
            log!(
                "playlist: video {}/{n}: {}",
                i + 1,
                self.config.sources[i].display()
            );
        }
        self.open_video(&output);
        self.playing = Some(output.clone());
        self.config.current = i;
        self.config.wallpaper = Some(output);
        self.save();
        self.clean_cache();
        self.rearm_timer();
        true
    }

    /// Switches to the next ready video, in order or shuffled.
    fn next_video(&mut self) {
        self.switch_due = false;
        let ready = self.ready_list();
        let random = self.rng.next();
        match playlist::next(self.config.current, &ready, self.config.shuffle, random) {
            Some(i) => {
                self.play_index(i);
            }
            None => {
                log!("playlist: no other video ready");
                if let Some(p) = &self.player {
                    p.continue_after_loop();
                }
                self.rearm_timer();
            }
        }
    }

    /// With several videos ready, times the next switch. The timer is
    /// one-shot and re-armed only after a switch, so a paused wallpaper
    /// wakes at most once per interval.
    fn ensure_timer(&mut self) {
        let ready = self.ready_list().into_iter().filter(|&r| r).count();
        if !self.persist || self.player.is_none() || ready < 2 {
            if self.timer_armed {
                self.host.kill_switch_timer();
                self.timer_armed = false;
            }
            self.switch_due = false;
            return;
        }
        if !self.timer_armed {
            let ms = self.config.switch_minutes.saturating_mul(60_000);
            self.host.set_switch_timer(ms);
            self.timer_armed = true;
        }
    }

    fn rearm_timer(&mut self) {
        self.timer_armed = false;
        self.switch_due = false;
        self.ensure_timer();
    }

    /// Queues imports for chosen videos that have none yet (the current one
    /// first, then in the chosen order) and starts one if none is running.
    fn ensure_imports(&mut self) {
        let Some(size) = self.size else { return };
        if self.fixed_video.is_some() {
            return;
        }
        let n = self.config.sources.len();
        for i in std::iter::once(self.config.current).chain(0..n) {
            let Some(source) = self.config.sources.get(i) else {
                continue;
            };
            let waiting =
                self.import_queue.contains(source) || self.importing.as_ref() == Some(source);
            if waiting || self.failed.contains(source) {
                continue;
            }
            if config::cache_file(source, size).is_some_and(|p| !p.is_file()) {
                self.import_queue.push_back(source.clone());
            }
        }
        self.import_next();
    }

    /// Imports run one at a time: each is CPU / GPU heavy, and the first
    /// chosen video should be ready first.
    fn import_next(&mut self) {
        let Some(size) = self.size else { return };
        while self.importing.is_none()
            && let Some(source) = self.import_queue.pop_front()
        {
            if !self.config.sources.contains(&source) {
                continue;
            }
            let Some(output) = config::cache_file(&source, size) else {
                log!("cannot read {}", source.display());
                continue;
            };
            if output.is_file() {
                continue;
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
            if self.tasks.importing() {
                self.importing = Some(source);
            } else {
                self.failed.insert(source);
            }
        }
    }

    /// Deletes imports of videos no longer chosen (just disk space). Keeps
    /// the file on screen and the temporary files of a running import.
    fn clean_cache(&self) {
        if !self.persist {
            return;
        }
        let keep: Vec<PathBuf> = match self.size {
            Some(size) => self
                .config
                .sources
                .iter()
                .filter_map(|s| config::cache_file(s, size))
                .collect(),
            None => Vec::new(),
        };
        let Ok(entries) = std::fs::read_dir(config::cache_dir()) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let temporary = name.contains(".part.") || name.contains(".src.");
            if !temporary
                && name.ends_with(".mp4")
                && !keep.contains(&path)
                && self.playing.as_ref() != Some(&path)
            {
                log!("cache: removing {name}");
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    fn on_task(&mut self, done: Done) {
        match done {
            Done::Picked(sources) if sources.is_empty() => {
                log!("picker closed without a choice");
            }
            Done::Picked(sources) => {
                log!("picked {} video(s)", sources.len());
                self.choose(sources);
            }
            Done::Imported {
                source,
                output,
                result,
            } => {
                self.importing = None;
                match self.config.sources.iter().position(|s| *s == source) {
                    None => {
                        log!("import of {} no longer needed", source.display());
                        let _ = std::fs::remove_file(&output);
                    }
                    Some(i) if result.is_ok() && output.is_file() => {
                        log!("import finished: {}", output.display());
                        // Show it at once if the current video is not on
                        // screen (first import after choosing, or the
                        // current one cannot be imported).
                        let current = self.ready(self.config.current);
                        if current.is_none() || current != self.playing {
                            self.play_index(i);
                        } else {
                            self.ensure_timer();
                        }
                    }
                    Some(_) => {
                        log!("import of {} failed: {result:?}", source.display());
                        self.failed.insert(source);
                    }
                }
                self.import_next();
            }
        }
    }

    fn menu(&mut self, at: (i32, i32)) {
        let autostart = shell::Autostart::is_enabled(&self.exe);
        let state = MenuState {
            paused: self.reasons.user,
            pause_on_battery: self.reasons.pause_on_battery,
            autostart,
            picking: self.tasks.picking(),
            videos: if self.persist {
                self.config.sources.len()
            } else {
                0
            },
            switch_minutes: self.config.switch_minutes,
            shuffle: self.config.shuffle,
        };
        let items = shell::menu(state, &playlist::SWITCH_CHOICES);
        let chosen = shell::show_menu(self.host.hwnd(), &items, Some(at));
        match chosen.and_then(Command::from_id) {
            Some(Command::Choose) => self.tasks.pick(),
            Some(Command::Next) => self.next_video(),
            Some(Command::SwitchEvery(minutes)) => {
                log!("playlist: switch every {minutes} min");
                self.config.switch_minutes = minutes;
                self.save();
                self.rearm_timer();
            }
            Some(Command::Shuffle) => {
                self.config.shuffle = !self.config.shuffle;
                self.save();
            }
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
        let imports = self.import_queue.len() + usize::from(self.importing.is_some());
        if self.player.is_none() {
            return if imports > 0 {
                "importing video\u{2026}".into()
            } else {
                "no video \u{2013} right-click to choose".into()
            };
        }
        let reasons = self.reasons.active();
        let text = if !reasons.is_empty() {
            format!("paused ({})", reasons.join(", "))
        } else if self.persist {
            let n = self.config.sources.len();
            let name = file_name(self.config.sources.get(self.config.current));
            if n > 1 {
                format!("playing {}/{n}: {name}", self.config.current + 1)
            } else {
                format!("playing {name}")
            }
        } else {
            format!("playing {}", file_name(self.fixed_video.as_ref()))
        };
        if imports > 0 {
            // First, so the tooltip's length limit cuts the file name instead.
            format!("importing {imports}\u{2026}\n{text}")
        } else {
            text
        }
    }
}

fn file_name(path: Option<&PathBuf>) -> String {
    path.and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
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
