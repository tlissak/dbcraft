# DbCraft

**A native database manager in pure Rust, aiming for Navicat Premium parity.**
Connections, object browser, SQL editor and result grid for SQLite, PostgreSQL, MySQL and MariaDB, in a native egui app with no Electron and no web view. Every action is a command, so the UI, the CLI and an MCP server (for AI agents) all drive the same engine.

Built with the method of [PhotoCraft](https://github.com/storytold/photocraft): engine-first, layered crates, never crash, tests are the gate.

> **Status:** early alpha. See [`ROADMAP.md`](ROADMAP.md) for every Navicat feature and its status, by priority.

## Get started

```sh
cargo run --release -p dbcraft          # desktop app
cargo run -p dbcraft-cli -- commands    # headless: list every command
cargo test --workspace
```

Test servers: `docker compose -f dev/docker-compose.yml up -d` (details in [`docs/development.md`](docs/development.md)).

## Built for agents

```sh
dbcraft-cli cmd connection.save '{"kind":"sqlite","name":"Local","database":"app.db"}'
dbcraft-cli sql conn-1 "select name from sqlite_master"
dbcraft-cli mcp      # MCP server on stdio (registered in .mcp.json for Claude Code)
```

New contributors and AI agents: start with [`AGENTS.md`](AGENTS.md).

<sub>Navicat is a trademark of PremiumSoft CyberTech Ltd. DbCraft is an independent project, not affiliated with or endorsed by PremiumSoft; the name is used only to describe compatible workflows.</sub>
