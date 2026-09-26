# Decisions

Record important architecture, dependency, data-flow, platform, and security decisions here.

## ADR-001 - Project memory files

### Decision

Use `AGENTS.md`, `CLAUDE.md`, `PROJECT_BRIEF.md`, `docs/`, and `logs/` as persistent project memory.

### Context

AI assistants and chat sessions may change over time. The repository needs enough context for a new session to continue safely.

### Alternatives considered

- Keep context only in chat.
- Keep a single short README.

### Why this was selected

Repository-local memory is inspectable, versionable, and available to every assistant and developer.

### Revisit when

Revisit if the project adopts a different documented knowledge-management workflow.

## ADR-002 - Language: Rust with the `windows` crate

### Decision

Write Wallive in Rust using Microsoft's `windows` crate (windows-rs) for Win32, COM, Media Foundation, Direct3D 11, and DirectComposition.

### Context

The always-running process must use near-zero CPU and very little RAM. It needs direct access to native Windows media and composition APIs.

### Alternatives considered

- C++20 / MSVC: equal runtime cost and more Microsoft samples, but easier to introduce memory bugs.
- C# / .NET (WPF, WinUI): adds runtime and GC; tens of MB of extra RAM.
- Electron / Tauri / WebView2: a browser engine in the wallpaper process; far too heavy.

### Why this was selected

No runtime or GC, small binary, and memory safety outside of FFI. Owner chose Rust over C++ for safety. Microsoft's C++ samples map closely onto windows-rs, so they remain usable as reference.

### Revisit when

A required API is missing or unusable in windows-rs.

## ADR-003 - Playback: Source Reader + D3D11 video processor + composition swap chain (revised 2026-09-26)

### Decision

A dedicated video thread decodes with a hardware `IMFSourceReader` (`MF_SOURCE_READER_D3D_MANAGER`, native NV12 D3D11 textures), draws each frame into a flip-model composition swap chain with one `ID3D11VideoContext::VideoProcessorBlt` (cover crop + scale + YCbCr->RGB, driver auto-processing off), and presents with sync interval 0. Frames are held for N display refreshes by waiting on DWM's compositor clock (`DCompositionWaitForCompositorClock`, resolved at run time; Windows 10 falls back to `Present(N)`). The swap chain is the content of one DirectComposition visual per wallpaper window. Pause = the thread blocks on a kernel event.

Originally (first version of this ADR): `IMFMediaEngine` in windowless swap-chain mode. Implemented and measured in commit `5a89838`, then replaced.

### Context

The lowest-cost path is: GPU fixed-function decoder -> NV12 frame -> compositor, with no shader/3D render pass. Budget: <1% CPU, 1-5% GPU at 1080p30.

### Alternatives considered

- `IMFMediaEngine` windowless swap chain (first choice): works, but measured ~7-8 ms CPU per frame (24.6% of a core, 3.1% of the CPU at 1080p30) across ~8 MF worker threads; the decode itself costs ~1.7 ms/frame (`--bench-decode`). Output-format and time-update-timer tweaks made no difference.
- Sync-interval pacing (`Present(2)` + frame-latency waitable): DWM retires those presents only every ~250 ms for a visual under the full-screen icon layer -> ~4 fps.
- `IDXGIOutput::WaitForVBlank`: returns immediately for this windowed swap chain.
- A waitable timer at the frame rate: works everywhere, but a timer where an OS frame clock exists (project rule).
- mpv / libVLC / FFmpeg: heavy runtimes or licensing, and they render through the 3D engine.

### Why this was selected

Measured ~6% of a core (~0.8% of the CPU) at 1080p30, 30.0 fps, seamless loop - 4x less CPU than the Media Engine, with the thread asleep in the kernel between frames. Still no extra dependency.

### Revisit when

- Windows 10 / classic layout shows the same present throttling (then add a `D3DKMTWaitForVerticalBlankEvent` clock).
- RAM needs to drop further: NV12 swap chain or fewer decoder surfaces.
- HDR / 10-bit sources are supported (colour spaces are fixed to BT.709 SDR now).

### Unverified

Whether MPO / overlay planes are used for the swap chain under Progman / WorkerW (PresentMon shows the present mode).

## ADR-004 - Output codec: H.264 by default, chosen by hardware probe

### Decision

Probe the adapter with `ID3D11VideoDevice::GetVideoDecoderProfile` and `CheckVideoDecoderFormat`, and probe for installed MF decoders with `MFTEnumEx`.
- Default output: H.264 High, 8-bit 4:2:0 (NV12).
- Use HEVC or AV1 only when both hardware decode and an installed decoder are confirmed.

Transcode once on import with the Media Foundation Transcode API or Sink Writer, using hardware encoders when present.
- Resolution: the largest connected monitor.
- Frame rate: capped at 30 fps by default (24 and 60 are options).
- No audio track.
- Seamless loop.
- Cache the result.

