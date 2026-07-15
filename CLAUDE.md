# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Current State

This project is at the very beginning. `src/main.rs` is still the default `Hello, world!` skeleton, and `Cargo.toml` declares a single binary crate `byteboy` with no dependencies. The substance of the project lives in the design doc `docs/my/ByteBoy v0 开发文档.md` (Chinese), which is the source of truth for what to build. When implementing, build toward the architecture below rather than the current skeleton.

## What ByteBoy Is

ByteBoy (`boy` is the CLI binary name) is an **AI development environment manager** for macOS (Apple Silicon first). v0 deliberately does NOT implement agents, workflows, or AI logic. It only manages a local AI dev environment:

- **Agent management** — list/launch external AI CLIs (claude, codex, hermes, aider). Launching an agent is just exec'ing its CLI.
- **MLX model management** — start/stop/restart/status/logs for local `mlx_lm.server` processes, run in the background.
- **Doctor** — check that required tools (Rust, Python, Git, uv, the agent CLIs, MLX, ComfyUI) are installed.
- **Config** — read/show/edit a TOML config.

Explicitly out of scope for v0 (deferred to v1+): Skill, Workflow, Prompt, MCP, Memory, Provider, RAG, any LLM API calls, multi-model routing. Don't introduce these unless the work is explicitly v1+.

## Build & Run

```bash
cargo build                 # build
cargo run -- <args>         # run (e.g. cargo run -- agent list)
cargo test                  # run all tests
cargo test <name>           # run a single test by name substring
cargo test -p <crate>       # run tests for one workspace crate (once split into crates)
cargo clippy --all-targets  # lint
cargo fmt                   # format
```

The shipped binary is invoked as `boy` (set via `[[bin]]` name in Cargo.toml). Uses Rust 2024 edition.

## Planned Architecture (target for implementation)

The design doc specifies a **Cargo workspace** split into small, single-responsibility crates under `crates/`:

| Crate | Responsibility |
|-------|----------------|
| `byteboy-cli` | clap command parsing; the `boy` binary entrypoint |
| `byteboy-core` | shared `Context`, `Error`, logging |
| `byteboy-agent` | agent listing and launching |
| `byteboy-model` | MLX model lifecycle (start/stop/status/logs) |
| `byteboy-config` | TOML config loading |
| `byteboy-doctor` | environment checks |

Intended dependency stack: `clap`, `tokio`, `serde`, `toml`, `anyhow`, `tracing`, `directories`, `sysinfo` (process queries), `duct` (running external commands).

### CLI surface

```
boy run <agent>          boy agent list | doctor
boy model list | start <id> | stop <id> | restart <id> | status | logs <id>
boy doctor               boy config show | edit            boy version
```

### Config

Lives at `~/.config/byteboy/config.toml` (resolve via the `directories` crate, not a hardcoded path). Sections: `[agents]` maps a name to a CLI command; `[models.<id>]` defines `name`, `command`, `model` path, `host`, `port`. `boy model start <id>` translates a `[models.<id>]` entry into a backgrounded `mlx_lm.server --model … --host … --port …` invocation; `stop`/`status` find that process (via `sysinfo`).

## Design Principles (from the doc)

CLI-first; configuration over code; small modules; one command = one responsibility; every command independently testable; no Web UI; macOS/Apple Silicon first (Linux later); trait-first design; keep it simple, avoid over-engineering. The v1 Skill abstraction is intended to be a single `trait Skill { async fn execute(ctx: Context) -> Result<()>; }` — keep core types compatible with that direction.
