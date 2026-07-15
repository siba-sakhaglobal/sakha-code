# Tech Stack Decision

## Recommended Stack

| Layer | Choice | Reason |
|---|---|---|
| Core runtime | Rust | Safety, performance, single binaries, strong concurrency |
| Async runtime | Tokio | Mature async I/O, process, scheduling, timers, networking |
| CLI | Rust + `clap` | Typed command definitions, reliable packaging |
| TUI | Rust + `ratatui` or `crossterm` | Native terminal session UI |
| Local daemon | Rust + `axum`/`tower` | Local API, streaming, WebSocket, SSE |
| Desktop | Tauri + React/Svelte | Rust backend with web frontend and small native app |
| Web app | TypeScript + React/Next.js | Fast UI iteration and ecosystem |
| Storage | SQLite + `sqlx` or `rusqlite` | Local-first durable sessions and memory |
| Search index | Tantivy + optional vector DB | Local code search and semantic retrieval |
| Provider HTTP | `reqwest` + SSE parser | Streaming provider calls |
| Serialization | `serde`, JSON Schema | Stable contracts and config |
| Observability | `tracing`, OpenTelemetry optional | Structured logs and spans |
| Sandboxing | OS-specific adapters | macOS sandbox, Linux namespaces/bubblewrap, Windows job objects |
| Plugins | MCP + WASI optional | Protocol-first plugin boundary |

## Why Rust Core

Sakha is mostly orchestration: filesystem, shell, PTY, git, streaming APIs, MCP, subprocesses, scheduling, and long-running state. Rust is the best fit because it gives memory safety, thread safety, and predictable distribution as native binaries.

## Where TypeScript Still Belongs

- Web frontend.
- Tauri frontend.
- Optional plugin SDK for web developers.
- MCP examples.

Do not put the core agent loop in TypeScript unless speed of prototyping is more important than reliability.

## Where Python Belongs

- Optional eval scripts.
- ML/compression experiments.
- Headroom library integration if its Python package is the most mature path.
- One-off data migration tools.

Do not make Python the core runtime unless the product becomes research-only.

## Where Go Is a Viable Alternative

Go is acceptable if team velocity matters more than Rust guarantees. It is simpler for server/CLI teams and has excellent concurrency, but weaker for deep type-driven safety in complex tool orchestration.

## Final Decision

Use:

- Rust for core runtime, daemon, CLI, tools, provider gateway, compression adapter, scheduler, and sandbox.
- TypeScript/React for web and desktop UI.
- SQLite for local state.
- MCP for extension boundary.
- Headroom as an external compression dependency/service with a first-class adapter.

