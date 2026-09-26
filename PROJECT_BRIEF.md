# Project Brief - Wallive

## Mission

Extremely low-resource live video wallpaper for Windows 10 and Windows 11. It plays a looping video behind the desktop icons using the GPU's fixed-function hardware video decoder, converts imported videos to a codec, resolution, and frame rate this PC decodes natively, and pauses automatically when the wallpaper is fully covered. Target: near 0% CPU and 1-5% GPU while playing, near 0% when paused.

## Profile

- Type: Desktop application
- Stack: Rust (windows crate) + Win32 + Media Foundation IMFMediaEngine + Direct3D 11 + DirectComposition + Media Foundation Transcode API; tray-only UI
- Generated: 2026-09-26

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


## Expected Structure

- Cargo.toml
- src/main.rs
- src/desktop/ - WorkerW and Progman attach for both layouts
- src/playback/ - Media Engine and DirectComposition
- src/transcode/ - decode capability probe and Media Foundation transcode
- src/occlusion/ - WinEvent hooks and coverage math
- src/power/ - power and session notifications plus EcoQoS
- src/tray/
- src/config/
- tools/bench/
- docs/
- logs/


## Constraints and Context

- Target under 1% CPU and 1-5% GPU while playing 1080p30 and near 0% CPU and GPU when paused
- Working-set RAM target under 30 MB
- Supported OS: Windows 10 22H2 and Windows 11 including 24H2+ (owner runs Windows 11)
- Multiple monitors all show the same video
- Default output codec is H.264 8-bit 4:2:0 (NV12) because every GPU decodes it in hardware and Windows bundles it - use HEVC or AV1 only when the probe confirms both hardware decode and an installed decoder
- Transcode to the largest monitor resolution and a capped frame rate (default 30 fps) with no audio track and a seamless loop
- Prefer observe-only APIs such as out-of-context SetWinEventHook over hooks that intercept input
- Task Manager GPU % is clock-relative - measure with PresentMon / GPU-Z / HWiNFO package power
- Hardware overlay (MPO) for the wallpaper window is unverified and must be tested not assumed


## Success Criteria

- The project can be understood from repository files without chat history.
- Important implementation decisions are documented.
- Build, test, and verification commands are known.
- Fresh AI sessions can read the handoff and continue safely.

## Notes

Replace this file with the real project brief as planning becomes more specific.
