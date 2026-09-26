# Wallive

[![CI](https://github.com/Kali452345/wallive/actions/workflows/ci.yml/badge.svg)](https://github.com/Kali452345/wallive/actions/workflows/ci.yml) [![Latest release](https://img.shields.io/github/v/release/Kali452345/wallive)](https://github.com/Kali452345/wallive/releases/latest)

A live video wallpaper for Windows 10 and 11 that tries to cost almost nothing. The video plays behind your desktop icons using the graphics chip's hardware video decoder, and it pauses by itself when you can't see it.

- A small native program (Rust + Win32). No browser engine, no .NET, no bundled FFmpeg.
- Tray icon only: right-click it to choose videos, pause, or quit.
- Videos you choose are converted once, in the background, into a format your PC decodes in hardware. Playback then uses the converted copy.

> **Version 1.0.0 - early.** It works well on the one machine it was developed on. Many setups have never been tried, and help testing them is very welcome (see [What is tested](#what-is-tested) and [Reporting problems](#reporting-problems)).

## Features

- Video behind the desktop icons; icons, the taskbar and windows stay on top.
- Same video on every monitor from one shared decoder.
- **Several videos:** pick more than one (Ctrl+click in the file dialog) and Wallive switches between them every 1 / 5 / 15 / 30 / 60 minutes, in order or shuffled. The switch waits for the current video to reach its end, so no scene is cut; **Next video** in the tray menu switches at once.
- Pauses automatically when:
  - every monitor is covered by windows;
  - a fullscreen app, game or presentation runs;
  - the display turns off;
  - Battery Saver / Energy Saver is on;
  - the session is locked or disconnected;
  - optionally, the PC runs on battery.
- After 10 s of pause it frees the video decoder's memory; playback continues when you come back.
- Keeps working after Explorer restarts and display changes.
- Start with Windows (tray menu), no admin rights needed.

## Measured cost

On the development laptop (Intel integrated graphics, 1920x1080 at 60 Hz, Windows 11 26200), playing a 1080p 30 fps wallpaper:

| | CPU (whole machine) | Memory (private) | Disk |
|---|---|---|---|
| Playing, on AC power | ~0.9% | ~81 MB | ~0 after the first loop (file cache) |
| Playing, on battery | ~1.2% | ~81 MB | ~0 |
| Paused (covered, fullscreen app, ...) | ~0% | ~81 MB, then 29 MB after 10 s | 0 |

On integrated graphics, video memory is ordinary RAM and is counted in Wallive's memory. Most of the ~81 MB is the decoder's frame buffers. Full numbers and method: [`logs/experiments.md`](logs/experiments.md).

## What is tested

| Area | Status |
|---|---|
| Windows 11 24H2 and later (desktop layout with a "raised" icon layer) | **Tested** (Windows 11 26200) |
| Windows 10, and Windows 11 before 24H2 (classic "WorkerW" desktop layout) | **Not tested on real hardware.** Implemented and unit-tested only. |
| One monitor, 1920x1080, 125% scaling | **Tested** |
| **Several monitors** (same video on each; add/remove a monitor; different sizes) | **Not tested.** The developer has no second monitor. |
| **4K / 1440p / high-DPI screens** (imports then target the biggest monitor, up to 4K) | **Not tested.** The developer has no such screen. |
| Changing the resolution while running | Tested only with a simulated display-change message |
| Intel integrated graphics | **Tested** |
| NVIDIA / AMD graphics | **Not tested** |
| Pause when covered by windows, Win+D, virtual desktops, borderless fullscreen, display off | **Tested** |
| Pause for exclusive-fullscreen games, Battery / Energy Saver toggles, on-battery, lock screen | Implemented, **not yet verified** |
| Explorer restart | **Tested** |
| Importing MP4 / WebM / 4K 60 fps / vertical YouTube (DASH) videos | **Tested** |
| Several videos, switching in order / shuffled | **Tested** |

If you have one of the untested setups, trying Wallive and reporting what happens (good or bad) is the most useful contribution right now.

## Install and use

### Installer (recommended)

1. Download `Wallive-<version>-setup.exe` from [Releases](../../releases/latest). Windows 10 or 11, 64-bit.
2. Run it. It installs for your user only (`%LOCALAPPDATA%\Programs\Wallive`) and needs no admin rights. It can start Wallive when you sign in.
   - The installer is not code-signed yet, so Windows SmartScreen may say "Windows protected your PC". Click **More info** > **Run anyway**. You can compare the file with the `.sha256` checksum next to it on the release page (`Get-FileHash Wallive-<version>-setup.exe`).
3. On first start Wallive asks for videos. Afterwards, right-click the tray icon.
   - Windows 11 may hide new tray icons under the **^** arrow; drag the icon onto the taskbar to keep it visible.

Updating: run the newer installer; it closes the running copy and keeps your settings. Uninstalling (Settings > Apps > Installed apps > Wallive) also removes the settings, the converted videos and the log.

### Build from source

1. Install [Rust](https://rustup.rs/) 1.88 or later (MSVC toolchain) and the Visual Studio Build Tools with the "Desktop development with C++" workload.
2. `cargo build --release`
3. Run `target\release\wallive.exe`.

To build the installer: install [Inno Setup](https://jrsoftware.org/isinfo.php) 6.3 or later, then run `powershell -ExecutionPolicy Bypass -File tools\package.ps1`. The installer and its checksum are written to `dist\`.

Command line:

| Command | What it does |
|---|---|
| `wallive` | Start (or do nothing if already running) |
| `wallive <video> [<video> ...]` | Use these videos (sent to the running copy if there is one) |
| `wallive --quit` | Close the running copy |
| `wallive --version` | Print the version |
| `wallive --play <video>` | Play a file as-is, without saving settings (testing) |
| `wallive --bench-decode <video>` | Measure hardware decode speed |

Files:

- Settings: `%APPDATA%\Wallive\config.txt` (plain text, editable)
- Converted videos: `%LOCALAPPDATA%\Wallive\cache\`
- Log: `%LOCALAPPDATA%\Wallive\wallive.log` (previous run: `wallive.old.log`)

## Reporting problems

Please [open an issue](../../issues/new/choose) and include:

1. Windows version (`winver`), graphics chip (Task Manager > Performance > GPU).
2. Monitors: how many, resolution, refresh rate and scaling (Settings > Display) for each.
3. What you did, what you expected, what happened. A screenshot helps for display problems.
4. The log: `%LOCALAPPDATA%\Wallive\wallive.log` (quit Wallive first, then attach the file; also `wallive.old.log` if the problem happened in the previous run). The log has file paths of your videos; remove anything you don't want to share.
5. For a video that fails to import: its format if you know it (for example "YouTube download, 4K VP9"), or a link.

The issue form asks for these.

## Contributing

Fixes and test reports for the untested setups above are especially welcome.

1. Fork the repository and create a branch from `main` (for example `fix/multi-monitor-offset`).
2. Read [`AGENTS.md`](AGENTS.md) for the project rules. The important ones:
   - nothing heavy in the always-running process (no UI framework, web engine, .NET, FFmpeg);
   - no polling timers where Windows sends an event;
   - `unsafe` code only in the `ffi.rs` files, each block with a `// SAFETY:` comment;
   - state and measure the CPU / memory cost of changes to playback, attach or pause (`tools/bench.ps1`).
3. Before opening a pull request, run:
   ```
   cargo build --release
   cargo clippy --all-targets -- -D warnings
   cargo fmt --check
   cargo test
   ```
4. Open a pull request describing what you changed (CI runs the same checks on Windows), how you tested it (Windows version, monitors, GPU), and the measured cost if relevant. Notes about decisions go in [`docs/decisions.md`](docs/decisions.md), problems found in [`logs/errors.md`](logs/errors.md).

Project documentation: [`docs/architecture.md`](docs/architecture.md), [`docs/testing.md`](docs/testing.md), [`docs/known-issues.md`](docs/known-issues.md).

## License

[MIT](LICENSE)
