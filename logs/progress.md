# Progress Log

## 2026-09-26 - Project memory scaffold

### Worked on

Generated starter AI project memory files for Wallive.

### Changed

- Added `AGENTS.md`.
- Added `CLAUDE.md`.
- Added `PROJECT_BRIEF.md`.
- Added starter `docs/` files.
- Added starter `logs/` files.
- Captured planned feature list from the project profile.
- Captured expected project structure from the project profile.

### Verification

- Generator completed file creation.

### Remaining

- Replace placeholders with project-specific implementation details.
- Add real build, test, and run commands after the stack is initialized.

### Next AI

Read `AGENTS.md`, inspect the current codebase, and update project status before making feature changes.

## 2026-09-26 - Desktop-attach spike (raised desktop)

### Worked on

Git init, Rust toolchain setup, and the first spike from the handoff: a window behind the desktop icons on the owner's Windows 11 (raised desktop), kept attached across Explorer restart and display changes.

### Changed

- Git repo initialised on `main` (initial commit `e176c41` = docs scaffold); spike work on branch `spike/desktop-attach`. Repo-local git identity set (`KaliOxygen`).
- Rust stable MSVC toolchain set as default via rustup (rustup was installed, no default toolchain).
- Added crate: `Cargo.toml` (`windows` 0.62, lints: `unsafe_code = deny`, `undocumented_unsafe_blocks = deny`), `build.rs` + `wallive.exe.manifest` (ADR-009).
- `src/desktop/tree.rs`: pure layout detection (classic / raised / unsplit) and minimal z-order fix, 17 unit tests.
- `src/desktop/ffi.rs`: Win32 wrappers (Progman/WorkerW discovery, `0x052C`, layered holder, surface window, z-order moves).
- `src/desktop/mod.rs`: `Wallpaper` - attach one holder+surface per monitor, cheap re-check on Explorer events.
- `src/runtime/`: hidden top-level host window, message loop with event queue, Explorer-scoped WinEvent hook, Ctrl+C / `WM_CLOSE` clean exit.
- `tools/inspect-desktop.ps1`: window-tree dump + screenshot for verification.
- Docs: ADR-006 verified notes + AGPL note on kirie, ADR-008 (event-driven re-attach), ADR-009 (manifest); `docs/testing.md` desktop-attach matrix; `docs/known-issues.md`; `logs/experiments.md`; `logs/errors.md`.

### Why

The raised-desktop attach and Explorer-restart behaviour were the riskiest unknowns (handoff step 1).

### Verification

