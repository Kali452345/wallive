# Testing

## Verification Commands

- `cargo build --release`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo run --release`

## Required Checks

- Run `cargo build --release`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` after any Rust change.
- After playback, desktop-attach, occlusion, or power changes, run the benchmark and record CPU / GPU / RAM numbers in `logs/experiments.md`.
- Run relevant tests after runtime, process, parser, queue, or filesystem changes.
- Verify the real desktop user flow, not only helper functions.
- Verify wallpaper attach on the real desktop: icons stay on top, Explorer restart re-attaches, and monitor add/remove/resolution change is handled.

## Desktop-attach checks

Tools: `tools/inspect-desktop.ps1 -Shot out.png` prints Progman's children in z-order and saves a screenshot. Stop the app cleanly by posting `WM_CLOSE` to the `WalliveHost` window (or Ctrl+C in its console) - this also prints the Explorer event count.

| Check | Raised (Win11 24H2+) | Classic (Win10 / Win11 <24H2) |
|---|---|---|
| Tree order icons > ours > Explorer layer | Pass 2026-09-26 | Not tested |
| Screenshot: fill behind icons, icons on top | Pass 2026-09-26 | Not tested |
| Explorer restart re-attaches | Pass 2026-09-26 (after fix) | Not tested |
| `WM_DISPLAYCHANGE` re-attaches | Pass (synthetic `SendNotifyMessage`) | Not tested |
| Real resolution change / monitor add-remove | Not tested | Not tested |
| Multi-monitor | Not tested (one monitor available) | Not tested |
| Wallpaper/slideshow change keeps us above Explorer layer | Unit-tested only (`z_fix`) | n/a |
| Icons hidden (View > Show desktop icons off) | Not tested | Not tested |

## Playback, pause and app checks

- Benchmark: `tools/bench.ps1` (`-Attach` for a running tray app). Results in `logs/experiments.md`.
- Pause: `tools/pause-check.ps1` (maximized window, mouse storm, half-screen window, display off; `-SkipDisplayOff` when someone is at the machine).
- App: `wallive <video>` (import + play), `wallive <video>` again while running (hand-off), `wallive` twice (second exits with code 3), `wallive --quit`. Check `%LOCALAPPDATA%\Wallive\wallive.log`.
- Tray menu: right-click the icon (Windows 11 may hide it under ^). Scripted: post `WM_APP+3` with `WM_CONTEXTMENU` in LOWORD of lParam to `WalliveHost`, from a DPI-aware process (`logs/errors.md`).

## Last Known Good

- Date: 2026-09-26
- Commit: see `logs/handoff.md`
- Commands run: `cargo build --release`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test` (59 passed)
- Manual checks: raised-desktop attach, Explorer restart, playback, pause policy, app shell flows (see `logs/experiments.md`)
