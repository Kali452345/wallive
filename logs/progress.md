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
