# Security

## Baseline Rules

- Validate paths and URLs before passing them to OS APIs or sidecar binaries.
- Keep shell/process permissions scoped to known commands.
- Do not add arbitrary command execution from the frontend.
- Treat CLI output, metadata, and downloaded files as untrusted input.
- Do not commit generated caches, local binaries, downloaded media, or build artifacts.

## Secrets

Document required environment variables and where they are allowed to exist. Never commit real secrets.

## Trust Boundaries

- User input:
- Filesystem:
- Network:
- External APIs:
- Build/deployment:

## Known Risks

- Replace this list as implementation details become clear.
