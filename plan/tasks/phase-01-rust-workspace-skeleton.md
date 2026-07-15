# Phase 01: Rust Workspace Skeleton

## Goal

Create the compile-ready Rust workspace and basic local daemon/CLI skeleton.

## Tasks

### 01.1 Workspace

- Create `Cargo.toml` workspace.
- Add crates listed in `04-core-domain-model.md`.
- Add shared lint/format config.
- Add CI build command.

### 01.2 Core Types

- Implement ID newtypes.
- Implement error enum.
- Implement budget types.
- Implement event envelope.
- Implement artifact refs.

### 01.3 SQLite Store

- Add migrations framework.
- Create initial tables:
  - sessions
  - turns
  - artifacts
  - audit_events
  - provider_profiles
- Add repository traits.

### 01.4 CLI

- Add `sakha --version`.
- Add `sakha doctor`.
- Add `sakha run --dry-run`.
- Add config path discovery.

### 01.5 Daemon Stub

- Add `sakhad`.
- Add `GET /health`.
- Add event stream stub.

## Definition of Done

- `cargo build` passes.
- `cargo test` passes for core crate.
- `sakha doctor` prints config/storage paths.
- `sakhad` health endpoint works.

