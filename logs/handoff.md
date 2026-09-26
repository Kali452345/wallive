# Current Handoff

## Current Branch

`spike/desktop-attach` (branched from `main` at `e176c41`). Checkpoints: `5a4de53` attach spike, `5a89838` Media Engine spike, `2472626` playback backend 2, `11ed5bf` pause policy, `3ef87a7` app shell, `892ab1c` owner-reported fixes, `2900f11` README / license / issue forms, `94a9b31` several videos (playlist). Not merged yet.

## Last Verified Build

2026-09-26: `cargo build --release`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test` (70 passed). rustc 1.98.1 MSVC, `windows` 0.62.2. `cargo` is not on PATH in agent shells: prepend `E:\DevTools\cargo\bin` (the user's `CARGO_HOME`), or `C:\Users\KaliOxygen\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin`.

## Current Phase

All planned features exist and run on the Windows 11 raised desktop. Remaining: RAM reduction and verification on setups not available here.

## Working Features

- `wallive` (tray app): plays the saved wallpaper, or asks for videos on first run. `wallive <video> [<video> ...]`: import (child processes, one at a time) + play, or hand the paths to the running instance. `wallive --quit`.
- Several videos take turns (ADR-012): switch every 1 / 5 / 15 / 30 / 60 min at the video's loop end, in order or shuffled; Next video switches at once.
- Tray menu: Choose videos..., Next video, Switch every >, Shuffle (the last three only with 2+ videos), Pause, Pause on battery, Start with Windows, Quit. Tooltip shows the state / pause reason / `i/n`.
- Video behind the desktop icons on every monitor, hardware decode, ~0.9% of the CPU for 1080p30 on AC (~1.2% on battery). Reads come from the file cache (no SSD reads after the first loop).
- Imports handle fragmented (DASH / YouTube) MP4 with edit lists.
- Re-attach after Explorer restart and display changes.
- Pause when every monitor is covered, for fullscreen apps, display off, Battery / Energy Saver, session lock / disconnect / remote, optional on battery. Paused costs ~0 CPU.
- Config `%APPDATA%\Wallive\config.txt`, cache `%LOCALAPPDATA%\Wallive\cache`, log `%LOCALAPPDATA%\Wallive\wallive.log`.
- `tools/bench.ps1` (benchmark mode, ADR-010), `tools/pause-check.ps1`.

## In Progress

- Nothing half-done in code.

## Broken or Risky

- Working set ~93 MB (81 MB private) playing vs the 30 MB target; 52 MB after 10 s paused (decoder released).
- Classic WorkerW layout and Windows 10 never run on real hardware; Windows 10 pacing falls back to `Present(n)` (may be throttled, `logs/errors.md`).
- Multi-monitor untested (one monitor here).
- Unverified pause triggers: exclusive-fullscreen game, saver toggles, battery, session lock.
- Windows 11 puts the tray icon in the overflow (^) area by default.
- Playlist: a video longer than the interval plays to its end before switching; after a monitor change, a restart re-imports every chosen video.

## Last Change

Several videos (ADR-012): multi-select picker and CLI, import queue, one-shot switch timer, switch at loop end, Next video / Switch every / Shuffle in the tray menu. Before that: README for testers, MIT license, issue forms (`2900f11`).

## Last Test

Real tray app: 3-video list via CLI, imports queued, timer + loop-end switches, Next / Shuffle / Switch every clicked in the menu, restart resume, 2-video list sent to the running copy, picker returning 2 files, memory over repeated switches (`logs/experiments.md`).

## Machine State Left Behind

- The tray app is running with the owner's config restored (one video, "Anime Red Eye").
- HKCU Run value `Wallive` is not set (Start with Windows off).

## Known Blockers

- None. Classic-layout testing needs a Windows 10 or pre-24H2 machine.

## Recommended Next Task

1. RAM while playing (81 MB private, 62.6 MB GPU): try an NV12 composition swap chain (~10 MB less at 1080p), and find what sets the decoder pool size (~15 surfaces for a 1-reference stream).
2. Verify on Windows 10 / a classic-layout machine and with two monitors.
3. Verify the remaining pause triggers (game, savers, battery, lock).
4. Merge `spike/desktop-attach` into `main` once the owner agrees.

## Files Most Relevant to Next Task

- `src/playback/mod.rs`, `src/playback/ffi.rs`
- `src/runtime/mod.rs`
- `tools/bench.ps1`
- `docs/decisions.md`
