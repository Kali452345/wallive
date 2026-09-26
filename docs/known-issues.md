# Known Issues

Track unresolved bugs, limitations, and risky areas here.

## Open

- Classic WorkerW layout (Windows 10 / Windows 11 before 24H2) is implemented and unit-tested but has never run on a real classic desktop. Needs a VM or second machine.
- Multi-monitor, real resolution changes and monitor add/remove are untested (owner machine has one monitor; display change tested only with a synthetic `WM_DISPLAYCHANGE`).
- Explorer recreating its wallpaper layer (wallpaper/slideshow change) is covered only by unit tests of `z_fix`.
- Spike runs with a console window (log output); the real app must switch to `windows_subsystem = "windows"` with a file or ETW log once the tray exists.
- A classic desktop that never answers `0x052C` shows no wallpaper (by design - we do not draw over the icons). No user-visible message yet.

## Resolved

- 2026-09-26: Explorer restart not handled until an unrelated message arrived (`logs/errors.md`).
