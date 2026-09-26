# Known Issues

Track unresolved bugs, limitations, and risky areas here.

## Open

- Classic WorkerW layout (Windows 10 / Windows 11 before 24H2) is implemented and unit-tested but has never run on a real classic desktop. Needs a VM or second machine.
- Multi-monitor, real resolution changes and monitor add/remove are untested (owner machine has one monitor; display change tested only with a synthetic `WM_DISPLAYCHANGE`).
- Explorer recreating its wallpaper layer (wallpaper/slideshow change) is covered only by unit tests of `z_fix`.
- Working set ~93 MB (81 MB private) while playing 1080p vs the 30 MB target; 52 MB (29 MB private) after 10 s of pause. Decoder surfaces and swap-chain buffers are charged to the process on this iGPU.
- Resuming after a pause longer than 10 s restarts at the previous key frame (up to 1 s back for new imports, up to ~4 s for imports made before 2026-09-26).
- Windows 10 has no compositor clock; pacing falls back to `Present(n)`, which DWM may throttle (see `logs/errors.md`). Untested.
- Exclusive-fullscreen games, Battery / Energy Saver toggles, battery power and session lock pause are implemented but not verified on hardware.
- New tray icons land in the Windows 11 overflow (^) area; users have to drag the icon to the taskbar to keep it visible.
- Several videos (ADR-012): a switch waits for the current video's loop end, so a video longer than the interval plays to its end first. "Choose videos..." replaces the whole list (no add / remove of single videos). Imports are made for the screen size at the first attach or the last choice; after a monitor change, restarting re-imports every video in the list for the new size.
- A classic desktop that never answers `0x052C` shows no wallpaper (by design - we do not draw over the icons). No user-visible message yet.

## Resolved

- 2026-09-26: switching to a shorter video failed (0xC00D36E5); fragmented MP4 imported no frames; playback read the SSD every loop (`logs/errors.md`).
- 2026-09-26: tray app now uses `windows_subsystem = "windows"` and `%LOCALAPPDATA%\Wallive\wallive.log`.
- 2026-09-26: second launch rotated the running instance's log (`logs/errors.md`).
- 2026-09-26: Explorer restart not handled until an unrelated message arrived (`logs/errors.md`).
