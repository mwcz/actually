# AGENTS.md

## Project Overview

`actually` is a Rust CLI tool that orchestrates multiple Claude Code agent instances to generate competing, contrarian strategies for a given task. It forces creative thinking by making each subsequent agent reject all prior strategies, producing increasingly unconventional approaches.

Requires Claude Code to be installed and available as `claude` on `$PATH`.

## Commands

```bash
cargo build              # Build
cargo test               # Run tests (5 unit tests in strategy module)
cargo clippy             # Lint
cargo fmt                # Format
cargo run -- "prompt"    # Run with a task prompt
```

There are no CI configs, Makefile, or deploy scripts. Cargo.lock is gitignored (treat as application, but lock is not committed).

## Code Organization

```
src/
├── main.rs         # CLI entry point (clap Args, tokio runtime, signal handling)
├── conductor.rs    # Core orchestration: 3-phase pipeline + ratatui TUI
├── session.rs      # Claude Code SDK wrapper (strategy queries + implementation runs)
├── strategy.rs     # Prompt templates, strategy parsing, markdown extraction
├── workspace.rs    # Per-instance workspace directory creation
└── output.rs       # Run output directory structure and session log writing
```

All modules are declared in `main.rs` as `mod` siblings (flat module structure, no `lib.rs`).

## Architecture & Phases

The tool runs in three phases:

1. **Phase 1 — Strategy Collection** (sequential): Agents run one at a time. Each sees the strategies of all prior agents and must propose something "utterly different." Agents run in `PermissionMode::Plan` (read-only, no writes, no commands).

2. **Phase 2 — Interactive TUI Review** (optional, default): A ratatui-based TUI lets users preview, edit (`$EDITOR`), delete, add, copy, or chat about strategies. Chat spawns an interactive `claude` subprocess. No agents are active in this phase.

3. **Phase 3 — Parallel Implementation** (optional, user-triggered): All strategies are implemented in parallel. Agents run with `PermissionMode::BypassPermissions` (`--dangerously-skip-permissions`).

## Key Dependencies

| Crate | Purpose |
|---|---|
| `claude-code-agent-sdk` | SDK for spawning/querying Claude Code agents |
| `tokio` | Async runtime (full features) |
| `clap` (derive) | CLI argument parsing |
| `ratatui` + `crossterm` | Terminal UI for strategy review |
| `arboard` | Clipboard access (wayland-data-control) |
| `serde` + `serde_json` | Serialization |
| `thiserror` / `anyhow` | Error handling (thiserror for module errors, anyhow at top level) |
| `tracing` + `tracing-subscriber` | Logging (suppressed in interactive mode, active in `--headless`) |
| `tempfile` | Temp files for editor-based strategy editing |
| `futures` | `join_all` for parallel implementation, `StreamExt` for streaming |

## Conventions & Patterns

### Error Handling
- Module-level errors use `thiserror` enums (`SessionError`, `WorkspaceError`, `OutputError`)
- Top-level `main()` uses `anyhow::Result`
- Errors in non-critical paths (like writing strategy files) are logged with `tracing::warn!` but don't halt execution

### Output Modes
- **Interactive mode** (default): All output via `println!`, tracing is disabled (`"off"` filter)
- **Headless mode** (`--headless`): Output via `tracing` macros, controlled by `--verbose` flag or `RUST_LOG`

### Strategy Format
- Agents reply with `STRATEGY: <text>` prefix
- Strategy text uses markdown with `**bold**` markers for key qualities
- `parse_strategy()` extracts text after `STRATEGY:` prefix, falls back to raw response (first 500 chars)
- `Strategy` struct separates: `markdown` (original), `raw` (stripped), `highlights` (bold phrases)

### Output Directory Structure
```
actually-{unix_timestamp}/
├── C0-strategy.md        # Strategy files (written during Phase 1)
├── C1-strategy.md
├── c0/                   # Workspace dirs (created during Phase 3)
│   └── session.log
├── c1/
│   └── session.log
└── ...
```

### TUI Patterns
- `conductor.rs` contains all TUI code (ratatui rendering, event handling, markdown-to-styled-text conversion)
- TUI exits temporarily for editor/chat operations, then re-enters
- Help popup overlays the main view
- Preview panel appears when terminal width >= 100 columns
- Markdown rendering supports headers, code blocks, bold, inline code, bullet/numbered lists

## Testing

Tests exist only in `src/strategy.rs` (`mod tests`). They cover:
- Strategy prompt building (with/without exclusions)
- Strategy parsing from agent responses (with/without `STRATEGY:` prefix)
- `Display` trait implementation

No integration tests, no test fixtures, no mocking of Claude Code SDK. The `session.rs`, `conductor.rs`, and other modules have no tests.

## Gotchas

- **Cargo.lock is gitignored** — despite this being a binary crate. Dependencies may resolve differently across machines.
- **Phase 3 runs with `BypassPermissions`** — implementation agents can do anything. This is intentional and documented.
- **The `claude` CLI must be on PATH** — the chat feature (`t` key in TUI) spawns `claude` as a subprocess directly.
- **TUI disables/re-enables raw mode** when shelling out to `$EDITOR` or `claude` — if the process crashes mid-edit, the terminal may be left in raw mode.
- **Tracing is completely off in interactive mode** — don't expect log output unless `--headless` is used or `RUST_LOG` env var is set.
- **`truncate()` function is duplicated** — exists in both `main.rs` (`truncate`) and `conductor.rs` (`truncate_for_log`) with identical logic.
- **Strategy indices shift on delete** — when a strategy is removed in the TUI, all subsequent C-indices shift. Strategy files on disk may become stale/mismatched.
