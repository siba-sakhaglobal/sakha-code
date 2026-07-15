# Module 18: Packaging and Deployment

## Responsibility

Package Sakha as CLI, TUI, desktop, local daemon, Docker image, and optional server deployment.

## Components

- `sakha` CLI binary.
- `sakhad` local daemon.
- Tauri desktop app.
- Web app.
- Docker image.
- MCP server package.

## Packaging Requirements

- Cross-platform binaries for Windows, macOS, Linux.
- Signed binaries where possible.
- Reproducible build metadata.
- No embedded secrets.
- Versioned schema migrations.
- Upgrade rollback plan.
- Offline install option.

## Docker

Images:

- `sakha-cli`
- `sakha-daemon`
- `sakha-web`
- `sakha-all-in-one`

Volumes:

- `/data/sakha`
- `/workspaces`

## Deployment Modes

- Local CLI only.
- Local daemon + web UI.
- Desktop app with embedded daemon.
- Team server.
- CI runner.
- Self-hosted automation worker.

## Implementation Tasks

1. Create Rust release profile.
2. Create build scripts.
3. Create Dockerfiles.
4. Create Tauri packaging.
5. Create schema migration runner.
6. Create install script.
7. Create update checker.
8. Create release manifest.
9. Create SBOM.
10. Create smoke tests per platform.

## Tests

- CLI binary runs.
- Daemon starts and health check passes.
- Docker image runs loop smoke test.
- Desktop connects to daemon.
- Migration runs on fresh and existing DB.

