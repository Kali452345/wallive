# Experiments

Record benchmarks, prototypes, failed approaches, and comparison results here.

## 2026-09-26 - Desktop-attach spike (raised desktop, Windows 11 26200)

### Question

Can Wallive put its own window behind the desktop icons on the owner's Windows 11 (24H2+ raised desktop), keep it there across an Explorer restart and a display change, and what does the attach layer cost at idle?

### Setup

- Environment: Windows 11 Pro 10.0.26200 (raised desktop layout), one 1920x1080 monitor, Intel i5-8350U (8 logical cores), Intel UHD Graphics 620, laptop.
- Toolchain: rustc 1.98.1 stable-x86_64-pc-windows-msvc, `windows` crate 0.62.2, release build (LTO, panic=abort).
- Inputs: `target/release/wallive.exe` at branch `spike/desktop-attach`. Content is a GDI solid teal fill (class background brush), no video yet.
- Tools: `tools/inspect-desktop.ps1` (window tree + screenshot), `Get-Process` CPU time / working set.

### Result

- Layout detected: `Raised` (Progman ex-style `0x00200080` includes `WS_EX_NOREDIRECTIONBITMAP`).
- Z-order after attach (Progman children, topmost first): `SHELLDLL_DefView` > `WalliveWallpaper` (layered holder, ex `0x080800A0`) > Explorer's `WorkerW`. The layered holder was accepted (manifest declares Windows 10/11).
- Screenshot with the desktop shown: teal fill behind all icons; icons and labels drawn on top.
- Explorer restart (`Stop-Process explorer -Force`, auto-restarted by Windows):
  - First run: **did not re-attach** - see `logs/errors.md` (sent messages not waking the loop). Fixed.
  - After fix: Explorer died at 10.35 s, `TaskbarCreated` arrived at 16.16 s, re-hooked the new Explorer pid and re-attached by 16.25 s (89 ms). Tree and screenshot correct again.
- Display change: `WM_DISPLAYCHANGE` delivered with `SendNotifyMessage` (same non-queued path Windows uses) -> re-attached in 18 ms. A real resolution change / monitor add-remove was **not** exercised.
- Idle cost (attached, no video):

| Measurement | Value |
|---|---|
| CPU, 60 s window, desktop covered by other apps | 0.0 ms total (0.000%) |
| CPU, 120 s window, normal use incl. display-change test | 15.6 ms total (~0.013% of one core; one scheduler tick) |
| Working set | 6.7 MB |
| Private bytes | 1.0 MB |
| Threads | 1 |
| Handles | 92 |
| GPU | not measured; static GDI fill, nothing presents after the first paint |

- Explorer WinEvent volume (hook scoped to Explorer pid, `OBJID_WINDOW` only): 546 events in 325 s (included Win+D and two Explorer kill/restart bursts); 195 events in 185 s (included one restart). Each event costs one snapshot of Progman's children; not visible in CPU numbers above.

### Conclusion

- The raised-desktop attach from ADR-006 works on the owner's machine: layered, opaque, click-through child of Progman between the icons and Explorer's `WorkerW`.
- The attach layer is effectively free at idle (0 CPU, <7 MB working set, 1 thread) - well inside the budget, leaving ~23 MB for playback.
- Still unverified: classic WorkerW layout (needs a Windows 10 or pre-24H2 machine/VM), multi-monitor, real display changes, Explorer recreating its layer on a wallpaper/slideshow change (covered by unit tests of `z_fix` only), and whether DirectComposition content inside the layered holder presents correctly (next spike).

## 2026-09-26 - Playback spike: Media Engine windowless swap chain (ADR-003)

### Setup

- Commit: playback checkpoint (see `logs/handoff.md`). Machine: i5-8350U (8 logical), Intel UHD 620, 1920x1080 @ 60 Hz, Windows 11 26200, on AC.
- Clips made by `wallive --make-test-clip` (H.264 High via the MF hardware encoder, synthetic gradient + moving square): `1080p30` (7.3 Mbit/s), `1080p15`, `720p30`.
- `tools/bench.ps1` (new): starts wallive, 5 s warm-up, 20 s window, CPU from process CPU time, GPU from `\GPU Engine(pid_*)\Utilization Percentage` per engine type, then closes via `WM_CLOSE`. `-Minimize` minimises all windows so the wallpaper is visible and composed.

### Result

- Works end to end: metadata at ~390 ms, playing at ~460 ms after launch, video visible **behind the desktop icons** (screenshot pair via `tools/desktop-motion.ps1`: 91.9% of sampled pixels changed in 700 ms, icons drawn on top).
- Clean exit on `WM_CLOSE` (engine shut down, windows destroyed).

