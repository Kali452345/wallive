# Current Handoff

## Current Branch

`spike/desktop-attach` (branched from `main` at `e176c41`). Checkpoints: `5a4de53` attach spike, `5a89838` Media Engine spike, `2472626` playback backend 2, then the pause-policy commit ("Pause policy: occlusion, fullscreen, power, session"). Not merged yet.

## Last Verified Build

2026-09-26: `cargo build --release`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test` (50 passed). rustc 1.98.1 MSVC, `windows` 0.62.2. `cargo` is not on PATH in agent shells: prepend `C:\Users\KaliOxygen\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin`.

## Current Phase

Features on top of working playback. Done: attach, playback, pause policy. Next: tray + config + import integration + Start with Windows + single instance.

## Working Features

- `wallive --play <video>`: video behind the desktop icons on every monitor (Windows 11 raised desktop verified), 1080p30 at ~0.8% of the CPU.
- Re-attach after Explorer restart and display changes; playback resumes at the saved position.
- Pause when every monitor is covered, for fullscreen apps, display off, Battery / Energy Saver, session lock / disconnect / remote; optional pause on battery (no UI for it yet). Paused costs ~0 CPU.
- `--import`, `--make-test-clip`, `--bench-decode` CLI modes. `tools/bench.ps1` (benchmark mode) and `tools/pause-check.ps1` (real-desktop pause test).

## In Progress

- Nothing half-done in code.

## Broken or Risky

- Working set ~115-120 MB vs the 30 MB target (decoder surfaces, video processor and swap-chain buffers are counted in the process on this iGPU).
- Classic WorkerW layout and Windows 10 never run on real hardware; Windows 10 pacing falls back to `Present(n)`, which may be throttled to ~4 fps like it is on Windows 11 (`logs/errors.md`).
- Multi-monitor untested (one monitor on the owner machine).
- Unverified pause triggers: exclusive-fullscreen game, saver toggles, battery, session lock.
- Mouse movement costs ~30 us per event through the LOCATIONCHANGE hook (ADR-005 notes).

## Last Change

Pause policy (ADR-005): `src/occlusion/`, `src/power/`, `src/runtime/pause.rs`, hooks / timer / power and session messages in `src/runtime/ffi.rs`.

## Last Test

Real desktop, `tools/pause-check.ps1` plus Win+D, virtual-desktop and fullscreen flows; `tools/bench.ps1` 60 s playing run (`logs/experiments.md`).

## Known Blockers

- None. Classic-layout testing needs a Windows 10 or pre-24H2 machine.

## Recommended Next Task

1. Config: `%APPDATA%\Wallive\config.txt` key=value (video, pause_on_battery, paused), pure parser + tests.
2. Tray (`Shell_NotifyIconW`): runtime-drawn icon, menu Choose video... / Pause / Start with Windows / Quit; re-add on `TaskbarCreated`.
3. Import integration: run `wallive --import` as a child process (below-normal priority, no window) into `%LOCALAPPDATA%\Wallive\cache`, switch playback when done.
4. Start with Windows (HKCU Run), single instance (named mutex), `windows_subsystem = "windows"` + file log.

## Files Most Relevant to Next Task

- `src/runtime/mod.rs`, `src/runtime/ffi.rs`
- `src/main.rs`
- `src/transcode/mod.rs`
- `docs/decisions.md`