### Context

Decode cost depends mostly on resolution x frame rate x bitrate, not codec. Every GPU hardware-decodes H.264 and Windows ships the decoder. HEVC needs the HEVC Video Extensions (paid unless OEM-provided). AV1 needs the AV1 Video Extension.

### Alternatives considered

- Always HEVC / AV1: smaller files, but not available on every machine.
- Bundled FFmpeg transcoder: accepts more input formats, but adds a large dependency and licensing burden. Kept as a possible optional fallback only.

### Why this was selected

It works everywhere with zero bundled codecs, and decode is always hardware-accelerated.

### Revisit when

Benchmarks show HEVC/AV1 decode is measurably cheaper at equal quality on common hardware.

## ADR-005 - Pause detection: event-driven coverage check

### Decision

Do not poll. Subscribe with out-of-context `SetWinEventHook` to foreground, minimize / restore, show / hide, location-change, and cloak / uncloak events. Debounce about 200 ms, then compute coverage per monitor:
1. Start from the monitor rect.
2. Walk visible top-level windows in z-order and subtract `DWMWA_EXTENDED_FRAME_BOUNDS`.
3. Skip minimized, cloaked (other virtual desktop), tool, and fully transparent click-through overlay windows.

Pause when every monitor is at least about 95% covered (threshold tunable).

Additional pause triggers:
- `SHQueryUserNotificationState`: fullscreen D3D apps and presentation mode.
- `RegisterPowerSettingNotification`: display off, Battery Saver, and optionally running on battery.
- `WTSRegisterSessionNotification`: session lock and remote sessions.

Paused means the Media Engine is paused. There are no presents and the last frame stays on screen. After a long pause, optionally release the decoder to free VRAM.

Opt the process into EcoQoS with `SetProcessInformation(ProcessPowerThrottling)`. EcoQoS effects apply on Windows 11; the call is harmless on Windows 10.

### Context

A covered wallpaper still costs decode and composition work. Lively uses a similar ~95% screen-coverage grid algorithm.

### Alternatives considered

- Timer polling of window layout: wastes CPU forever.
- Low-level hooks (`WH_CALLWNDPROC`, etc.): invasive and unnecessary.
- Foreground-window-only check: misses covering windows that are not in the foreground.

### Why this was selected

Zero cost while nothing changes. It uses observe-only APIs.

### Revisit when

Event storms (for example while dragging windows) show measurable CPU. In that case raise the debounce or drop location-change events while a mouse drag is in progress.

### Implementation notes (2026-09-26)

- "Paused" now means the video thread sleeps on its control event (backend 2, ADR-003 revised); the last frame stays on screen. A video thread started while paused (re-attach) still shows one frame first.
- Hooks: six narrow ranges (foreground, move/size end, minimize start..end, show..hide, location change, cloaked..uncloaked). The callback keeps only `OBJID_WINDOW` events for top-level windows, or for windows already destroyed (a hide may arrive after the window is gone), and re-arms a 200 ms one-shot `SetTimer` on the host window. That timer is the debounce, not polling: it exists only after an event and is killed when it fires.
- The coverage check does not need z-order: every visible top-level window is above the wallpaper. Union area per monitor by coordinate compression. Occluders are visible, not minimized, not cloaked, not click-through (`WS_EX_LAYERED | WS_EX_TRANSPARENT`), not Progman/WorkerW, and not tool windows except the taskbars. A maximized window plus the taskbar is 100%; a maximized window alone is 95.6% at 1080p.
- Fullscreen: `QUNS_BUSY`, `QUNS_RUNNING_D3D_FULL_SCREEN`, `QUNS_PRESENTATION_MODE`, re-read with each coverage check.
- Power: `GUID_SESSION_DISPLAY_STATUS` (the documented choice for user-mode apps, rather than `GUID_CONSOLE_DISPLAY_STATE`), `GUID_POWER_SAVING_STATUS`, `GUID_ENERGY_SAVER_STATUS` (Windows 11 24H2+; defined locally because the `windows` crate lacks it) and `GUID_ACDC_POWER_SOURCE`. Pausing on battery is an option, off by default; the savers always pause.
- Session: lock/unlock, console/remote connect/disconnect, plus `SM_REMOTESESSION`.
- All reasons feed one `PauseReasons` set in `runtime`; playback runs only when it is empty.
- Measured cost (logs/experiments.md): paused 0-0.6% of one core; out-of-context `LOCATIONCHANGE` delivery for cursor moves costs ~30 us per event (~3% of one core at a continuous 1000 Hz mouse, ~0.4% at 125 Hz, 0 when the mouse is still). Kept because it is the only event for maximize / restore / snap.

