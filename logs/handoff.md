# Current Handoff

## Current Branch

`spike/desktop-attach` (branched from `main` at `e176c41`). Checkpoints: `5a4de53` attach spike, `5a89838` Media Engine spike, `2472626` playback backend 2, `11ed5bf` pause policy, `3ef87a7` app shell. Not merged yet.

## Last Verified Build

2026-09-26: `cargo build --release`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test` (59 passed). rustc 1.98.1 MSVC, `windows` 0.62.2. `cargo` is not on PATH in agent shells: prepend `C:\Users\KaliOxygen\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin`.

## Current Phase

All planned features exist and run on the Windows 11 raised desktop. Remaining: RAM reduction and verification on setups not available here.

## Working Features

- `wallive` (tray app): plays the saved wallpaper, or asks for a video on first run. `wallive <video>`: import (child process) + play, or hand the path to the running instance. `wallive --quit`.
- Tray menu: Choose video..., Pause, Pause on battery, Start with Windows, Quit. Tooltip shows the state / pause reason.
- Video behind the desktop icons on every monitor, hardware decode, ~0.9% of the CPU for 1080p30.
- Re-attach after Explorer restart and display changes.
- Pause when every monitor is covered, for fullscreen apps, display off, Battery / Energy Saver, session lock / disconnect / remote, optional on battery. Paused costs ~0 CPU.
- Config `%APPDATA%\Wallive\config.txt`, cache `%LOCALAPPDATA%\Wallive\cache`, log `%LOCALAPPDATA%\Wallive\wallive.log`.
- `tools/bench.ps1` (benchmark mode, ADR-010), `tools/pause-check.ps1`.

## In Progress

- Nothing half-done in code.

## Broken or Risky

- Working set ~90-120 MB vs the 30 MB target.
- Classic WorkerW layout and Windows 10 never run on real hardware; Windows 10 pacing falls back to `Present(n)` (may be throttled, `logs/errors.md`).
- Multi-monitor untested (one monitor here).
- Unverified pause triggers: exclusive-fullscreen game, saver toggles, battery, session lock.
- Windows 11 puts the tray icon in the overflow (^) area by default.

## Last Change

App shell (ADR-011): `src/shell/`, `src/config/`, `src/runtime/tasks.rs`, `App` in `src/runtime/mod.rs`, modes and log file in `src/main.rs`. ADR-010 documents the benchmark tool.

## Last Test

Real desktop: first-run import of a 4K60 clip, hand-off of a second video to the running instance, restart from config, second instance, tray menu Pause / Start with Windows, picker dialog; `bench.ps1 -Attach` on the tray app (`logs/experiments.md`).

## Machine State Left Behind

- The tray app may still be running with a scratch test clip as the wallpaper (`config.txt` points to it). Choose a real video from the tray menu, or run `wallive --quit`.
- HKCU Run value `Wallive` is not set (Start with Windows off).

## Known Blockers

- None. Classic-layout testing needs a Windows 10 or pre-24H2 machine.

## Recommended Next Task

1. RAM: measure what the working set is made of (VMMap / `QueryWorkingSetEx`); try fewer swap-chain buffers, a smaller reader sample pool, `SetProcessWorkingSetSize` trimming when paused, and releasing the decoder after a long pause.
2. Verify on Windows 10 / a classic-layout machine and with two monitors.
3. Verify the remaining pause triggers (game, savers, battery, lock).
4. Merge `spike/desktop-attach` into `main` once the owner agrees.

## Files Most Relevant to Next Task

- `src/playback/mod.rs`, `src/playback/ffi.rs`
- `src/runtime/mod.rs`
- `tools/bench.ps1`
- `docs/decisions.md`
