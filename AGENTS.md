# AGENTS.md: guide for AI agents and contributors

DbCraft is an open-source, native, **Navicat Premium-comparable database manager** written in **Rust only** (no JavaScript or TypeScript). **No Tauri, Electron or webview shells:** the desktop app is native egui/eframe on wgpu. The product name is always written **DbCraft** in user-facing text (UI, window titles, About, installers, docs prose). Machine names stay lowercase: crates (`dbcraft-*`), binaries, file names. The goal is 1:1 Navicat Premium parity (same menus, shortcuts, dialogs, workflows) with better performance, and every feature drivable by agents. This project follows the method of [storytold/photocraft](https://github.com/storytold/photocraft) (engine-first, everything is a command, layered crates, never crash, tests are the gate). Read this file first, then `docs/`.

The reference app is the locally installed **Navicat Premium 16** (`C:\Program Files\PremiumSoft\Navicat Premium 16`). Use it to observe behaviour and look.

## 1. Orientation (5 minutes)

| Read | Why |
|---|---|
| `docs/architecture.md` | Crate map, layers, the engine/UI seam, data model |
| `docs/development.md` | Build, test, run, drive the app from the CLI/MCP, docker test servers |
| `ROADMAP.md` | Every Navicat feature by priority (P0–P4) with its status: **the work list to pick from** |
| `docs/navicat-reference.md` | What each Navicat feature does (from the public manual): dialogs, fields, grid behaviour, shortcuts |
| `docs/parity-checklist.md` | Generated: every Navicat menu item, live or missing |
| `log/devlog.md` (local, gitignored) | What the last sessions did and what's still open |

## 2. Workspace map

```text
crates/
  core sql            L0 foundation (values, result sets, connection config, errors) / standalone SQL text utils
  model               L1 workspace model: saved connections, object tree (pure data, serde)
  drivers             L2 database drivers behind one blocking `Connection` trait (sqlx: SQLite, PostgreSQL, MySQL/MariaDB) + SSH tunnel (russh)
  engine              L3 Session + command registry (every action is a command)
  ui-egui automation  L4 egui shell: theme, icons, tree, Objects tab, table viewer, connection dialog (thin: all actions go through the engine) / MCP server
apps/
  dbcraft             L5 desktop app (eframe/wgpu)
  dbcraft-cli         L5 headless CLI (commands / cmd / sql / mcp)
xtask/                cargo xtask layers | parity | ci
dev/                  docker-compose test servers (PostgreSQL, MySQL, MariaDB, SSH bastion) + seed SQL
```

**Layering is enforced** by `cargo xtask layers`. A crate may depend only on lower layers. `sql` depends on nothing in the workspace. Nothing below L4 may use egui, eframe, egui_extras, egui-phosphor, winit or rfd. A new crate must be registered in `LAYERS` in `xtask/src/main.rs`.

## 3. Golden rules

### Never crash (outranks feature work)

People trust a database tool with production data; a crash can lose an unsaved query or leave a transaction hanging. A bad server, malformed result, wrong param, corrupt workspace file or full disk must produce an error the user or agent can act on, never a panic.

- **Non-test code never panics.** No `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`. The workspace lints deny them; `clippy.toml` allows them in tests only. Return the crate's `Result` and propagate with `?`. Unfinished features return `Error::Unsupported`.
- **No `unsafe`.** The workspace sets `unsafe_code = "forbid"`.
- **Input-derived numbers are hostile.** Use `get()` not `[i]` for indices from params, results or arithmetic on them; `checked_*`/`saturating_*` for counts; cap row limits and allocations sized by input.
- **Last-resort guard.** `Session::dispatch` catches an escaped panic and reports it; it's a safety net, not a licence to panic.
- **Prove it.** Every crash fix comes with a regression test that panicked before the fix. The `panic_hunt` test (`crates/engine/tests/panic_hunt.rs`) fuzzes every command with adversarial params and must stay green.

### The rules

1. **Everything is a command.** New user-visible behaviour = a command in the engine (`crates/engine/src/<area>_cmds.rs` with a `specs()` fn, registered in `commands.rs`) with id, label, menu path, shortcut, params doc, `enabled` and `run`, plus tests. The UI, CLI and MCP all dispatch commands by id. Use the **exact id from `crates/ui-egui/src/menu_catalog.rs`** and the menu item goes live automatically. Only pure view/window state belongs to the shell (`UI_COMMANDS` in `ui-egui/src/lib.rs`).
2. **No engine assumptions.** Never hard-code one engine's SQL in the engine or UI. Engine-specific SQL (catalog queries, quoting, paging) lives in the driver or goes through `dbcraft_sql::quote_ident` with the connection's `Dialect`. Every metadata feature must handle "this engine doesn't have that" with `Unsupported`, not a wrong answer.
3. **Clean-room.** Study Navicat for *behaviour and look only*. Never copy its code, binaries, resources, icons, help text or license-protected assets, and never decompile or disassemble it. Implement from public docs (engine manuals, wire protocols) and observation. Third-party assets must be permissively licensed and credited.
4. **Tests are the gate.** Every change comes with tests. Drivers: SQLite tests always run; server tests run against `dev/docker-compose.yml` (`--ignored`). Keep the parity floor (`PARITY_FLOOR` in `crates/ui-egui/src/lib.rs`) from regressing.
5. **The UI is thin and data-driven.** UI state lives in `ui-egui/src/state.rs` (serde). Drawing code collects `Action`s and applies them after the frame; it never talks to a driver directly.
6. **Verify UI changes.** Drive the real app headlessly with egui_kittest (`crates/ui-egui/tests/ui_*.rs`: click, type, press keys, then assert on the database or `UiState`). Every UI feature gets one. To look at it, run the app against the dev workspace (`DBCRAFT_WORKSPACE=dev/data/workspace.json`) with startup flags that land on the screen you changed: `--select <conn>[/<db>]`, `--open-table <conn>/<db>/[<schema>.]<table>`, `--new-connection`, `--console <conn>[/<db>]`, `--tool transfer|automation`, `--mode queries|backups|users`. Capture only the app window (never the whole desktop). Colours come from `theme::Tokens::*()` (current light/dark palette; never hard-code a UI colour), sizes from the theme; icons are Phosphor glyphs via `icons.rs`. kittest lessons (each cost a debugging session):
   - After changing state with `state_mut()`, run a frame before locating widgets: rects are from the last frame.
   - kittest doesn't advance time, so two clicks are a triple-click; the grid also edits on click-a-selected-cell.
   - egui ComboBoxes don't expose their selected text; set picker values via `state_mut()`.
   - Ctrl+C arrives as `egui::Event::Copy`, not a key press; send that event to test copying.
   - Don't right-align buttons in a resizable `Panel`: they end up under the resize handle.
   - Check `lost_focus()` before calling `request_focus()` again, or Enter is swallowed.
   - Tests/shots of things running in a worker (`jobs::Job`) must `step()` until the job finishes.
   - An auto-sized `egui::Window` with a vertical separator (or anything that fills the available height) grows every frame, and clicks then land below their target. Give dialogs `fixed_size`, and assert that a rect stays put across frames.
   - Build test apps with `DbCraftApp::new(..).waiting_for_tasks()` so queries, opening and page loads finish before the next frame. Only `ui_async.rs` leaves it off on purpose, to test the asynchronous path.
7. **Secrets are sacred.** Never log passwords or full connection strings, never return passwords from `connection.list` or MCP, never commit credentials (dev docker creds excepted).
8. **Long work runs in the background; the UI thread never waits on a server.** Interactive work (opening, queries, table pages, object lists, DDL) runs as a `tasks::Task` over `Session::fork_shared` (shared connections, Stop with Ctrl+T, results merged back with `Session::absorb`). Long tools (transfer, backup, restore, batch jobs) run as `jobs::Job::spawn(ctx, &session, command, params)`: a forked `Session` with its own connections on a worker thread, progress lines through `Session::report`, results polled in `DbCraftApp::poll_jobs`. Engine commands meant for jobs open connections on demand (`Session::conn_auto`) and are `enabled: always`. Drivers must keep every call cancellable (go through `common::block_on`) and register a server-side stop for long statements (`common::on_cancel`).
9. **Performance is a feature.** Results can be millions of rows: page (`table.open` limit/offset), stream where possible, never re-query per frame, and keep the UI responsive.

## 4. Picking work

1. Read `ROADMAP.md`: take the first ⬜/🟡 item in priority order (P0 → P4) unless the owner asks otherwise. Check `docs/navicat-reference.md` for how Navicat behaves before designing it.
2. `cargo xtask parity` → `docs/parity-checklist.md` lists every missing menu item. Low-hanging fruit: a missing command whose SQL is simple.
3. `log/devlog.md` → the "Still open" bullets of recent entries.

When parity rises, raise `PARITY_FLOOR` (never lower it). When you finish a roadmap item, flip its status in `ROADMAP.md` and add the date.

## 5. Before you finish a task

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask layers
cargo xtask parity          # if you added commands; commit the regenerated docs/parity-checklist.md
cargo test -p dbcraft-drivers --test servers -- --ignored   # if you touched drivers (docker servers up)
```

`cargo xtask ci` runs fmt/clippy/test/layers in one go.

Then append a terse entry to `log/devlog.md` (what landed, numbers, what's still open). Sessions can end abruptly, so the dev log plus a green tree is how the next agent picks up. Keep the tree building at every step.

## 6. Parallel agents

- Use your own target dir (`CARGO_TARGET_DIR=target/agent-<name>`) to avoid the Cargo build lock, and edit only the files you own. Shared files (`engine/src/commands.rs`, `engine/src/lib.rs`, `ui-egui/src/app.rs`, `menu_catalog.rs`, `state.rs`) get small, surgical edits; re-read before editing.
- Put new commands in a **new module** (`engine/src/<area>_cmds.rs`) rather than growing a shared file.
- Keep every `Cargo.toml` valid at all times: the `crates/*` glob means one broken manifest breaks everyone's build.
- If someone else's in-progress edit breaks the build, wait and retry; don't fix their files.

## 7. Driving DbCraft as an agent

- `dbcraft-cli commands` lists every command and its params.
- `dbcraft-cli cmd <id> '<json>'` runs one; `dbcraft-cli sql <conn-id> "<sql>"` runs SQL.
- `dbcraft-cli mcp` is an MCP server; `.mcp.json` registers it for Claude Code (tools are command ids with `.` → `_`).
- Set `DBCRAFT_WORKSPACE=dev/data/workspace.json` so you never touch the user's real saved connections.