## ADR-006 - Desktop attach: support both Explorer layouts

### Decision

Send `0x052C` to Progman so Explorer creates the wallpaper layer, then detect the layout.
- Classic (Windows 10 and Windows 11 before 24H2): two top-level WorkerW windows. Parent the wallpaper window to the WorkerW behind the icons.
- Raised desktop (Windows 11 24H2+): Progman has `WS_EX_NOREDIRECTIONBITMAP` and `SHELLDLL_DefView` stays a child of Progman. Place a `WS_EX_LAYERED` (alpha 255) child of Progman directly below `SHELLDLL_DefView` and above Explorer's wallpaper WorkerW.

Re-attach on the `TaskbarCreated` message (Explorer restart) and on `WM_DISPLAYCHANGE`.

### Context

Windows 11 24H2 changed the desktop window tree. Apps that only knew the classic layout drew on top of the icons.

### Alternatives considered

- Classic layout only: broken on the owner's Windows 11.

### Why this was selected

It is required for correct behavior on both supported OS families.

### Revisit when

A future Windows update changes the tree again. Keep layout detection isolated in `src/desktop/`.

### References

- https://github.com/UnhingedSoftware/kirie/pull/11
- https://github.com/rexxpaper/rexpaper/pull/2
- https://github.com/rocksdanister/lively/discussions/2464

kirie is AGPL-3.0. It was read only for facts about Explorer's window tree (class names, the `0x052C` arguments `wParam=0xD, lParam=1`, the layered-holder technique). No kirie code was copied; Wallive's implementation is written independently. Keep it that way.

### Verified (2026-09-26)

Raised layout verified on the owner's Windows 11 26200: layered (alpha 255, click-through) holder child of Progman sits between `SHELLDLL_DefView` and Explorer's `WorkerW`; icons draw on top; Explorer restart re-attaches. See `logs/experiments.md`. Classic layout is implemented and unit-tested (`src/desktop/tree.rs`) but **not yet verified on a real classic desktop**.

Implementation notes:
- A layered child window needs the exe manifest to declare Windows 8+ (ADR-009); otherwise Windows silently drops `WS_EX_LAYERED`, so the style is read back after creation.
- A layered window is invisible until `SetLayeredWindowAttributes(..., 255, LWA_ALPHA)`.
- On a classic desktop that never splits, Wallive attaches nothing and waits, rather than drawing over the icons.

## ADR-007 - Multi-monitor: one decoder, same video everywhere

### Decision

All monitors show the same video. Use a single Media Engine instance and decoder. Each monitor gets its own wallpaper window and DirectComposition visual, and all visuals reference the same composition swap-chain surface, scaled per monitor by DComp. Transcode to the largest monitor's resolution.

### Context

The owner wants the same wallpaper on every monitor. Decoding once instead of N times divides decode cost by the number of monitors.

### Alternatives considered

- One Media Engine per monitor: simple, but N times the decode work.

### Why this was selected

It has the lowest cost for the required behavior.

### Revisit when

Monitors are driven by different GPUs, because a surface cannot be shared across adapters. That case needs a per-adapter fallback. Also revisit if per-monitor different videos becomes a feature.

### Unverified

Whether one windowless swap-chain handle can back visuals in several windows. This is an inference from the DComp surface-handle model and must be prototyped first.

## ADR-008 - Keeping the wallpaper attached: Explorer events, not polling

### Decision

Keep the wallpaper attached using OS notifications only:
- A hidden **top-level** host window (message-only windows do not receive broadcasts) handles `TaskbarCreated` (Explorer restart: re-hook the new Explorer and re-attach) and `WM_DISPLAYCHANGE` (re-attach for the new monitor layout).
- An out-of-context `SetWinEventHook` for `EVENT_OBJECT_CREATE..EVENT_OBJECT_REORDER`, **scoped to Explorer's process id**, with `WINEVENT_SKIPOWNPROCESS`, filtered to whole windows (`OBJID_WINDOW`, `CHILDID_SELF`). Each event triggers a cheap re-check: re-attach if our windows died or lost their parent, otherwise restore the z-order (icons > ours > Explorer's layer) moving only what is out of place.
- Events are coalesced in a queue drained by the message loop outside the window procedure. `push()` posts a wake message when the queue becomes non-empty, because sent messages and WinEvent callbacks are delivered inside `GetMessageW` (see `logs/errors.md`, 2026-09-26).
- `0x052C` is sent once per Progman instance. If Explorer creates its layer lazily, the resulting `WorkerW` creation event triggers the re-check; there is no sleep/retry loop.

### Context

