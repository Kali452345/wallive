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

## ADR-003 - Playback: Media Foundation Media Engine + DirectComposition

### Decision

Decode with `IMFMediaEngine` backed by a D3D11 device (hardware decode via DXVA). Use `IMFMediaEngineEx::EnableWindowlessSwapchainMode` and present the swap-chain handle through a DirectComposition visual in the wallpaper window.

### Context

The lowest-cost path is: GPU fixed-function decoder -> NV12 frame -> compositor, with no shader/3D render pass. Windowless swap-chain mode lets Media Foundation manage presentation and lets DWM use hardware overlay planes (MPO) when the hardware and window qualify.

### Alternatives considered

- mpv / libmpv (used by Lively): hardware decode, but its renderer draws each frame through the 3D engine. Lively has reports of high GPU use with mpv.
- VLC / libVLC: heavy dependency, same render-pass issue.
- `IMFSourceReader` + own D3D11 renderer: more control, but we would write the color conversion and frame pacing ourselves and lose MF's overlay handling.
- FFmpeg decode: large dependency, licensing burden, no benefit over MF for our formats.

### Why this was selected

It uses the built-in OS pipeline with no extra dependencies and the lowest possible GPU 3D engine usage.

### Revisit when

Benchmarks show Media Engine overhead is higher than an `IMFSourceReader` + DComp path, or windowless mode misbehaves inside the desktop window tree.

### Unverified

Whether MPO / overlay planes are used for a window parented under Progman / WorkerW. This is an inference and must be measured (PresentMon shows the present mode).

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