| Clip, desktop visible | CPU (% of one core) | CPU (% of all 8) | Working set | GPU video decode | GPU video processing |
|---|---|---|---|---|---|
| 1080p30 | 24.6 | 3.08 | 155 MB | 9.8% | 11.0% |
| 1080p15 | 10.9 | 1.37 | 141 MB | 4.7% | 5.4% |
| 720p30 | 21.9 | 2.74 | 109 MB | 4.7% | 11.7% |
| 1080p30, desktop covered by windows | 8.9 | 1.11 | 180 MB | 6.6% | 7.5% |

- Variants of 1080p30 (visible): `EnableTimeUpdateTimer(false)` 24.3%; `MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT` BGRA 19.3%, NV12 19.0% - all within run-to-run noise (~±5 points).
- CPU is spread over ~8 MF worker threads at priority 18-19 (MMCSS); the UI thread uses 0 ms. CPU scales with frame rate (~0.8% of a core per fps, i.e. ~7-8 ms CPU per frame), barely with resolution. RAM scales with resolution (decoder + swap-chain surfaces).
- Decode floor (`wallive --bench-decode`, new): hardware Source Reader (`MF_SOURCE_READER_D3D_MANAGER`, native NV12 textures, no presentation), flat out: **1.70 ms CPU/frame** at 1080p (560 fps), 1.42 ms at 720p.

### Conclusion

- The Media Engine path misses every budget: ~3% CPU vs <1%, ~150 MB vs <30 MB, ~20% GPU (decode + video processing) vs 1-5%.
- ~75% of the per-frame CPU is Media Engine presentation overhead, not decoding. A Source Reader + own composition swap chain paced by vsync should cost ~2 ms/frame (~0.75% of the CPU at 1080p30). ADR-003's "revisit when" condition is met; next spike replaces the backend and measures it.
- The 30 MB RAM target looks unreachable while decoder surfaces on an integrated GPU are counted in the process; to be re-measured with the new backend.

## 2026-09-26 - Playback backend 2: Source Reader + video processor + composition swap chain (ADR-003 revised)

### Setup

- Same machine and clips as the Media Engine spike above. `tools/bench.ps1`, 5 s warm-up, 20-40 s windows.
- Pipeline: `wallive-video` thread; hardware Source Reader (`MF_SOURCE_READER_D3D_MANAGER`, NV12 textures) -> `ID3D11VideoContext::VideoProcessorBlt` (cover crop, scale, BT.709 studio -> RGB full, auto-processing off) -> 2-buffer flip-sequential composition swap chain (frame latency 1) -> DComp visual per wallpaper window.

### Pacing attempts

| Pacing | Result |
|---|---|
| `Present(2)` (sync interval) + frame-latency waitable | **~4 fps.** Waitable released every ~250 ms (sometimes 500 ms). DWM throttles vsync-synced presents for our visual - the full-screen icon layer above it makes the window look occluded. `Present(1)` identical. |
| `Present(0)` + `IDXGIOutput::WaitForVBlank` x N | **~500 fps** - `WaitForVBlank` returns immediately for this windowed swap chain. |
| `Present(0)` + `DCompositionWaitForCompositorClock` x N (Win11, resolved at run time), control event in the same wait | **30.0 fps**, rewinds exactly every 10.0 s, no hold > 50 ms or decode > 30 ms in 40 s traces. Chosen. |

### Result (1080p30 clip, compositor-clock pacing)

| Run | CPU (% of one core) | CPU (% of all 8) | Working set | GPU video decode | GPU video processing | Achieved fps |
|---|---|---|---|---|---|---|
| desktop visible, run 1 | 6.76 | 0.845 | 115 MB | 7.2% | 8.7% | 30.2 / 30.1 / 30.1 |
| desktop visible, run 2 | 6.35 | 0.794 | 115 MB | 6.4% | 7.7% | 30.2 / 28.4 / 28.9 |
| desktop visible, run 3 | 6.16 | 0.769 | 114 MB | 6.2% | 7.7% | 22.6 / 28.3 / 22.0 |
| desktop visible, 40 s | 5.30 | 0.663 | 118 MB | 7.2% | 8.8% | 30.2 / 30.1 / 30.0 |
| desktop covered, 30 s | 7.85 | 0.981 | 113 MB | 7.2% | 8.8% | 30.2 / 30.1 / 29.7 |