Explorer recreates its wallpaper `WorkerW` on wallpaper or slideshow changes, and that can land above our windows on the raised desktop. Other projects (kirie, and Lively's equivalent logic) re-check on a timer. The project rules forbid polling when an OS notification exists.

### Alternatives considered

- Timer re-check every N ms: simple, but costs wakeups forever.
- Global (all-process) WinEvent hook: sees far more events than needed.
- Relying on `TaskbarCreated` alone: misses layer recreation and lazy `0x052C` answers.

### Why this was selected

Zero wakeups while Explorer's windows do not change; measured 0 ms CPU over 60 s idle and ~1 event/s during normal use (`logs/experiments.md`).

### Revisit when

Event volume from Explorer shows up in CPU measurements (then filter by class/parent in the callback or split the event range), or a Windows update moves the desktop windows out of Explorer's process.

## ADR-009 - Exe manifest embedded through the MSVC linker

### Decision

`wallive.exe.manifest` declares Windows 10/11 `supportedOS` and `PerMonitorV2` DPI awareness. `build.rs` embeds it with `/MANIFEST:EMBED /MANIFESTINPUT:<path>` linker arguments.

### Context

Layered child windows (ADR-006) require a Windows 8+ `supportedOS` entry. Monitor rectangles and window positions must be in physical pixels on mixed-DPI setups.

### Alternatives considered

- `embed-resource` / `winres` crates: extra build dependency, needs `rc.exe`.
- Calling `SetProcessDpiAwarenessContext` at runtime: works for DPI but not for `supportedOS`.

### Why this was selected

No extra dependency; the MSVC linker is already required.

### Revisit when

The project needs other resources (tray icon, version info). Then a `.rc` file via a resource crate may replace this.

## ADR-010 - Benchmark mode: an external script, not code in the app

### Decision

Benchmark mode is `tools/bench.ps1`. It starts wallive (or attaches to a running one) and samples it from outside: process CPU time, working set and private bytes, `\GPU Engine(pid_*)\Utilization Percentage` per engine type, and the battery discharge rate from `root\wmi BatteryStatus` when on battery. `tools/pause-check.ps1` does the same per phase while it drives the desktop.

### Context

The brief asks for a benchmark mode that measures CPU / GPU / power. Measuring from inside would put PDH / WMI code and its DLLs in the always-running process.

### Alternatives considered

- A `--bench` flag in wallive using PDH: measures itself, but loads pdh.dll and adds code to the resident binary.
- PresentMon / WPR traces: more detail (present mode, MPO), but an external download and manual analysis; still useful for one-off investigations.

### Why this was selected

It adds nothing to the resident process, and it measures the real binary with its real flags.

### Revisit when

Users need to run benchmarks without PowerShell, or per-frame present statistics are needed routinely.

## ADR-011 - App shell: tray-only, child processes for the picker and imports

### Decision

- `wallive` with no arguments is the tray app (`windows_subsystem = "windows"`). It uses one named mutex per session (`Local\Wallive.SingleInstance`). `wallive <video>` hands the path to the running instance through `WM_COPYDATA`, or starts with it; `wallive --quit` posts `WM_CLOSE`.
- The tray icon is `Shell_NotifyIconW` with `NOTIFYICON_VERSION_4`, re-added on `TaskbarCreated`. The icon image is drawn at run time (`CreateIconIndirect`), so no resource compiler is needed (ADR-009). The menu has Choose video..., Pause, Pause on battery, Start with Windows (HKCU Run key) and Quit.
- The file dialog runs in a `wallive --pick` child process that prints the chosen path. Imports run as a `wallive --import` child at below-normal priority with no window. Each child has one waiter thread (64 KB stack) blocked on it; the waiter posts the result to the host window. All children belong to a kill-on-close job object.
- Settings live in `%APPDATA%\Wallive\config.txt` as `key=value` lines. Imports are cached in `%LOCALAPPDATA%\Wallive\cache\<fnv64 of path, size, mtime, target size>.mp4`, and only the current import is kept. The log is `%LOCALAPPDATA%\Wallive\wallive.log`; it is opened only after the single-instance check, keeps the previous run as `wallive.old.log`, and stops growing at 1 MB.

### Context

The resident process must stay small (PROJECT_BRIEF). `IFileOpenDialog` loads a large part of the shell (and shell extensions) into the calling process, and those DLLs stay loaded. The encoder MFTs and 4K frame buffers of an import peaked at 425 MB in the child during testing.

### Alternatives considered

- Dialog and import in-process: simpler, but they permanently raise the resident working set, and a crashing codec or shell extension would take the wallpaper down.
- Registry for settings: no file to edit or back up by hand. A text file is transparent and roams with `%APPDATA%`.
- Task Scheduler for autostart: needed only for elevated start; the Run key is enough.

### Why this was selected

It keeps the always-running process to the wallpaper, the tray and the event wiring. Heavy, rare work pays its cost in a process that exits.

### Revisit when

Startup of the picker child is noticeably slow, or several imports need to be queued.
