# Error Log

Record significant errors and fixes here.

## 2026-09-26 - Explorer restart not handled until an unrelated message arrived

### Area

`src/runtime/ffi.rs` - message loop / event queue.

### Symptoms

After `Stop-Process explorer -Force`, the spike logged `Waiting("Progman not found")` and then never re-attached, although Explorer restarted within seconds and the host window was alive and responsive. Manually posting `TaskbarCreated` to the host made it re-attach at once.

### Environment

- OS: Windows 11 Pro 10.0.26200
- Runtime: rustc 1.98.1 MSVC, `windows` 0.62.2
- Process not elevated (medium integrity), so UIPI was not involved.

### Error

```text
[  221279 ms] attach: Waiting("Progman not found (Explorer not running?)")
(no "Explorer restarted" line)
```

### Root Cause

`TaskbarCreated` (and `WM_DISPLAYCHANGE`, and out-of-context WinEvent callbacks) are *sent*, not posted. Windows delivers them to the window procedure from inside `GetMessageW`, which then keeps waiting for a posted message. The window procedure only queued the event; the queue was drained after `DispatchMessageW`, which did not run until some unrelated posted message arrived.

### Failed Attempts

1. Suspected the hidden host's `WS_EX_TOOLWINDOW` style blocked the broadcast. Ran a tool-window host and a plain host side by side with logging of every registered message: both received `TaskbarCreated` (0xC0D2) 6.4 s after Explorer died, and neither acted on it. Style ruled out.
2. Suspected UIPI (elevated process). Checked: medium integrity. Ruled out.

### Working Fix

`push()` posts a no-op `WM_WAKE` to the host whenever the event queue goes from empty to non-empty, so `GetMessageW` returns and the loop drains the queue. The WinEvent callback's separate wake post was removed (now covered by `push`).

### Verification

Explorer killed again with the fix: `TaskbarCreated` at 16.16 s, re-attached at 16.25 s; window tree and screenshot correct. `WM_DISPLAYCHANGE` via `SendNotifyMessage` also handled (18 ms).

### Related Files

- `src/runtime/ffi.rs`

### Status

RESOLVED

## 2026-09-26 - Video presented at ~4 fps: DWM throttles vsync presents under the icon layer

### Area

`src/playback/ffi.rs` - swap-chain pacing (backend 2).

### Symptoms

The 1080p30 clip played at ~4 fps. The frame-latency waitable object was released only every ~250 ms (sometimes 500 ms).

### Environment

- OS: Windows 11 Pro 10.0.26200, raised desktop, UHD 620, 60.05 Hz panel
- Runtime: rustc 1.98.1 MSVC, `windows` 0.62.2

### Error

No error; the `playback: N frames/s` log lines reported about 4 frames/s.

### Root Cause

DWM treats our wallpaper visual as occluded (the full-screen `SHELLDLL_DefView` icon layer sits above it), and throttles vsync-synced presents (`Present(1)`, `Present(2)`) for occluded windows to about 4 Hz.

### Failed Attempts

1. `Present(1)` instead of `Present(2)` - identical ~4 fps.
2. `Present(0)` + `IDXGIOutput::WaitForVBlank` x N - `WaitForVBlank` returned immediately for this windowed swap chain, so ~500 fps.

### Working Fix

`Present(0)`, then hold the frame for N compositor ticks with `DCompositionWaitForCompositorClock` (Windows 11; resolved from dcomp.dll at run time), waiting on the control event in the same call. Windows 10 falls back to `Present(n)`; whether it is throttled there is unverified.

### Verification

30.0 fps over 40 s traced runs, loop rewinds exactly every 10.0 s (`logs/experiments.md`).

### Related Files

- `src/playback/ffi.rs`
- `src/playback/mod.rs`

### Status

RESOLVED on Windows 11; OPEN (unverified) on Windows 10.

## 2026-09-26 - Benchmark / test scripts: PowerShell 5.1 pitfalls

### Area

`tools/bench.ps1`, `tools/pause-check.ps1`.

### Symptoms

1. `bench.ps1` could not close wallive: `FindWindowW('WalliveHost', $null)` returned NULL.
2. `pause-check.ps1` failed with "Cannot bind argument to parameter 'Path' because it is an empty string" in the `-Exe` default.
3. Script text written through the Bash tool's heredoc had `\r`, `\t` in Windows paths turned into control characters.

### Environment

- Windows PowerShell 5.1; Git Bash (Bash tool).

### Root Cause

