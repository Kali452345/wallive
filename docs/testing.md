# Testing

## Verification Commands

- `cargo build --release`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo run --release`

## Required Checks

- Run `cargo build --release`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` after any Rust change.
- After playback, desktop-attach, occlusion, or power changes, run the benchmark and record CPU / GPU / RAM numbers in `logs/experiments.md`.
- Run relevant tests after runtime, process, parser, queue, or filesystem changes.
- Verify the real desktop user flow, not only helper functions.
- Verify wallpaper attach on the real desktop: icons stay on top, Explorer restart re-attaches, and monitor add/remove/resolution change is handled.

## Manual Test Matrix

- Primary happy path:
- Error path:
- Empty/loading state:
- Permission/auth state:
- Regression cases:

## Last Known Good

- Date:
- Commit:
- Commands run:
- Manual checks:
