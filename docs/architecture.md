# Architecture

## Layers

| Layer | Crate | Role | May use |
|---|---|---|---|
| L0 | `dbcraft-core` | `Value`, `ResultSet`, `ConnectionConfig`, `DbKind`, `Error` | serde, thiserror |
| L0 | `dbcraft-sql` | Statement splitting, identifier quoting per `Dialect` | nothing (standalone) |
| L1 | `dbcraft-model` | `Workspace` (saved connections), `DbObject`, `ObjectKind` | L0 |
| L2 | `dbcraft-drivers` | `Connection` trait + SQLite/PostgreSQL/MySQL/MariaDB on sqlx | L0–L1, tokio, sqlx |
| L3 | `dbcraft-engine` | `Session`, command registry, `dispatch` | L0–L2 |
| L4 | `dbcraft-ui-egui` | egui shell, menu catalog, parity | L0–L3, egui, eframe, rfd |
| L4 | `dbcraft-automation` | MCP server over stdio | L0–L3 |
| L5 | `dbcraft`, `dbcraft-cli` | Binaries | everything |

`cargo xtask layers` enforces this.

## The engine/UI seam

```text
 egui UI ──┐
 CLI ──────┼──> Session::dispatch(id, json params) ──> CommandSpec.run ──> drivers::Connection
 MCP ──────┘                                    │
                                                └──> Session state (workspace, open connections,
                                                     databases, objects, last_result, messages)
```

- A **command** is `CommandSpec { id, label, menu, shortcut, params_doc, enabled, run }`. Params and results are JSON so the same command serves the UI, CLI and MCP.
- The UI reads `Session` fields directly to draw, but **mutates only through `dispatch`**. Drawing collects `Action`s, applied after the frame.
- `dispatch` wraps `run` in `catch_unwind`: an escaped panic becomes an error and the session survives.

## Drivers

`Connection` is synchronous on purpose (simple engine, simple tests). Each driver owns an sqlx pool and blocks on a shared 2-thread tokio runtime (`drivers/src/common.rs`). Cell decoding tries candidate Rust types per engine in order (`decode_row!`); an unknown type shows as `<TYPE>` instead of failing the whole query.

PostgreSQL scopes connections to one database, so `PgConn` keeps one pool per database. MySQL switches with `USE` on an acquired connection.

**Known limitation:** queries run on the UI thread, so a slow query freezes the window. Moving `dispatch` for long commands to a worker (with cancel) is item 1.1 in `ROADMAP.md`.

## Persistence

`Workspace` is saved as JSON (atomic write + rename) at `%APPDATA%\dbcraft\DbCraft\config\workspace.json`, or `DBCRAFT_WORKSPACE`. A corrupt file is reported in the status bar and left untouched; the app starts empty.

## Menu catalog and parity

`ui-egui/src/menu_catalog.rs` lists Navicat's menu items with the command id each one maps to. An item is live when the id is registered. `cargo xtask parity` writes `docs/parity-checklist.md`; `PARITY_FLOOR` keeps it from regressing.
