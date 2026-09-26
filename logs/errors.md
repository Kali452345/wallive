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