1. `$null` passed to a P/Invoke `string` parameter is marshalled as `""`, not NULL; a window titled "" does not exist.
2. `$PSScriptRoot` is empty inside `param()` default expressions in Windows PowerShell 5.1 when run with `-File`.
3. Shell quoting of backslashes in the heredoc / sed replacement.

### Working Fix

1. Pass `[NullString]::Value`.
2. Default `-Exe` to `''` and resolve it after `param()` from `$MyInvocation.MyCommand.Path`.
3. Write scripts with the file-writing tool (or Python with forward-slash paths) and assert there are no stray `\r` / `\t` after edits.

### Verification

Both scripts ran end to end (results in `logs/experiments.md`).

### Related Files

- `tools/bench.ps1`
- `tools/pause-check.ps1`

### Status

RESOLVED

## Second launch rotated the running instance's log

### Date

2026-09-26

### Area

`src/main.rs` (resident mode, log file).

### Symptoms

After starting `wallive` while it was already running, `wallive.log` held only the second process's lines and the running instance's log had moved to `wallive.old.log`.

### Environment

- Windows 11 26200, release build.

### Root Cause

The log file was opened (and the previous one rotated) before the single-instance mutex was checked.

### Working Fix

`run_resident` acquires `SingleInstance` first and opens the log only after that, and only when it is not the `--play` test mode.

### Verification

Second `wallive` exits with code 3; the running instance keeps writing to the same `wallive.log`.

### Related Files

- `src/main.rs`

### Status

RESOLVED

## Driving the tray from test scripts

### Date

2026-09-26

### Area

Scratch test scripts for the tray menu.

### Symptoms

1. Clicking the tray icon via `Shell_NotifyIconGetRect` opened the overflow flyout instead of the menu.
2. Menu clicks landed ~25% off target.
3. Screenshots with `CopyFromScreen` did not contain the popup menu.

### Root Cause

1. Windows 11 puts new tray icons in the overflow area; `Shell_NotifyIconGetRect` then returns the chevron's rect, even with the flyout open.
2. Windows PowerShell 5.1 is DPI-unaware; the machine runs at 125%.
3. Popup menus are layered windows; `CopyFromScreen` without CAPTUREBLT misses them (the .NET enum rejects the CaptureBlt combination).

### Working Fix

1. Post the tray callback (`WM_APP+3`, `WM_CONTEXTMENU` in LOWORD of lParam, anchor in wParam) to the `WalliveHost` window, as Explorer does.
2. Call `SetProcessDPIAware()` first and use physical pixels.
3. Not needed further: the menu window (`#32768`, owned by the wallive pid) and its rect were located instead, and the clicked command was confirmed in `wallive.log`.

### Status

RESOLVED (workarounds in test scripts only)

## Switching videos failed with 0xC00D36E5

### Date

2026-09-26 (reported by the owner)

### Area

`src/playback/mod.rs` (`Player::open`).

### Symptoms

After choosing "Anime Red Eye" (11 s) while a longer video played, the import finished but the wallpaper stopped: `playback: ...: The operation on the current offset is not permitted. (0xC00D36E5)`. Videos chosen after a longer one also started mid-way.

### Root Cause

`open` set the start position to 0 and then called `restart`, which stops the old video thread and stores *its* position as the start position. The new video was sought to the old video's position; past its end, `SetCurrentPosition` fails.

### Working Fix

Stop the old thread before resetting the position. A resume seek that fails now starts from 0 instead of stopping playback.

### Verification

Real tray app: a 33 s video played for 26 s, then `wallive <Red Eye>`: import, switch, 30 fps.

### Related Files

- `src/playback/mod.rs`

### Status

RESOLVED

## Fragmented MP4 (YouTube / DASH) imported zero frames

### Date

2026-09-26

### Area

`src/transcode/`.

### Symptoms

`videoplayback.mp4` (1080x1920 H.264, DASH fragmented): `fatal: The operation failed because no samples were processed by the sink. (0xC00D4A44)`. `--bench-decode` on the source: 0 frames.

### Root Cause

Media Foundation's MP4 source reads no frames from a fragmented MP4 whose track has an edit list (`edts`/`elst`, here media time 512). Renaming only `sidx` or the `dash` brand did not help; renaming `edts` to `free` gave all 986 frames. The misleading sink error came from finalizing a writer that got no samples.

### Working Fix

