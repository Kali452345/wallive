# Known Issues

Track unresolved bugs, limitations, and risky areas here.

## Open

- Classic WorkerW layout (Windows 10 / Windows 11 before 24H2) is implemented and unit-tested but has never run on a real classic desktop. Needs a VM or second machine.
- Multi-monitor, real resolution changes and monitor add/remove are untested (owner machine has one monitor; display change tested only with a synthetic `WM_DISPLAYCHANGE`).
- Explorer recreating its wallpaper layer (wallpaper/slideshow change) is covered only by unit tests of `z_fix`.
- Working set ~90-120 MB while playing vs the 30 MB target (decoder surfaces, video processor and swap-chain buffers are charged to the process on this iGPU).
- Windows 10 has no compositor clock; pacing falls back to `Present(n)`, which DWM may throttle (see `logs/errors.md`). Untested.
- Exclusive-fullscreen games, Battery / Energy Saver toggles, battery power and session lock pause are implemented but not verified on hardware.
- New tray icons land in the Windows 11 overflow (^) area; users have to drag the icon to the taskbar to keep it visible.
- A classic desktop that never answers `0x052C` shows no wallpaper (by design - we do not draw over the icons). No user-visible message yet.

## Resolved

- 2026-09-26: tray app now uses `windows_subsystem = "windows"` and `%LOCALAPPDATA%\Wallive\wallive.log`.
- 2026-09-26: second launch rotated the running instance's log (`logs/errors.md`).
- 2026-09-26: Explorer restart not handled until an unrelated message arrived (`logs/errors.md`).
