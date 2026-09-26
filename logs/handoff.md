# Current Handoff

## Current Branch

Unknown. Inspect Git status.

## Last Verified Build

Not verified yet.

## Current Phase

Project memory scaffold generated for Wallive.

## Working Features

- Starter AI documentation structure exists.

## In Progress

- Project-specific implementation details need to be filled in.
- Planned: Video wallpaper behind desktop icons on Windows 10 and Windows 11 (classic WorkerW layout and 24H2+ raised-desktop layout)
- Planned: Same video on all monitors driven by a single shared decoder
- Planned: Hardware decode through Media Foundation IMFMediaEngine
- Planned: Presentation through the Media Engine windowless swap chain and DirectComposition with no extra render pass
- Planned: Import pipeline that probes hardware decode support and transcodes to the native codec at monitor resolution
- Planned: Automatic pause when the wallpaper is fully covered on every monitor
- Planned: Pause on fullscreen apps and games / display off / session lock / Battery Saver
- Planned: Re-attach after Explorer restart and display changes
- Planned: Tray icon with a minimal menu
- Planned: Start with Windows
- Planned: Benchmark mode that measures CPU / GPU / power

## Broken

- Nothing recorded yet.

## Last Change

Generated project starter files.

## Last Test

Generator file creation only.

## Known Blockers

- Rust is not installed on the owner machine (checked 2026-09-26). Install rustup with the `stable-x86_64-pc-windows-msvc` toolchain and the Visual Studio C++ Build Tools.
- The folder is not a Git repository yet.

## Recommended Next Task

Build the riskiest pieces first, as throwaway prototypes, before the real app:

1. Desktop-attach spike (ADR-006): a window with a solid color behind the icons on Windows 11 24H2+ (the owner's machine), re-attached after Explorer restart.
2. Playback spike (ADR-003 / ADR-007): Media Engine in windowless swap-chain mode shown through DComp in that window, then the same surface in a second monitor's window. Measure CPU / GPU / RAM and check the present mode with PresentMon.

Record the results in `logs/experiments.md` and update ADR-003 / ADR-007 "Unverified" sections.

## Files Most Relevant to Next Task

- `AGENTS.md`
- `docs/decisions.md`
- `docs/architecture.md`
- `docs/testing.md`