Import reads the first frame before creating the encoder. No frame + fragmented MP4 with edit lists: retry from a temporary copy with `edts` renamed to `free` (`src/transcode/mp4.rs`, 4 tests). No frame otherwise: "source has no decodable video frames".

### Verification

Real tray app: `wallive videoplayback.mp4` imported 986 frames in 7.2 s and played at 30 fps; the temporary copy is deleted.

### Related Files

- `src/transcode/mod.rs`
- `src/transcode/mp4.rs`

### Status

RESOLVED

## Playback read the SSD on every loop

### Date

2026-09-26 (noticed by the owner)

### Area

`src/playback/ffi.rs` (`Reader::open`).

### Symptoms

Constant disk activity (~1.1 MB/s) while playing a cached 11 MB video, with 7 GB of RAM free.

### Root Cause

Media Foundation's file byte stream (by URL, and also `MFCreateFile` with `MF_FILEFLAGS_NONE`) did not use the system file cache: physical reads matched the process's reads.

### Failed Attempts

- `MFCreateFile(MF_ACCESSMODE_READ, MF_OPENMODE_FAIL_IF_NOT_EXIST, MF_FILEFLAGS_NONE)`: unchanged.

### Working Fix

`SHCreateStreamOnFileEx` + `MFCreateMFByteStreamOnStream`, content type `video/mp4`. Physical reads while playing: ~0.

### Verification

A/B in `logs/experiments.md`.

### Related Files

- `src/playback/ffi.rs`

### Status

RESOLVED

## Decoder surface pool grows after a decode burst

### Date

2026-09-26

### Area

`src/playback/mod.rs` (resume after the decoder was released).

### Symptoms

After a long pause the reopened decoder used 157.6 MB of GPU memory instead of 62.7 MB.

### Root Cause

Resume decoded the frames between the key frame and the saved position as fast as possible without showing them; the decoder grew its surface pool to keep up and never shrank it. Separately, released D3D11 resources were not freed until the context was flushed.

### Working Fix

Resume at the key frame (imports now have one every second), no burst. `ClearState` + `Flush` + `IDXGIDevice3::Trim` after releasing.

### Verification

Two pause cycles: paused 29 MB private, resumed 81 MB (same as a fresh start).

### Related Files

- `src/playback/mod.rs`, `src/playback/ffi.rs`

### Status

RESOLVED

## Stale playlist switch after "Next video"

### Date

2026-09-26

### Area

Playlist timer (`src/runtime/ffi.rs`, `src/runtime/mod.rs`), ADR-012.

### Symptoms

With the tray menu open, the switch timer fired. After choosing "Next video", the new 10 s clip was marked to switch again at its first loop end: the log showed `playlist: switching at the end of this loop` 30 ms after `playlist: video 1/3`.

### Root Cause

`TrackPopupMenuEx` runs a modal loop that still dispatches `WM_TIMER`, and the window procedure queued `Event::SwitchDue`. The app handles queued events only after the menu call returns. By then the menu choice had switched videos and re-armed the timer, but the stale event was still in the queue.

### Working Fix

`Host::set_switch_timer` / `kill_switch_timer` also remove a queued `SwitchDue` (`drop_pending`). The loop pops events one at a time, so a handler can remove later ones.

### Verification

Rebuilt; later switches in the real tray app happen only at the first loop end after the timer (`logs/experiments.md`).

### Status

RESOLVED

## Test script keys went to the terminal

### Date

2026-09-26

### Area

Scratch test scripts for the tray menu.

### Symptoms

Arrow / Enter keys sent with `keybd_event` after opening the tray menu from a script did not move through the menu. The menu stayed open.

### Root Cause

The menu is opened by a posted tray message. `SetForegroundWindow` from the background wallive process does not get the foreground, so keyboard input went to the foreground window (Windows Terminal). Posting `WM_CANCELMODE` to the owner window did not close the menu.

### Working Fix

Use a mouse click on the item. The menu `#32768` window rect (owned by the wallive pid) gives the item positions; clicking an item or outside the menu closes it. The cursor is put back afterwards.

### Status

RESOLVED (test scripts only)

## Template

### Date

YYYY-MM-DD

### Area

Affected subsystem.

### Symptoms

What failed.

### Environment

- OS:
- Runtime:
- Framework:
- Relevant versions:

### Error

```text
Paste relevant error text here.
```

### Root Cause

Explain the actual cause.

### Failed Attempts

1. Attempt - why it failed.

### Working Fix

What changed.

### Verification

How the fix was tested.

### Related Files

- `path/to/file`

### Status

OPEN
