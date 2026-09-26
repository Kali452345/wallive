# Changelog

## 1.0.0 - 2026-09-26

First release.

- Video wallpaper behind the desktop icons on Windows 11 24H2 and later (tested) and the classic WorkerW desktop of Windows 10 / older Windows 11 (implemented, not yet tested on real hardware).
- Same video on every monitor from one hardware decoder (several monitors not yet tested).
- Videos are converted once, in the background, into a format the PC decodes in hardware (H.264 at the largest monitor's size, 30 fps, no audio). Fragmented YouTube / DASH MP4 files work.
- Several videos: switch every 1 / 5 / 15 / 30 / 60 minutes at the end of a loop, in order or shuffled; Next video in the tray menu.
- Pauses when every monitor is covered, for fullscreen apps and games, when the display is off, with Battery / Energy Saver, when the session is locked, and optionally on battery. After 10 s of pause the decoder's memory is freed.
- Keeps working after Explorer restarts and display changes.
- Tray menu, Start with Windows, `wallive --version`.
- Per-user installer (no admin rights) built with Inno Setup.

Measured on the development laptop (Intel iGPU, 1080p30): ~0.9% CPU on AC power, ~81 MB private memory while playing, 29 MB after 10 s paused, no disk reads after the first loop.