- `cargo build --release`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test` (17 passed).
- Real desktop: tree order icons > ours > Explorer layer; screenshot shows fill behind icons; Explorer kill/restart re-attached in 89 ms after `TaskbarCreated`; synthetic `WM_DISPLAYCHANGE` re-attached in 18 ms.
- Idle cost: 0 ms CPU / 60 s, 6.7 MB working set, 1 thread (`logs/experiments.md`).
- Found and fixed: sent messages (`TaskbarCreated`, `WM_DISPLAYCHANGE`, WinEvents) did not wake the loop (`logs/errors.md`).

### Remaining

- Classic layout on real hardware, multi-monitor, real display changes, wallpaper-change layer recreation, hidden icons.
- Playback spike.

### Next AI

Read `logs/handoff.md`. Start the playback spike in the attached surface window.

## 2026-09-26 - Playback (Media Engine spike, then backend 2)

### Changed

- `5a89838`: Media Engine windowless-swap-chain playback spike, measured (~25% of a core, ~155 MB) - over budget (`logs/experiments.md`).
- `2472626`: playback backend 2 (ADR-003 revised): hardware Source Reader -> D3D11 video processor (cover crop, BT.709) -> composition swap chain shown in one DComp visual per wallpaper window, paced by `DCompositionWaitForCompositorClock`. `src/playback/`, `tools/bench.ps1` (benchmark mode), `tools/desktop-motion.ps1`, `--bench-decode`, `--make-test-clip`.

### Verification

- 1080p30 at 30.0 fps behind the icons, ~6% of one core (~0.8% of the CPU), GPU decode ~7% + processing ~9%, ~115 MB working set. Explorer restart during playback re-attaches and resumes. `logs/experiments.md`, `logs/errors.md` (~4 fps throttling).

## 2026-09-26 - Pause policy (ADR-005)

### Changed

- `src/occlusion/`: pure coverage math (union area per monitor, occluder filter, 95% threshold, 8 tests) + FFI (`EnumWindows`, DWM frame bounds / cloak, `SHQueryUserNotificationState`).
- `src/power/`: power-setting decoding (display, Battery Saver, Energy Saver, AC/DC) and session changes (3 tests); registration FFI; EcoQoS.
- `src/runtime/ffi.rs`: six global out-of-context WinEvent hooks, 200 ms one-shot debounce timer, `WM_POWERBROADCAST` / `WM_WTSSESSION_CHANGE`.
- `src/runtime/pause.rs`: one `PauseReasons` set (4 tests); `runtime` pauses / resumes the player and logs why.
- `src/playback/mod.rs`: `set_paused` wired; a thread started while paused still shows one frame.
- `tools/pause-check.ps1`: real-desktop pause test with CPU per phase. `bench.ps1`: `-Exe` default fixed for PowerShell 5.1.

### Verification

- build / clippy / fmt / 50 tests pass.
- Real desktop: maximized window pauses (0-0.6% of one core while paused) and closing resumes; half-screen window keeps playing; Win+D, virtual-desktop switch and borderless fullscreen all correct; display off pauses and on resumes. Mouse storm cost measured. Playing cost unchanged (0.80% of the CPU). `logs/experiments.md`.

### Remaining

- Unverified: exclusive-fullscreen game, Battery / Energy Saver toggles, battery power, session lock.
- Tray, config, import integration, Start with Windows, single instance, windows subsystem + file log, RAM reduction.

## 2026-09-26 - App shell (ADR-010, ADR-011)

### Changed

- `src/config/`: `config.txt` key=value parser / writer (atomic save), config / cache paths, cache file name hash (5 tests).
- `src/shell/`: tray menu model, tooltip, runtime-drawn icon (4 tests); FFI for the tray icon, popup menu, HKCU Run autostart, single-instance mutex, `WM_COPYDATA` hand-off, file picker, job object, console attach.
- `src/runtime/tasks.rs`: picker and import as child processes of the same exe, waiter thread + posted message.
- `src/runtime/mod.rs`: `App` owns wallpaper, player, tray, config, tasks and pause state; start-up source order (`--play`, argument, saved import, re-import of the saved source, first-run picker).
- `src/main.rs`: `windows` subsystem, `wallive <video>`, `--quit`, `--pick`, log file with one rotation and a 1 MB cap.
- `docs/decisions.md`: ADR-010 (benchmark tool), ADR-011 (app shell).

### Verification

- build / clippy / fmt / 59 tests pass.
- Real desktop: 4K60 source imported to 1080p30 in a child (12.8 s, 425 MB peak in the child) and played; second video sent to the running instance, imported and switched, old cache deleted; restart from config shows the first frame in 261 ms; second instance exits 3; tray menu Pause and Start with Windows verified through `wallive.log` and the registry; picker dialog shown. Tray app playing: 0.886% of the CPU, 90.5 MB (`logs/experiments.md`).
- Found and fixed: second launch rotated the running instance's log (`logs/errors.md`).

### Remaining

- RAM (~90-120 MB vs the 30 MB target).
- Unverified: Windows 10 / classic layout, multi-monitor, exclusive-fullscreen game, saver toggles, battery, session lock.

## 2026-09-26 - Owner-reported fixes: video switch, DASH MP4, disk reads, RAM

### Changed

- `src/playback/mod.rs`: switching videos no longer inherits the old position; a failed resume seek starts over; after 10 s of pause the decoder is released (and the device flushed / trimmed), reopened on resume.
- `src/playback/ffi.rs`: buffered shell file stream instead of MF's file stream; `MF_LOW_LATENCY`; `trim()`; `Signal::wait_for`.
- `src/transcode/`: first frame read before the encoder exists; fragmented-MP4 edit-list workaround (`mp4.rs`, 4 tests); encoder asked for no B-frames and a 1 s key frame interval.
- `Cargo.toml`: `Win32_System_Ole` (VARIANT type only).

### Verification

- build / clippy / fmt / 63 tests pass.
- Real tray app: Red Eye switch plays; `videoplayback.mp4` imports (986 frames) and plays; SSD reads while playing ~0 (was ~1.1 MB/s); private 81 MB playing (was 108), 29 MB after 10 s paused, 81 MB resumed; CPU not worse in a same-conditions A/B (`logs/experiments.md`).

### Remaining

- RAM while playing is still 81 MB private (62.6 MB of it GPU surfaces). Next candidates: NV12 composition swap chain (~10 MB), decoder pool size.

## 2026-09-26 - README for testers; several videos (playlist)

### Changed

- `README.md`, `LICENSE` (MIT), `.github/` issue forms (bug, test report) and PR template: what is tested and what is not (multi-monitor, 4K, Windows 10, NVIDIA / AMD), how to report and contribute (commit `2900f11`). Playlist: commit `94a9b31`.
- Several videos take turns (ADR-012):
  - `src/config/mod.rs`: `source=` list, `current`, `switch_minutes`, `shuffle`; reads older one-video configs.
  - `src/runtime/playlist.rs`: next index in order or shuffled (xorshift).
  - `src/runtime/mod.rs`: import queue (one at a time, current first), cache cleanup for videos no longer chosen, one-shot switch timer, switch at loop end, Next video / Switch every / Shuffle handlers, tooltip "playing i/n".
  - `src/runtime/ffi.rs`: `SWITCH_TIMER`, `Event::SwitchDue`, multi-path `WM_COPYDATA`, dropping a stale queued `SwitchDue`.
  - `src/runtime/tasks.rs`: picker returns several paths; picking and import state tracked separately.
  - `src/playback/mod.rs`: `notify_at_loop_end` / `MEDIA_LOOPED` / `continue_after_loop`.
  - `src/shell/`: menu model with a submenu (Choose videos..., Next video, Switch every > 1 / 5 / 15 / 30 / 60 min, Shuffle), multi-select picker, several paths to a running copy.
  - `src/main.rs`: `wallive <video> [<video> ...]`; `--pick` prints one path per line.

### Verification

- build / clippy / fmt / 70 tests pass.
- Real tray app (`logs/experiments.md`):
  - `wallive clip Red-Eye Rimuru` showed the imported Red Eye at once, imported the other two one at a time, and switched at the loop end after the 1 min timer (first frame of the next video ~0.2 s after the old one's last frame, which stays on screen).
  - Next video, Shuffle and the Switch every submenu were clicked in the real menu. A restart resumed from the config.
  - A 2-video list sent to the running copy replaced the list and deleted the unused import. The picker returned two files.
- Found and fixed: a stale timer event after "Next video" (`logs/errors.md`).

### Remaining

- Multi-monitor / 4K / Windows 10 still untested (README asks testers).
- RAM while playing (81-85 MB private at 1080p).