- Runs 2-3 dipped while the machine was busy with unrelated work (system CPU up to 90% and a disk at 96% in the screenshot overlay at the time); later traced runs showed no stalls in our loop.
- Explorer killed during playback: video thread stopped with the windows, re-attached 130 ms after `TaskbarCreated`, playback resumed at the saved position; screenshot pair shows motion (87.9% of sampled pixels changed) with icons on top.
- Clean exit on `WM_CLOSE`.

### Conclusion

- vs the Media Engine: CPU 24.6% -> ~6% of a core (3.1% -> ~0.8% of the CPU, **inside the <1% budget**), GPU decode + processing ~21% -> ~15%, working set 155 -> ~115 MB.
- RAM is still far over the 30 MB target; decoder surfaces, the video processor and two 1080p BGRA swap-chain buffers live in shared memory on this integrated GPU. Next levers: fewer decoder surfaces (`MF_SA_MINIMUM_OUTPUT_SAMPLE_COUNT`), NV12 swap chain (no BGRA buffers, no conversion), trimming when paused.
- Windows 10 has no compositor clock; there the code falls back to `Present(n)`. Whether classic-layout windows get the same ~4 fps throttling is **unverified** (needs a Windows 10 / pre-24H2 machine).

## 2026-09-26 - Pause policy (ADR-005): occlusion, fullscreen, power, session

### Setup

- Same machine (i5-8350U, UHD 620, 1920x1080 @ 60.05 Hz, Windows 11 26200, AC) and 1080p30 clip.
- `tools/pause-check.ps1` drives the real desktop from another process (WinForms windows, synthetic input, `SC_MONITORPOWER`) and reads wallive's log; 10 s measurement windows. Flow checks for Win+D and virtual desktops were run with a scratch script using the same approach.
- 6 global out-of-context WinEvent hooks, 200 ms one-shot debounce, then `EnumWindows` + DWM frame bounds / cloak state + `SHQueryUserNotificationState`. EcoQoS on.

### Result

| Phase | CPU (% of one core) | CPU (% of all 8) | Working set | Log |
|---|---|---|---|---|
| playing (bench.ps1, 60 s) | 6.43 | 0.804 | 120 MB | GPU decode 7.4%, processing 9.1%; 30.2 fps with EcoQoS |
| maximized window (paused) | 0.00 - 0.62 | 0.000 - 0.078 | 112-116 MB | `pause: desktop covered` ~200 ms after the window appeared |
| paused + mouse storm (~1000 synthetic moves/s) | 2.96 | 0.371 | 116 MB | ~1000 cursor LOCATIONCHANGE events/s delivered to us |
| window closed | playing again | | | `playback: resumed` |
| half-screen window | 3.9 - 7.5 (playing) | | | no pause (correct) |
| display off (`SC_MONITORPOWER`) | paused | | | `power: DisplayOn(false)` -> `pause: display off`; Windows flapped on/off several times, playback followed every change |

Flow checks (all correct):
- maximized -> paused; Win+D -> resumed; Win+D again -> paused; close -> resumed.
- borderless topmost fullscreen window -> `pause: desktop covered, fullscreen app`; close -> resumed.
- maximized -> paused; Ctrl+Win+D (new virtual desktop, window cloaked) -> resumed; Ctrl+Win+F4 (back) -> paused; close -> resumed.
- Startup: Windows delivered all four power settings (display, Battery Saver, Energy Saver, AC/DC) right after registration, so no initial query is needed. `GUID_ENERGY_SAVER_STATUS` registers fine on 26200.

Occlusion check cost: 9-51 checks per run, 1.5-4.5 ms average, 2.5-39 ms max (the max was during the display-off flapping). Runs only after 200 ms of quiet, on the UI thread.

### Conclusion

- Paused really is ~0: the video thread sleeps on a kernel event, the UI thread only wakes for events.
- The cost of watching is the out-of-context delivery of `EVENT_OBJECT_LOCATIONCHANGE`, which also fires for every cursor move: ~30 us per event, i.e. ~3% of one core (0.37% of the CPU) at a continuous 1000 Hz mouse, ~0.4% of one core at 125 Hz, 0 when the mouse is still. It is the only event that catches maximize / restore / snap / programmatic resizes, so it stays; recorded as ADR-005's "revisit when" data point.
- Not verified: `QUNS_RUNNING_D3D_FULL_SCREEN` with a real exclusive-fullscreen game; Battery Saver / Energy Saver toggles and battery power (machine on AC; the notifications arrive, the mapping is unit-tested); session lock (cannot unlock unattended).
