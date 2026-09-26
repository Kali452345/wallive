# Architecture

## Overview

Wallive is a single native Rust process with no visible main window: tray icon plus one wallpaper window per monitor. Video frames flow from the GPU's hardware decoder to DirectComposition without passing through the CPU or a 3D render pass. Everything else (occlusion, power, display changes) is driven by OS event notifications. There are no polling timers.

```text
                 import (one-time)
video file ──> transcode/ (probe HW decode -> MF transcode -> cached H.264 NV12 @ monitor res, 30 fps, no audio)
                                   │
                                   ▼
             playback/ (HW Source Reader -> D3D11 video processor -> composition swap chain, ADR-003)
                                   │  swap-chain handle
                   ┌───────────────┼───────────────┐
                   ▼               ▼               ▼
             DComp visual    DComp visual    DComp visual      (one per monitor, same surface)
                   │               │               │
             wallpaper hwnd  wallpaper hwnd  wallpaper hwnd    desktop/ attaches each behind the icons
                                   ▲
      occlusion/ + power/ ── pause / resume ──┘   (WinEvent hooks, power + session notifications)
```

## Current Shape

Implemented (2026-09-26):

- `Cargo.toml`, `build.rs` + `wallive.exe.manifest` (ADR-009)
- `src/main.rs`: modes (tray app, `wallive <video>`, `--quit`, `--play`, `--import`, `--pick`, `--make-test-clip`, `--bench-decode`), log macro and log file
- `src/runtime/`: `App` (owns everything below), hidden host window, message loop, WinEvent hooks (Explorer + window changes), debounce timer, power / session / tray / `WM_COPYDATA` messages; `pause.rs` pause reasons; `tasks.rs` child processes (picker, import) with waiter threads
- `src/desktop/`: Explorer layout detection (classic WorkerW vs 24H2+ raised desktop), attach, and re-attach on `TaskbarCreated` / `WM_DISPLAYCHANGE`
- `src/playback/`: D3D11 device, hardware Source Reader, video processor, composition swap chain, one DComp visual per monitor, loop, pause / resume, compositor-clock pacing
- `src/transcode/`: decoder capability probe and Media Foundation transcode to the cache (runs in a `--import` child)
- `src/occlusion/`: per-monitor coverage math and window enumeration (ADR-005)
- `src/power/`: display state, Battery / Energy Saver, AC/DC, session lock / remote, EcoQoS
- `src/shell/`: tray icon and menu, Start with Windows, single instance, file picker, job object (ADR-011)
- `src/config/`: `config.txt` in `%APPDATA%\Wallive`, cache paths (ADR-011)
- `tools/bench.ps1` (benchmark mode, ADR-010), `tools/pause-check.ps1`, `tools/inspect-desktop.ps1`
- `docs/`, `logs/`

## Boundaries

- `unsafe` / FFI lives in thin wrappers inside each module (`ffi.rs`). The crate denies `unsafe_code` and clippy denies `undocumented_unsafe_blocks`; only `ffi.rs` files opt back in. Logic such as coverage math, codec selection, and config stays in safe Rust and is unit-testable without Windows APIs.
- `occlusion/` and `power/` only emit pause/resume *reasons*. `playback/` owns the actual pause state (paused if any reason is active).
- `desktop/` knows nothing about video. It only returns an HWND per monitor.
- `transcode/` never runs in the playback path. It runs only on import, in a short-lived `wallive --import` child process (ADR-011).
- The tray settings UI must not load any UI framework into the resident process.

## Resource Budget

| State | CPU | GPU | RAM (working set) |
|---|---|---|---|
| Playing 1080p30 H.264 | < 1% | 1-5% (Video Decode engine; 3D near 0) | < 30 MB |
| Paused | ~0% | ~0% | < 30 MB (less if the decoder is released) |

Measure with PresentMon, GPU-Z / HWiNFO (clocks and package power), and Task Manager GPU engines. Task Manager percentages are clock-relative.

## Open Questions

- Does DWM use an overlay plane (MPO) for our DComp visual inside the desktop window tree? (ADR-003)
- Can one windowless swap-chain surface back visuals in multiple windows? (ADR-007)
- Which input containers does the MF source reader accept on a stock Windows install (MP4 / MOV / MKV / WebM)? This decides whether an optional FFmpeg import fallback is worth it.
- Seamless loop: the Source Reader seeks to 0 at end of stream; a visible hitch on long GOPs has not been measured.
