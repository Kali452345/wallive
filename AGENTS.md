# AGENTS.md - Wallive

## Purpose of This File

This file is the persistent operating manual and handoff document for AI coding agents working on this project.

The project owner may switch between AI coding assistants, local tools, and fresh chat sessions. Therefore, no AI should assume conversation history is available.

Core rule: never rely on chat memory for project state. Record important decisions, progress, errors, fixes, experiments, and unfinished work in the repository.

## Project Mission

Extremely low-resource live video wallpaper for Windows 10 and Windows 11. It plays a looping video behind the desktop icons using the GPU's fixed-function hardware video decoder, converts imported videos to a codec, resolution, and frame rate this PC decodes natively, and pauses automatically when the wallpaper is fully covered. Target: near 0% CPU and 1-5% GPU while playing, near 0% when paused.

## Project Profile

- Project type: Desktop application
- Baseline stack: Rust (windows crate) + Win32 + Media Foundation IMFMediaEngine + Direct3D 11 + DirectComposition + Media Foundation Transcode API; tray-only UI
- Generated: 2026-09-26

Update this section when the project direction or stack changes.

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


## Expected Project Structure

- Cargo.toml
- src/main.rs
- src/desktop/ - WorkerW and Progman attach for both layouts
- src/playback/ - Source Reader, video processor and DirectComposition (ADR-003)
- src/transcode/ - decode capability probe and Media Foundation transcode
- src/occlusion/ - WinEvent hooks and coverage math
- src/power/ - power and session notifications plus EcoQoS
- src/shell/ - tray, autostart, single instance, picker (ADR-011)
- src/config/
- tools/bench/
- docs/
- logs/


## Non-Negotiable Rules

- Read this file before changing code.
- Inspect the actual implementation before proposing or making changes.
- Do not assume current API, package, framework, platform, or provider behavior when the information may have changed.
- For version-sensitive behavior, verify against official documentation or primary sources.
- State when something is an inference rather than a confirmed fact.
- Do not silently replace real behavior with mock/sample data in user-facing flows.
- Mock data is allowed only when clearly isolated for tests, fixtures, or explicit demo states.
- Do not overwrite working implementation without understanding why it exists.
- Do not delete contextual docs, logs, failed attempts, or handoff notes just to make the repository cleaner.
- Never add a UI framework / web engine / WebView2 / .NET runtime / mpv / VLC or any heavy runtime to the always-running process
- Every new feature must state and measure its idle and playing CPU / GPU / RAM cost
- No polling loops or timers when an OS event notification exists
- Keep unsafe Rust confined to thin FFI wrapper modules with a SAFETY comment on every unsafe block
- Test desktop attach on both the classic WorkerW layout and the Windows 11 24H2+ raised-desktop layout
- Do not bundle FFmpeg by default - any FFmpeg fallback must be optional and license-reviewed

## Mandatory First-Run Procedure

Before changing code, every AI must:

1. Read `AGENTS.md` completely or at least all sections relevant to the task.
2. Read `CLAUDE.md` when using Claude or Claude-compatible tooling.
3. Read `PROJECT_BRIEF.md`.
4. Inspect the current Git branch and working tree.
5. Inspect recent commits when the repository has Git history.
6. Read `logs/handoff.md`.
7. Read the latest entries in `logs/progress.md`.
8. Read `docs/decisions.md` if the task involves architecture or technology choices.
9. Read `logs/errors.md` if the task touches an area with known problems.
10. Inspect the actual current implementation before editing.
11. Verify current external documentation for APIs, packages, providers, or platform behavior that may have changed.

Never assume a file exists because an earlier AI said it existed. Check the repository.

## Choosing the Right Approach

AI defaults to the most common solution in its training data. The most common solution usually works, but it is often not the best fit for this project. "It technically works" is not the goal; "it works well for this project's real constraints" is.

### Project Constraints and Context

- Target under 1% CPU and 1-5% GPU while playing 1080p30 and near 0% CPU and GPU when paused
- Working-set RAM target under 30 MB
- Supported OS: Windows 10 22H2 and Windows 11 including 24H2+ (owner runs Windows 11)
- Multiple monitors all show the same video
- Default output codec is H.264 8-bit 4:2:0 (NV12) because every GPU decodes it in hardware and Windows bundles it - use HEVC or AV1 only when the probe confirms both hardware decode and an installed decoder
- Transcode to the largest monitor resolution and a capped frame rate (default 30 fps) with no audio track and a seamless loop
- Prefer observe-only APIs such as out-of-context SetWinEventHook over hooks that intercept input
- Task Manager GPU % is clock-relative - measure with PresentMon / GPU-Z / HWiNFO package power
- Hardware overlay (MPO) for the wallpaper window is unverified and must be tested not assumed

