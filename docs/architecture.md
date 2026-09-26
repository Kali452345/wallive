# Architecture

## Overview

Wallive is a single native Rust process with no visible main window: tray icon plus one wallpaper window per monitor. Video frames flow from the GPU's hardware decoder to DirectComposition without passing through the CPU or a 3D render pass. Everything else (occlusion, power, display changes) is driven by OS event notifications. There are no polling timers.

```text
                 import (one-time)
video file ──> transcode/ (probe HW decode -> MF transcode -> cached H.264 NV12 @ monitor res, 30 fps, no audio)
                                   │
                                   ▼
             playback/ (IMFMediaEngine on D3D11 device, loop, windowless swap chain)
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

Implemented so far (2026-09-26, desktop-attach spike): `Cargo.toml`, `build.rs` + `wallive.exe.manifest` (ADR-009), `src/main.rs`, `src/desktop/` (`tree.rs` pure layout logic + tests, `ffi.rs` Win32 wrappers, `mod.rs` attach/re-check), `src/runtime/` (host window, message loop, Explorer WinEvent hook - ADR-008), `tools/inspect-desktop.ps1`. Everything else below is planned.

- `Cargo.toml`
- `src/main.rs`: entry point and log macro (EcoQoS opt-in planned)
- `src/runtime/`: hidden top-level host window, message loop, event queue, Explorer-scoped WinEvent hook; turns OS notifications into events for the other modules
- `src/desktop/`: Explorer layout detection (classic WorkerW vs 24H2+ raised desktop), attach, and re-attach on `TaskbarCreated` / `WM_DISPLAYCHANGE`
- `src/playback/`: D3D11 device, Media Engine, windowless swap chain, DComp visuals, loop, and pause / resume
- `src/transcode/`: decoder capability probe and Media Foundation transcode to the cache
- `src/occlusion/`: `SetWinEventHook` subscriptions, debounce, and per-monitor coverage math
- `src/power/`: display state, Battery Saver, AC/DC, session lock / remote, and fullscreen-app state
- `src/tray/`: `Shell_NotifyIcon` menu (choose video, pause, start with Windows, quit)
- `src/config/`: small settings file (TOML or JSON) in `%APPDATA%\Wallive`
- `tools/bench/`: measurement harness (CPU, GPU engines, RAM, PresentMon capture)
- `docs/`
- `logs/`

## Boundaries

- `unsafe` / FFI lives in thin wrappers inside each module (`ffi.rs`). The crate denies `unsafe_code` and clippy denies `undocumented_unsafe_blocks`; only `ffi.rs` files opt back in. Logic such as coverage math, codec selection, and config stays in safe Rust and is unit-testable without Windows APIs.
- `occlusion/` and `power/` only emit pause/resume *reasons*. `playback/` owns the actual pause state (paused if any reason is active).
- `desktop/` knows nothing about video. It only returns an HWND per monitor.
- `transcode/` never runs in the playback path. It runs only on import (it may be a separate short-lived process to keep the resident RAM low).
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
- Seamless loop: does Media Engine `SetLoop(TRUE)` loop without a visible hitch, or is an encoder-side closed-GOP / timestamp fix needed?
