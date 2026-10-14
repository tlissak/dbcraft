# Development

## Prerequisites (Windows)

| Tool | Why | Install |
|---|---|---|
| Rust stable (≥ 1.95) + clippy, rustfmt, wasm32 target | everything | `winget install Rustlang.Rustup` (pinned via `rust-toolchain.toml`) |
| MSVC Build Tools (C++ workload) | linker; SQLite is compiled from C by `libsqlite3-sys` | Visual Studio Build Tools |
| Git | | |
| Docker Desktop | PostgreSQL/MySQL/MariaDB test servers | |
| Navicat Premium 16 | the reference app to observe | already installed |

## Build and run

```sh
cargo build --workspace
cargo run -p dbcraft                         # desktop app
cargo run -p dbcraft-cli -- commands         # list commands
cargo run --release -p dbcraft               # optimized
```

Use a dev workspace so you never touch your real saved connections:

```sh
export DBCRAFT_WORKSPACE=dev/data/workspace.json      # bash
$env:DBCRAFT_WORKSPACE = "dev/data/workspace.json"    # PowerShell
```

## Test

```sh
cargo test --workspace                                  # unit + SQLite + panic hunt
docker compose -f dev/docker-compose.yml up -d          # test servers (ports 15432, 13306, 13307)
cargo test -p dbcraft-drivers --test servers -- --ignored
cargo xtask ci                                          # fmt, clippy -D warnings, test, layers
```

Test-server connections (user/password `dbcraft`/`dbcraft`, database `sakila_lite`):

```sh
cli=target/debug/dbcraft-cli.exe
$cli cmd connection.save '{"id":"pg","kind":"postgres","name":"PG (docker)","host":"127.0.0.1","port":15432,"user":"dbcraft","password":"dbcraft","database":"sakila_lite"}'
$cli cmd connection.save '{"id":"my","kind":"mysql","name":"MySQL (docker)","host":"127.0.0.1","port":13306,"user":"dbcraft","password":"dbcraft","database":"sakila_lite"}'
$cli sql pg "select * from film_list"
```

## Drive it from an agent

- CLI: `dbcraft-cli cmd <id> '<json>'`. Each process is a fresh session (connections are re-opened); the workspace persists.
- MCP: `dbcraft-cli mcp` (stdio). `.mcp.json` registers it for Claude Code once `target/debug/dbcraft-cli.exe` is built. The MCP session is long-lived, so `connection_open` then `query_run` works across calls.

## Adding a command (checklist)

1. Pick the id from `crates/ui-egui/src/menu_catalog.rs` (add the menu item there if missing).
2. Add it in `crates/engine/src/<area>_cmds.rs` (`specs()`), register the module in `commands.rs`.
3. Validate every param via `params.rs`; return `Err`, never panic.
4. Test: a happy-path test plus a graceful-failure test. `panic_hunt` covers it automatically.
5. If it needs UI (a dialog), add it in `ui-egui/src/app.rs` `menu_item`.
6. `cargo xtask parity`, raise `PARITY_FLOOR`, update `ROADMAP.md`.

## Adding a driver (checklist)

1. New module in `crates/drivers/src/`, implement `Connection`, route it in `connect()` and `supported_kinds()`.
2. Add a docker service in `dev/docker-compose.yml` and seed SQL.
3. Add it to `crates/drivers/tests/servers.rs`.