Before implementing a non-trivial technical choice (API, algorithm, library, protocol, storage, or OS integration):

1. Re-read the constraints above and the purpose of the feature in `PROJECT_BRIEF.md`. Know *why* the feature exists, not only *what* was asked.
2. Consider more than the first familiar option. Ask: is there a newer, faster, less invasive, or hardware-accelerated alternative that fits these constraints?
3. Read the Remarks, warnings, and "recommended alternative" notes in the official docs, not only the function signature. Official docs often say when *not* to use an API.
4. Prefer the least invasive mechanism that meets the need: observe instead of intercept, read instead of write, async instead of blocking, scoped instead of global.
5. Match strength to purpose: use security-grade tools where security matters, and faster/lighter tools where it does not. Look for cheap pre-checks that skip expensive work entirely.
6. Consider hidden costs that do not show up in a quick test: latency, blocking/freezing behavior, resource use, failure modes, security exposure, and maintenance burden.
7. When real options exist, present them briefly as "use A when..., use B when..." with a recommendation for this project. Do not bury the choice in exhaustive detail.
8. Record the choice and the rejected alternatives in `docs/decisions.md`.

If the purpose or constraints are unclear and the choice materially changes the result, ask the owner instead of silently picking the generic default.

## Repository Documentation Structure

Maintain this structure as the project grows:

```text
docs/
  architecture.md
  decisions.md
  security.md
  testing.md
  troubleshooting.md
  known-issues.md
logs/
  progress.md
  errors.md
  experiments.md
  handoff.md
```

Create missing files when needed. Do not create unnecessary documentation for trivial changes, but important work must be recorded.

## Progress Logging

After meaningful work, update `logs/progress.md`.

Each entry should answer:

- What was worked on?
- What changed?
- Why was it changed?
- What was verified?
- What remains unfinished?
- What should the next AI do?

Use concise entries, but include enough detail that a new AI can continue without repeating investigation.

## Error and Fix Logging

Record significant errors in `logs/errors.md`.

Include:

- Date
- Area
- Symptoms
- Environment
- Exact relevant error text
- Root cause
- Failed attempts
- Working fix
- Verification
- Related files
- Status: `OPEN` or `RESOLVED`

Do not mark a problem resolved until the fix has been tested.

## Decision Log

Record important architecture, dependency, platform, data-flow, persistence, and security decisions in `docs/decisions.md`.

Use this shape:

```markdown
## ADR-001 - Short decision title

### Decision
What was decided.

### Context
Why the decision was needed.

### Alternatives considered
- Option A
- Option B

### Why this was selected
Explain the tradeoff.

### Revisit when
What evidence would justify changing it.
```

Do not silently change major architecture. Document the change first or immediately after the change.

## Handoff Rules

Update `logs/handoff.md` at the end of major work sessions.

It should always show:

- Current branch
- Last verified build/test
- Current phase
- Working features
- In progress
- Broken or risky areas
- Last change
- Last test
- Known blockers
- Recommended next task
- Files most relevant to the next task

## Git Workflow

- Keep commits focused and descriptive.
- Do not commit dependencies, generated caches, local build output, logs from tools, archives, or large binary artifacts unless the project explicitly tracks them.
- Do not rewrite history or reset work unless the project owner explicitly asks.
- Do not revert unrelated dirty worktree changes.
- Run relevant verification before committing.
- Record the commit hash in `logs/handoff.md` after an important committed checkpoint.

## Verification Rules

- Run `cargo build --release`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` after any Rust change.
- After playback, desktop-attach, occlusion, or power changes, run the benchmark and record CPU / GPU / RAM numbers in `logs/experiments.md`.
- Run relevant tests after runtime, process, parser, queue, or filesystem changes.
- Verify the real desktop user flow, not only helper functions.
- Verify wallpaper attach on the real desktop: icons stay on top, Explorer restart re-attaches, and monitor add/remove/resolution change is handled.

Useful commands for this preset:

- `cargo build --release`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo run --release`

Rust stable (MSVC toolchain, 1.98.1 on 2026-09-26) and Visual Studio Build Tools 2022 are installed on the owner machine.

## Architecture Rules

