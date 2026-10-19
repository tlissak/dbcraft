# DbCraft roadmap

Every Navicat Premium 16 feature, ordered by priority, with its status in DbCraft. The feature list comes from the Navicat user manual: see [`docs/navicat-reference.md`](docs/navicat-reference.md) for what each feature should do. This is the single work list: pick from the top, and when you finish an item, flip its status and add the date.

**Status:** ✅ done · 🟡 partial (what's missing is noted) · ⬜ not started · 🚫 not planned · ➡️ moved

_Last updated: 2026-10-19 (owner: Backup, Data Transfer and Structure Synchronization are prioritised tools (P1); the Table Designer moves to P3; tabs are not restored on startup)._

---

## P0: Owner's priorities (look and feel, connections, browse, edit)

| # | Feature | Status | Notes |
|---|---|---|---|
| 0.1 | Navicat-like light theme: white panes, grey toolbar, Windows-blue selection, Segoe UI | ✅ 2026-10-14 | `ui-egui/src/theme.rs`. Fonts load from the system at runtime and are never committed. |
| 0.2 | Icons throughout: big tinted main toolbar (Connection, New Query, Table, View, Function, User, Others, Query, Backup, Automation, Model, Charts), tree, tabs, buttons | ✅ 2026-10-14 | Phosphor icons (MIT), our own tints. Toolbar buttons with no feature behind them are greyed out. |
| 0.3 | Connection icon changes colour when connected (engine colour vs grey); database turns green once loaded | ✅ 2026-10-14 | `icons::connection_glyph`. A lock marks SSH connections. |
| 0.4 | New/Edit Connection dialog with General · Advanced · Databases · SSL · SSH tabs | 🟡 | General and SSH work. Advanced, Databases and SSL are placeholders (see 1.20–1.22). |
| 0.5 | SSH tunnel: password or public-key (with passphrase) auth, host resolved from the SSH server | ✅ 2026-10-14 | `drivers/src/tunnel.rs` (russh). Tested through a docker bastion. Host keys are not verified yet (1.24). |
| 0.6 | Test Connection button, run on a worker thread, with "Connection Successful" or the error | ✅ 2026-10-14 | `connection.test` command. Fast failures (refused or bad password) come back immediately. |
| 0.7 | Clicking a database opens the Objects tab: the table list with Rows, Data Length, Engine, Owner, Created/Modified Date, Collation, Comment | ✅ 2026-10-14 | Shows the columns the engine provides. Search box, Views group, double-click to open. |
| 0.8 | Information pane: General and DDL for the selected object | 🟡 | Missing: Using, Used By, Privileges, Field, Preview. |
| 0.9 | Table viewer with live editing: double-click (or click a selected cell, or F2/Enter) to edit, Enter to keep, Esc to cancel; Set to NULL / Empty String; Add (Insert, Ctrl+N) and Delete (Ctrl+Delete) records; Apply (✓, Ctrl+S) and Discard (✗); **auto-save when moving to another row** (Navicat behaviour) | ✅ 2026-10-14 | All changes go in one transaction. An update or delete that doesn't hit exactly one row rolls everything back (protects tables without a key). |
| 0.10 | Filter wizard bar: enable/disable conditions, field, operator (=, ≠, <, ≤, >, ≥, contains, does not contain, begins/ends with, is (not) null, is (not) empty), value, match all/any, Apply Filter (Ctrl+R), Clear | ✅ 2026-10-14 | Missing: brackets/groups, negation, between/in list, filter profiles (see 1.12). |
| 0.11 | Sort by clicking a column header (asc → desc → none), paging (first/prev/next/last), "Records x–y of N", primary-key icon, generated SQL shown | ✅ 2026-10-14 | |

## P1: Daily-driver essentials

### Infrastructure

| # | Feature | Status | Notes |
|---|---|---|---|
| 1.1 | Async execution: queries, open and refresh off the UI thread, with **Stop** (Ctrl+T) | ✅ 2026-10-19 | Opening, query and console statements, table pages, object lists and DDL run as background tasks over shared connections (`ui-egui/src/tasks.rs`). Stop is available as Ctrl+T, the Stop buttons (query tab, table tab, console, "Connecting…" in the tree) or closing the tab. The server stops the statement too (PostgreSQL `pg_cancel_backend`, MySQL/MariaDB `KILL QUERY`, SQLite progress handler). Pools of 4 let the tree work while a query runs. |
| 1.2 | Control channel (`dbcraft --control`, loopback + token, JSON) and offscreen snapshot, so agents can drive and see the UI | ✅ 2026-10-19 | `ui-egui/src/control.rs`: 127.0.0.1 only, random token in `control.json` next to the workspace (removed on exit), one JSON request per line, handled on the UI thread. Methods: ping, commands, dispatch, action (startup-flag shapes), ui, input (keys, text, clicks via `raw_input_hook`), wait_idle, state (passwords masked), snapshot (PNG of the window). `dbcraft-cli control <method>` is the client. Offscreen rendering of scripted screens: `examples/screenshots.rs`. |
| 1.3 | Passwords in the OS keychain (Windows Credential Manager, macOS Keychain, Secret Service) | ✅ 2026-10-19 | `engine/src/secrets.rs` (keyring 4). The user's workspace moves database and SSH passwords to the keychain on first launch; `workspace.json` then holds none (`"password_storage": "keychain"`). Entries are named `DbCraft` / `<workspace id>/<connection>/<field>` and written only when they change. Workspaces set with `DBCRAFT_WORKSPACE` / `--workspace` (dev, tests, portable) keep their mode. If the keychain fails, passwords stay where they were. |
| 1.4 | Options dialog (Tools › Options): General, Tabs, Editor, Records (page size, auto begin transaction), Display formats | 🟡 2026-10-19 | `tools.options` command (validated, saved in the workspace) and dialog with Appearance (theme, toolbar), Tabs (ask to save on close), Editor (font size, word wrap), Records (limit, records per page) and Display Formats (NULL text, strftime date/time/date-time formats applied in the grids). Missing: auto begin transaction (needs 1.16), pop-ups in new windows, the rest of Navicat's General page. |
| 1.5 | Dark theme; toolbar "Use Big Icons" / "Show Caption" toggles | ✅ 2026-10-19 | `theme.rs` has light and dark palettes (`Tokens::bg()` and so on read the current one); Options › Appearance switches theme and toolbar style live. |
| 1.6 | Restore tabs on startup ("continue where you left off") | 🚫 | Owner's decision: DbCraft starts clean, with no tabs reopened and no connection opened automatically. |

### Prioritised tools (owner's priority: fast, reliable, complete)

| # | Feature | Status | Notes |
|---|---|---|---|
| 1.T0 | **Table Designer** (owner, 2026-10-19: top priority) and table actions: Design Table, New Table, Delete Table; Fields (name, type, length, decimals, not null, key, comment; default, auto increment, collation, unsigned, on update), Indexes, Foreign Keys, Uniques, Checks, Triggers, Options, Comment, SQL Preview | ✅ 2026-10-19 | `engine/src/alter.rs` (CREATE / ALTER per engine; SQLite rebuild with foreign keys off), commands `table.schema/designSql/save/delete/empty/truncate/rename/duplicate`, `ui-egui/src/designer_tab.rs` (pages per engine like Navicat), table context menu in the tree and Objects tab, confirmations. Tested on SQLite, PostgreSQL, MySQL, MariaDB (engine) and through the UI (kittest). Missing: PostgreSQL Excludes/Rules pages, partitions, Find Field, field picker for referenced columns. |
| 1.T1 | **Backup / Restore**: fast (streamed, batched multi-row INSERTs, no full-table buffering), complete (indexes, foreign keys, defaults, auto-increment, views), compressed option, Extract SQL | 🟡 2026-10-14 | Works on all engines (see 2.11). Missing: indexes/FKs/defaults/views in the dump, streaming for big tables, compression. |
| 1.T2 | **Data Transfer**: fast (keyset paging, multi-row INSERT batches, one transaction per batch), complete (indexes, foreign keys, defaults, auto-increment, views), profiles | 🟡 2026-10-19 | Schema model (`TableSchema` from every driver) and portable DDL (`engine/src/ddl.rs`): types (native on the same engine), defaults, auto-increment/identity (sequences reset), primary keys, indexes, foreign keys (after the data, tables in dependency order), comments, views (best effort across engines). Keyset paging, INSERTs capped at 1 MiB, one transaction per batch, reader/writer overlapped, 4 tables in parallel. Missing: checks, triggers, profiles, transfer UI options for the new switches. |
| 1.T3 | **Structure Synchronization**: compare two databases (any engines), show differences per object, generate the deployment script, run it | ⬜ | Moved up from P3 (was 3.2). Needs a schema model (tables, columns, PK, indexes, FKs) read from each driver, a differ and DDL generation per dialect. |

### Query editor

| # | Feature | Status | Notes |
|---|---|---|---|
| 1.7 | Syntax highlighting, line numbers, code folding, brace highlight, word wrap, zoom | ⬜ | Plain text editor today. |
| 1.8 | Run · Run Selected · Run Current Statement (Ctrl+R / Ctrl+Shift+R); Explain | 🟡 | Run executes the whole script. |
| 1.9 | Up to 20 result tabs (one per statement), named with `-- NAME:x`; results below the editor or in a new page | 🟡 2026-10-14 | One Result tab per SELECT (max 20) plus Message, each query tab keeps its own results. Missing: `-- NAME:` tab names, results in a new page. |
| 1.10 | Code completion (schema-aware: databases, tables, columns, keywords, snippets) | ⬜ | |
| 1.11 | Save queries per connection, open/save external `.sql` files, History Log (Ctrl+L), Beautify/Minify SQL, comment toggle (Ctrl+/), find/replace | 🟡 2026-10-14 | **Saved queries done**: Save (Ctrl+S) / Save As, ● modified marker and `*` on the tab, "Save changes?" (Save / Don't Save / Cancel) when closing, Queries node in the tree and Objects › Query list, double-click to open. Missing: external files, history log, beautify, find/replace. |

### Data viewer depth

| # | Feature | Status | Notes |
|---|---|---|---|
| 1.12 | Filter wizard extras: brackets, Toggle Negator, between / in list, Custom Filter, quick filter by cell value, filter profiles | ⬜ | |
| 1.13 | Multi-column Sort dialog; show/hide columns; freeze columns; reorder columns; column width "best fit" | ⬜ | |
| 1.14 | Cell selection by block, Ctrl+C tab-separated copy, paste into cells and rows, Copy As INSERT / UPDATE / TSV, Save Data As | 🟡 2026-10-14 | **Done in result grids and the table viewer**: cell, Shift-block, whole rows (row numbers; Ctrl/Shift), whole columns (header), all (corner / Ctrl+A); Ctrl+C, Copy with Headers, Copy as INSERT; delete selected rows. Missing: paste, Copy As UPDATE, Save Data As. |
| 1.15 | Find/Replace in the grid (Ctrl+F, F3) | ⬜ | |
| 1.16 | Manual transactions: Begin Transaction / Commit / Rollback buttons | ⬜ | The button is there but disabled. Apply already uses one transaction per batch. |
| 1.17 | Typed editors: date/time picker, enum dropdown, set checklist, Text/Hex/Image assistant editors, BLOB load/save, Generate UUID, Raw Mode | ⬜ | BLOB cells are read-only today. |
| 1.18 | Foreign Key Data Selection (Ctrl+Enter on an FK cell opens a picker on the referenced table) | ⬜ | |
| 1.19 | Form View | ⬜ | |

### Connections and navigation

| # | Feature | Status | Notes |
|---|---|---|---|
| 1.20 | SSL tab: MySQL (client key/cert, CA, verify, cipher); PostgreSQL (SSL mode, certs, CRL) | ⬜ | sqlx supports this; it needs wiring. |
| 1.21 | Advanced tab: auto connect, encoding/charset, keepalive, limit sessions, settings location | ⬜ | |
| 1.22 | Databases tab: custom database list, auto open. SQLite: New SQLite 3 / existing file, Attached Databases | ⬜ | SQLite creates the file if it is missing. |
| 1.23 | HTTP tunnel tab (`ntunnel_*.php`) | ⬜ | |
| 1.24 | SSH host-key verification (known_hosts, trust on first use, with a prompt) | ⬜ | |
| 1.25 | Connection colours, Duplicate Connection, connection groups (virtual grouping), Import/Export Connections (`.ncx`), connection profiles | 🟡 | Edit and Delete Connection work. |
| 1.26 | Navigation: type to filter, Show Only Active Objects (F12), Show Hidden Items, PostgreSQL schemas as tree level, Refresh (F5) | 🟡 | Refresh is in the context menu. |
| 1.27 | Objects tab List / Detail / ER Diagram views; View › Choose Columns | 🟡 | Detail view only. |
| 1.28 | Keyboard shortcuts from the manual (`docs/navicat-reference.md` §18): Ctrl+Q, F8, Ctrl+Tab, Ctrl+D, Ctrl+G… | 🟡 | Grid shortcuts are done (Ctrl+S, Ctrl+R, Insert, Ctrl+N, Ctrl+Delete, Esc, arrows, F2). |

## P2: Object management and core tools

| # | Feature | Status | Notes |
|---|---|---|---|
| 2.2 | Table actions: Delete, Empty, Truncate, Rename, Duplicate (structure / structure+data), Open Table (Quick) | 🟡 2026-10-19 | Delete, Empty, Truncate, Rename, Duplicate (structure + data via `table.duplicate {data}`) from the table context menu, with confirmations. Missing: structure-only duplicate in the menu, Open Table (Quick). |
| 2.3 | View designer: Definition, Preview, Explain, View Builder, Beautify | ⬜ | |
| 2.4 | Functions/Procedures: wizard, designer, Execute with Input Parameter dialog, multiple result sets | ⬜ | Listed in the tree today. |
| 2.5 | Triggers, Events (MySQL), Indexes, Sequences, Schemas, Materialized Views (refresh), PostgreSQL Types / Domains / Extensions | ⬜ | Listed in the tree today. |
| 2.6 | New/Edit/Drop Database (and Schema) | ⬜ | |
| 2.7 | Users, Roles, Privilege Manager (per engine) | 🟡 2026-10-14 | **User/Role manager for MySQL, MariaDB, PostgreSQL**: list (User toolbar button), New User / New Role / Edit / Delete (confirm); designer tabs General (name, host, plugin, password, lock / login, connection limit, expiry), Member Of, Server Privileges (MySQL *.*) or Role Attributes (PostgreSQL), Privileges (db.* / db.table, DATABASE / SCHEMA / TABLE), SQL Preview (password masked). Revokes before DROP ROLE. Missing: Privilege Manager (object-first view), SQL Server/Oracle/MongoDB. |
| 2.8 | Import Wizard: CSV, TXT, JSON, XML, Excel; field mapping; import modes; profiles | ⬜ | |
| 2.9 | Export Wizard: CSV, TXT, JSON, XML, SQL, HTML, Excel; options; profiles | 🟡 | CSV of the current result only (`export.resultCsv`). |
| 2.10 | Dump SQL File (structure / structure+data) and Execute SQL File | 🟡 2026-10-14 | Data Transfer → File writes a SQL script in any dialect (MySQL / PostgreSQL / SQLite). Execute SQL File: via Backup › Restore for now. |
| 2.11 | Backup / Restore (built-in format) / Extract SQL | ✅ 2026-10-14 | Optimisation and completeness tracked in 1.T1. Backup toolbar button / Backups tree node: New Backup (comment), list (created, tables, rows, size, comment), Restore (confirm; drops and recreates the backed-up tables), Delete (confirm; only inside the backups folder), Open Folder. SQL scripts named YYYYMMDDhhmmss (same second → `_2`). Runs in the background with a live log. Tested on SQLite, PostgreSQL, MySQL, MariaDB. |
| 2.12 | Maintain menu: Analyze, Check, Optimize, Repair, Vacuum, Reindex, Get Rows Count | ⬜ | |
| 2.13 | Find in Database/Schema (Ctrl+Shift+F): data or structure, contains / whole word / prefix / regex | ⬜ | |
| 2.14 | Server Monitor: process list (End Process), variables, status | ⬜ | |
| 2.15 | Console (F6), Favorites (Ctrl+1..9), Code Snippets | 🟡 2026-10-14 | **Console done** (Tools › Console, connection/database context menu): mysql/psql-style prompt, multi-line statements until `;`, boxed result tables, `use db` / `\c db`, `clear`, Up/Down history. Missing: F6 shortcut, Favorites, Code Snippets. |
| 2.16 | ER Diagram view in the Objects tab (auto-generated from foreign keys) | ⬜ | |
| 2.17 | Print Structure (PDF/HTML report) | ⬜ | |

## P3: Advanced Navicat tools

| # | Feature | Status | Notes |
|---|---|---|---|
| 3.1 | Data Transfer (cross-engine, objects, options, SQL file target, profiles) | ✅ 2026-10-14 | Optimisation and completeness tracked in 1.T2. Tools › Data Transfer: source/target connection + database or SQL file (choose dialect), table checklist, options (create tables, drop target first, create records, continue on error, batch size), background run with live log, Save as Batch Job. Carries columns (engine-neutral types), NOT NULL, primary keys; converts booleans, binary, datetimes per engine. Tested PG→MySQL, MySQL→PG, MariaDB→SQLite, SQLite→MariaDB, PG→SQLite. Missing: indexes, foreign keys, defaults, auto-increment, views/routines, profiles. |
| 3.2 | Structure Synchronization | ➡️ | Moved to 1.T3. |
| 3.10 | Table Designer | ➡️ | Back to the top as 1.T0 (owner, 2026-10-19). |
| 3.3 | Data Synchronization (key mapping, compare & preview, deploy) | ⬜ | |
| 3.4 | Data Generation (generators: numbers, dates, names, emails, FK, regex…) | ⬜ | |
| 3.5 | Query Builder (visual SELECT: joins, criteria, layers) and Query Parameters `[$name]` | ⬜ | |
| 3.6 | Model: physical, logical and conceptual ER design; reverse engineering; synchronize to database; export SQL; export PDF/PNG/SVG | ⬜ | |
| 3.7 | Charts and Dashboards | ⬜ | |
| 3.8 | Automation: batch jobs, scheduling (Windows Task Scheduler), email notifications, `-batchjob` CLI | 🟡 2026-10-14 | **Batch jobs done**: steps Backup / Query / Data Transfer (reorder, remove), continue on error, Start (background, live log, last run), Set/Delete Task Schedule (once / hourly / daily / weekly) registered with Windows Task Scheduler from XML: runs on battery, catches up missed starts, logs to `data/logs/<job>.log`; `dbcraft-cli batchjob <id|name>`. Verified with a real task. Missing: email notification, other job types (import/export). |
| 3.9 | PL/pgSQL debugger (pldbgapi) | ⬜ | |

## P4: More engines and integrations

| # | Feature | Status | Notes |
|---|---|---|---|
| 4.1 | SQL Server driver (tiberius) | ⬜ | A docker service (profile `later`) is ready in `dev/docker-compose.yml`. |
| 4.2 | MongoDB: driver, Grid/Tree/JSON views, document editor, Find/Aggregate builders, GridFS, schema analysis | ⬜ | |
| 4.3 | Redis: key browser | ⬜ | |
| 4.4 | Oracle (needs a client library; research a pure-Rust option first) | ⬜ | |
| 4.5 | Cloud presets: AWS RDS/Aurora/Redshift, Azure, Google Cloud SQL, Alibaba, Tencent, Huawei, MongoDB Atlas | ⬜ | Mostly connection-dialog presets plus SSL. |
| 4.6 | Packaging: MSI installer, portable zip, macOS/Linux builds, auto-update | ⬜ | |
| 4.7 | Navicat Cloud / On-Prem collaboration, registration and activation | 🚫 | Proprietary services; out of scope. |

---

## Verification status (what's proven, how)

- Drivers: SQLite always; PostgreSQL 17, MySQL 8.4, MariaDB 11 and SSH-tunnelled PostgreSQL/MySQL against `dev/docker-compose.yml` (`cargo test -p dbcraft-drivers --test servers -- --ignored`).
- Engine: `panic_hunt` (every command fuzzed), `table_data` (filters, sort, paging, atomic edits, injection attempts).
- UI, driven headlessly with egui_kittest (`crates/ui-egui/tests/`): edit + Apply, auto-save on row change, Insert record, Discard, filter bar, connection dialog SSH tab, background Test Connection; saved query + close prompt, row selection + Ctrl+C, Console, Data Transfer, Backup, Automation; user manager against PostgreSQL (`ui_users`, `--ignored`).
- Engine/servers: `tools.rs` (saved queries, transfer, backup/restore, automation), `tools_servers.rs` (cross-engine transfer, server backups), driver `user_lifecycle` (create/grant/login/revoke/drop on PG, MySQL, MariaDB).
- Fixed this pass: hover no longer grows tree items (no border shift); labels aren't text-selectable (no I-beam, rows clickable); the screen repaints right after an action; backups in the same second no longer overwrite each other.
