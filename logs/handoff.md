# Current Handoff

## Current Branch

`spike/desktop-attach` (branched from `main` at `e176c41`, the docs scaffold). Not merged yet.

## Last Verified Build

2026-09-26: `cargo build --release`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test` (17 passed). rustc 1.98.1 MSVC, `windows` 0.62.2.

## Current Phase

Spikes. Desktop-attach spike done on the raised desktop; playback spike next.

## Working Features

- `cargo run --release` puts a solid teal window behind the desktop icons on every monitor (Windows 11 raised desktop verified) and logs to the console.
- Re-attaches after Explorer restart (`TaskbarCreated`) and on `WM_DISPLAYCHANGE`; keeps z-order under the icons via an Explorer-scoped WinEvent hook. No timers.
- Clean exit on Ctrl+C or `WM_CLOSE` to the `WalliveHost` window.

## In Progress

- Nothing half-done in code. Next is the playback spike.

## Broken or Risky

- Classic WorkerW layout never run on a real classic desktop.
- Multi-monitor and real display changes untested (one monitor on owner machine).
- Explorer layer recreation (wallpaper/slideshow change) only unit-tested.
- Unknown: whether a DirectComposition target on the surface window (a child of a *layered* holder) presents correctly. If not, try the DComp target on the holder itself, or drop the holder and target a plain child of Progman (DComp/flip presents do not need Progman's redirection surface).

## Last Change

Desktop-attach spike plus fix for sent messages not waking the message loop (`logs/errors.md`).

## Last Test

Real desktop on Windows 11 26200: tree dump + screenshot, Explorer kill/restart, synthetic display change, 60 s and 120 s idle CPU measurement (`logs/experiments.md`).

## Known Blockers

- None for the next task. Classic-layout testing needs a Windows 10 or pre-24H2 VM.

## Recommended Next Task

Playback spike (ADR-003 / ADR-007), in the existing surface window from `desktop::Wallpaper`:

1. D3D11 device (video support flag) + `IMFDXGIDeviceManager`; `IMFMediaEngine` with `MF_MEDIA_ENGINE_DXGI_MANAGER`, looping, muted.
2. `IMFMediaEngineEx::EnableWindowlessSwapchainMode(TRUE)`, get the handle with `GetVideoSwapchainHandle`, and show it through `IDCompositionDevice::CreateSurfaceFromHandle` on a DComp visual targeting the surface window. Update the video rect with `UpdateVideoStream` on size change.
3. Hand-made 1080p30 H.264 test clip (no audio). Measure CPU / GPU engines / RAM, capture PresentMon to see the present mode (MPO or not), record in `logs/experiments.md`.
4. Then try the same swap-chain surface in a second visual to validate ADR-007 (can be a second window on one monitor).

Verify every API against current Microsoft docs first; update ADR-003 / ADR-007 "Unverified" sections with the results.

## Files Most Relevant to Next Task

- `src/desktop/mod.rs` (surface window per monitor)
- `src/runtime/mod.rs` (event wiring)
- `docs/decisions.md` (ADR-003, ADR-007)
- `docs/architecture.md`
- `logs/experiments.md`
