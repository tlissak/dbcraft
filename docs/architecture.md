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

`Connection` is synchronous on purpose (simple engine, simple tests) and shared: methods take `&self`, the trait is `Send + Sync`, and the session holds `Arc<dyn Connection>`. Each driver owns sqlx pools (4 connections; 1 for an in-memory SQLite) and blocks on a shared 2-thread tokio runtime (`drivers/src/common.rs`). Cell decoding tries candidate Rust types per engine in order (`decode_row!`); an unknown type shows as `<TYPE>` instead of failing the whole query.

PostgreSQL scopes connections to one database, so `PgConn` keeps one pool per database. MySQL switches with `USE` on an acquired connection.

## Background work and Stop

Nothing slow runs on the UI thread.

- **Tasks** (`ui-egui/src/tasks.rs`) do interactive work: opening a connection, running a query or console statement, loading a table page, object lists or DDL. A task runs engine commands on a worker over `Session::fork_shared()`, which shares the live connections. When it finishes, the UI thread calls `Session::absorb(worker)` to take back newly opened connections, caches, the last result and messages; a connection the user closed meanwhile stays closed. Query, table and console tabs own their task. The tree's open/load tasks live in `DbCraftApp::bg`, and actions waiting for a connection that is still opening are queued in `after_open`.
- **Jobs** (`ui-egui/src/jobs.rs`) run long tools (transfer, backup, automation) on a `Session::fork` with their own connections and a progress log.
- **Stop** (Ctrl+T, the Stop buttons, closing a tab): `dbcraft_drivers::Cancel`. A worker runs under `with_cancel`, and every driver `block_on` on that thread races the Stop signal, so connect, metadata and queries can all be stopped. Drivers also stop the statement server-side: PostgreSQL `pg_cancel_backend(pid)` and MySQL/MariaDB `KILL QUERY id` over a side connection, and SQLite through a progress handler. A stopped call returns `Error::Cancelled`.
- UI tests use `DbCraftApp::waiting_for_tasks()` so each task finishes before the next frame. `tests/ui_async.rs` drives the real asynchronous path.

## Persistence

`Workspace` is saved as JSON (atomic write + rename) at `%APPDATA%\dbcraft\DbCraft\config\workspace.json`, or `DBCRAFT_WORKSPACE`. A corrupt file is reported in the status bar and left untouched; the app starts empty.

Passwords: the user's workspace keeps them in the OS keychain (`engine/src/secrets.rs`, `Session::use_os_keychain`). In memory they are plain `ConnectionConfig` fields. On save, `secrets::for_file` writes changed ones to the keychain and strips all of them from the JSON. Workspaces set with `DBCRAFT_WORKSPACE` keep their mode (`password_storage`, file by default) so dev and test runs never touch the real keychain; tests use `secrets::MemoryStore`.

## Menu catalog and parity

`ui-egui/src/menu_catalog.rs` lists Navicat's menu items with the command id each one maps to. An item is live when the id is registered. `cargo xtask parity` writes `docs/parity-checklist.md`; `PARITY_FLOOR` keeps it from regressing.
