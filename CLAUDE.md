# CLAUDE.md - Wallive

You are working on Wallive.

## Start Here

Before changing code:

1. Read `AGENTS.md`.
2. Read `PROJECT_BRIEF.md`.
3. Read `logs/handoff.md`.
4. Read the latest entries in `logs/progress.md`.
5. Inspect the current implementation and Git status.

Do not rely on chat history for project state. The repository is the source of truth.

## Project Profile

- Type: Desktop application
- Stack: Rust (windows crate) + Win32 + Media Foundation IMFMediaEngine + Direct3D 11 + DirectComposition + Media Foundation Transcode API; tray-only UI

## Planned Features

- Video wallpaper behind desktop icons on Windows 10 and Windows 11 (classic WorkerW layout and 24H2+ raised-desktop layout)
- Same video on all monitors driven by a single shared decoder
- Hardware decode through Media Foundation IMFMediaEngine
- Presentation through the Media Engine windowless swap chain and DirectComposition with no extra render pass
- Import pipeline that probes hardware decode support and transcodes to the native codec at monitor resolution
- Automatic pause when the wallpaper is fully covered on every monitor
- Pause on fullscreen apps and games / display off / session lock / Battery Saver
- Re-attach after Explorer restart and display changes
- Tray icon with a minimal menu
- Start with Windows
- Benchmark mode that measures CPU / GPU / power


## Core Behavior

- Keep changes small and grounded in the existing code.
- Verify current documentation for version-sensitive APIs, packages, platforms, or providers.
- Do not default to the most common solution. Check the project constraints in `AGENTS.md` and pick the best fit; when real alternatives exist, state briefly when each applies and recommend one.
- Do not introduce mock data into real user-facing flows unless it is explicitly isolated as a demo or test fixture.
- Do not delete docs, logs, decisions, failed attempts, or handoff notes.
- Update project memory files after meaningful work.
- Never add a UI framework / web engine / WebView2 / .NET runtime / mpv / VLC or any heavy runtime to the always-running process
- Every new feature must state and measure its idle and playing CPU / GPU / RAM cost
- No polling loops or timers when an OS event notification exists
- Keep unsafe Rust confined to thin FFI wrapper modules with a SAFETY comment on every unsafe block
- Test desktop attach on both the classic WorkerW layout and the Windows 11 24H2+ raised-desktop layout
- Do not bundle FFmpeg by default - any FFmpeg fallback must be optional and license-reviewed

## Verification

- Run `cargo build --release`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` after any Rust change.
- After playback, desktop-attach, occlusion, or power changes, run the benchmark and record CPU / GPU / RAM numbers in `logs/experiments.md`.
- Run relevant tests after runtime, process, parser, queue, or filesystem changes.
- Verify the real desktop user flow, not only helper functions.
- Verify wallpaper attach on the real desktop: icons stay on top, Explorer restart re-attaches, and monitor add/remove/resolution change is handled.

## Known Commands

- `cargo build --release`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo run --release`

## Required Closeout

After meaningful work, update:

- `logs/progress.md`
- `logs/handoff.md`
- `logs/errors.md` if a significant problem was encountered
- `docs/decisions.md` if architecture, dependency, security, or data-flow decisions changed
