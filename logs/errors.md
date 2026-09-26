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