- Keep UI components separate from filesystem, process, and OS integration logic.
- Build command invocations with argument arrays, not shell-concatenated user input.
- Keep live process handles in a runtime/process manager layer.
- Keep tool-specific behavior under focused modules with shared interfaces.
- For OS integration (input, hooks, hotkeys, file watching), use the least invasive API that meets the need: observe-only APIs before intercepting ones, and dedicated APIs (for example a hotkey API) before general hooks.
- Never do slow work inside blocking OS callbacks; hand it to a worker thread and return immediately so the rest of the system is not stalled.
- Document sidecars, permissions, packaging assumptions, and update behavior.

## Security Rules

- Validate paths and URLs before passing them to OS APIs or sidecar binaries.
- Keep shell/process permissions scoped to known commands.
- Do not add arbitrary command execution from the frontend.
- Treat CLI output, metadata, and downloaded files as untrusted input.
- Do not commit generated caches, local binaries, downloaded media, or build artifacts.

## UI Rules

- Build the actual working product surface first, not a marketing landing page, unless the task is explicitly marketing.
- Keep workflows practical, clear, and efficient for repeated use.
- Use visible instructional text sparingly and only where it helps the user complete the task.
- Verify that UI text does not overflow at desktop and mobile widths.
- Keep accessibility, keyboard behavior, loading states, empty states, and error states in scope for user-facing work.

## When Fixing Bugs

Use this cycle:

```text
Observe -> Reproduce -> Capture evidence -> Identify root cause -> Make the smallest reasonable fix -> Test -> Record result
```

Do not rewrite an entire subsystem because of one error until the root cause is understood.

## Failed Approaches

When an approach fails, document it in `logs/experiments.md`, `logs/errors.md`, or `docs/decisions.md`.

Include:

- Why it was attempted
- What failed
- Evidence
- Why it was rejected
- When it should be reconsidered

## Session-End Procedure

At the end of meaningful work:

1. Verify the project state.
2. Run relevant tests/builds.
3. Update `logs/progress.md`.
4. Update `logs/errors.md` for new significant issues.
5. Update `docs/decisions.md` for architectural choices.
6. Update `logs/experiments.md` for benchmarks or experiments.
7. Update `logs/handoff.md` with the exact stopping point.
8. Commit when appropriate and allowed by the owner.

## Current Project Status

### Current Phase

Feature-complete tray app on the raised desktop (2026-09-26); verification on other setups and RAM reduction remain. See `logs/handoff.md` for the exact stopping point.

### Working Features

- Video behind the icons on every monitor, hardware decode, ~0.9% of the CPU for 1080p30 (ADR-003).
- Import: HW decode probe + transcode to monitor resolution in a child process, cached.
- Pause when covered, fullscreen app, display off, Battery / Energy Saver, lock / disconnect / remote, optional on battery (ADR-005).
- Re-attach after Explorer restart and display change (ADR-008).
- Tray menu, config, Start with Windows, single instance, file log (ADR-011); benchmark script (ADR-010).

### In Progress

- Nothing half-done in code.

### Not Yet Implemented / Not Verified

- RAM target (30 MB; measured ~90-120 MB).
- Classic layout / Windows 10, multi-monitor, exclusive-fullscreen games, saver toggles and battery power not verified on real hardware.

### Known Risks

- External API, framework, package, or platform behavior may change over time.
- Missing verification commands can cause agents to assume success without evidence.
- Sparse handoff notes can make future sessions repeat work.

## Definition of Done

A feature is complete only when:

- Implementation exists.
- The chosen approach fits the documented project constraints, not only the most common approach, and meaningful tradeoffs are recorded.
- Relevant tests exist or testing is explicitly documented as not practical.
- Build/check commands pass where available.
- The real user-facing or developer-facing path is verified.
- Errors are handled.
- Important limitations are documented.
- Progress is logged.
- Handoff state is updated.

## Fast Start for a New AI Chat

```text
1. Read AGENTS.md.
2. Read CLAUDE.md if using Claude.
3. Read PROJECT_BRIEF.md.
4. Read logs/handoff.md.
5. Read latest logs/progress.md entries.
6. Inspect git status and recent commits.
7. Inspect relevant source files.
8. Make the requested change.
9. Test it.
10. Log the result.
11. Update handoff.md.
```

## Golden Rules

1. Read before editing.
2. Verify assumptions against current official sources.
3. Choose the best-fit approach for this project, not the most common one.
4. Do not trust chat history to preserve project state.
5. Write important knowledge into the repository.
6. Log errors and their real fixes.
7. Record failed approaches.
8. Use small, understandable changes.
9. Test the real affected path.
10. Do not silently change architecture.
11. Protect secrets, user data, and local machine state.
12. Leave the project ready for another AI to continue.
