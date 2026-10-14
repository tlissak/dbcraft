# Navicat Premium 16: functional reference

Summary of the public Navicat Premium 16 user manual (the PDF that ships with the app, 303 pages), written for the clean-room clone: **behaviour only, in our own words**. Use it to check what a feature should do. Never copy Navicat code, resources, icons or help text.

What the manual does **not** document (do not guess and present it as Navicat behaviour): per-engine Objects tab columns, Table Designer tab list and fields, the Index/Sequence/Trigger/Schema designers, Data Profiling/BI, an exact import/export format table, and icon-only button names.

Editions: "Non-Essentials" marks features missing from Essentials. Model and Charts exist only in Premium/Enterprise.
## 1. Supported servers and cloud
- **On-premises:** MySQL 3.23+ (also Drizzle, OurDelta, Percona), PostgreSQL 7.3+, Oracle 8i+, SQLite 2 and 3, SQL Server 2000+, MariaDB 5.1+, MongoDB 3.0–5.0.
- **AWS:** Redshift, Aurora MySQL, Aurora PostgreSQL, RDS (MySQL, PostgreSQL, Oracle, SQL Server, MariaDB), DocumentDB.
- **Google Cloud SQL:** MySQL, PostgreSQL, SQL Server.
- **Oracle Cloud:** Database Cloud Service, MySQL Cloud Service.
- **Azure:** SQL Database, Database for MySQL, PostgreSQL and MariaDB.
- **MongoDB Atlas.**
- **Alibaba ApsaraDB:** RDS MySQL, PostgreSQL and SQL Server; MongoDB; OceanBase.
- **Tencent TencentDB:** MySQL, PostgreSQL, SQL Server, MariaDB, MongoDB.
- **Huawei:** RDS MySQL, PostgreSQL and SQL Server; Document Database Service.
- **OceanBase:** Community and Enterprise editions.
- **Registration:** perpetual 16-digit key with online or manual activation (Request Code / Activation Code), or a subscription through Navicat ID with hourly reload and a Reload Plan button.
- **Migration:** File -> Export Connections / Import Connections (`.ncx`). Help -> Check For Updates.

## 2. Main window and UI
- **Main Toolbar:** opens object types (Connection, User, Table, View, Function, Others, Query, Backup, Automation, Model, Charts…). Right-click it to toggle "Use Big Icons" and "Show Caption".
- **Navigation Pane:** tree of connections, databases and objects with context menus. Double-click to connect or open.
  - Type while the tree has focus to filter it.
  - View -> Navigation Pane -> Show Only Active Objects / Flatten Connection / Show Navigation Pane.
  - When signed in to the cloud, local connections sit under "My Connections".
  - Option "Show objects under schema in navigation pane".
- **Tab Bar:** CTRL+TAB switches tabs. Pop-ups open in a tab or in a new window, depending on Options.
- **Object Pane / Objects tab:** views are List (names only), Detail (property columns chosen via View -> Choose Columns) and ER Diagram (Non-Essentials).
  - View -> Flatten Object List.
  - Search box filters the list.
  - Double-click in List or Detail opens the Table Viewer. Double-click in ER view opens the Table Designer.
- **ER Diagram view** (MySQL, Oracle, PostgreSQL, SQLite, SQL Server, MariaDB; tables only):
  - Auto-generated diagram. Add an FK by dragging a field from the child to the parent table.
  - Right-click a relation for Design Foreign Key or Delete Foreign Key. SHIFT+click adds or removes a vertex.
  - Overview slider for zoom.
  - Bottom toolbar: Refresh (plus Regenerate ER Diagram with auto layout), Move Diagram (hand mode, or hold SPACE), New Relation, Color, Paper Size.
- **Information Pane** buttons:
  - General
  - Preview (query SQL)
  - DDL (CTRL+F search)
  - Using (dependencies)
  - Objects (tablespace contents)
  - Member Of
  - Used By
  - Members
  - Code Snippet
  - Identifiers (tables, views and fields of the current DB)
  - Field (selected field in Table Viewer)
  - Privileges
  - Project (members and activity)
  - Type Color (MongoDB)
- **Status Bar.**
- **View -> Show Hidden Items** shows system databases and tables.
- **Object designers** have a SQL Preview or Script Preview tab. A bottom drop-down shows the SQL for Save versus Save As.
- **Shortcuts:**
  - Create Open Table/View/Materialized View/Collection Shortcut: a desktop shortcut that opens the object directly.
  - Open Table (Quick): defers loading BLOBs. It only appears if you hold SHIFT while right-clicking. Not available for SQLite or MongoDB.
  - Empty Table keeps the auto-increment value; Truncate Table resets it. SQLite and MongoDB have Empty only.

## 3. Collaboration (Navicat Cloud / On-Prem Server)
- **Synced items:** connections (without passwords), queries, snippets, models, workspaces and virtual groups.
- **Manage Cloud (File -> Manage Cloud):**
  - Create Navicat ID, Sign In (with two-step verification), Sign Out.
  - View Usage, change avatar, Manage Account (opens the web portal).
- **On-Prem Server:**
  - New On-Prem Server dialog fields: Host, Port, Verify Server Certificate, Enable Push Synchronization. Alternative: New On-Prem Server with URI.
  - Edit, Rename and Remove (requires sign-out first). Multiple servers allowed.
- **Other cloud actions:** Push Synchronization toggle, Clear Cache and Local Copies, Open Containing Folder.
- **Projects:** New, Rename, Delete (owner only, project must be empty), Quit.
  - Manage Members / Add Members by Navicat ID or user checkbox.
  - Member rights: Can Manage & Edit / Can Edit / Can View.
- **Moving items into projects:** Move/Copy Connection To project (optionally with its queries and virtual groups). Move/Copy Snippet To project.

## 4. Connections
- **Entry points:** Connection button -> server type. Right-click -> Edit Connection.
  - PAM and GSSAPI authentication are supported.
  - MySQL/MariaDB: right-click -> Flush (needs the RELOAD privilege).
  - Azure SQL: right-click -> SQL Azure Firewall Rules (IP ranges).
- **Dialog tabs:** **General, Advanced, Databases** (SQLite: **Attached Databases**), **SSL, SSH, HTTP** (with sub-tabs Authentication and Proxy), **Compatibility** (MySQL, when enabled).

### General tab, RDBMS
The manual lists these fields in alphabetical order, not UI order, and notes they depend on server type:

| Field | Applies to / values |
|---|---|
| Add To | My Connections or a project (when signed in) |
| Authentication | SQL Server: SQL Server Authentication / Windows Authentication |
| Connection Name | All |
| Connection Type | Oracle: Basic (Host, Port, Service Name / SID radio) or TNS (Net Service Name) |
| Database File | SQLite. Absolute path on the web server if HTTP tunnel is used |
| Endpoint | AWS |
| Host | All network engines |
| Initial Database | All network engines |
| Password | All network engines |
| Port | All network engines |
| Type | SQLite: Existing Database File / New SQLite 3 / New SQLite 2 |
| User Name | All network engines |

### General tab, MongoDB
- Connection Name, Add To.
- Connection: Standalone / Shard Cluster / Replica Set.
- SRV Record checkbox.
- Host (host, IP or UNIX socket), Port.
- Member list (replica set or shard instances), Read Preference, Replica Set name.
- Authentication: None / Password (plus Authentication Database) / LDAP / Kerberos (Service Name, Principal) / X.509.
- URI button: paste a MongoDB URI to auto-fill the General, SSL and SSH tabs.

### Advanced tab
Alphabetical; availability depends on server type:
- Auto connect
- Client Character Set
- Client Driver Version (Default or Legacy)
- Compatibility (enables the Compatibility tab)
- Encoding
- Encrypted (SQLite, plus password)
- Keepalive interval (sec)
- Limit connection sessions
- Native Client Driver (SQL Server)
- OS authentication
- Retryable Reads / Retryable Writes (Mongo)
- Role (Oracle: Default / SYSOPER / SYSDBA)
- Settings Location (per-connection folder for queries, backups, export and import profiles, ER files, etc.)
- Use compression
- Use encryption (SQL Server)
- Use named pipe, socket

### Databases tab
MySQL, Oracle, PostgreSQL, SQL Server, MariaDB:
- "Use custom database list", then check databases in the Database column. "Auto Open" checkbox.
- Add Database to List (hidden DB) / Remove Database from List.

### Attached Databases tab
SQLite:
- Attach Database dialog: Database File, Database Name, Encrypted plus Password.
- Detach Database.

### SSL tab
MySQL, PostgreSQL 8.4+, MariaDB, MongoDB:
- **MySQL/MariaDB:** "Use authentication", Client Key, Client Certificate, CA Certificate, "Verify server certificate against CA", Specified Cipher.
- **PostgreSQL:** SSL Mode (require / verify-ca / verify-full). With "Use authentication": Client Key, Client Certificate, Root Certificate, Certificate Revocation List.
- **MongoDB:** "Use authentication", Client Key, Client Key Password, "Allow any server SSL certificates", CA Certificate, Certificate Revocation List, "Allow invalid host names".

### SSH tab
MySQL, Oracle, PostgreSQL, SQL Server, MariaDB, MongoDB. Field labels as documented:
- **Host** (the SSH server; the General-tab host is resolved relative to it)
- **Port** (default 22)
- **User Name** (the SSH user, not the DB user)
- **Authentication Method:** **Password** -> **Password**; **Public Key** -> **Private Key**, **Passphrase**

The manual does not name an enable checkbox for SSH (for HTTP it says "Use HTTP tunnel"). SSH and HTTP tunnels are mutually exclusive.

### HTTP tab
MySQL, PostgreSQL, SQLite, MariaDB:
- "Use HTTP tunnel", Tunnel URL (`ntunnel_mysql.php` / `ntunnel_pgsql.php` / `ntunnel_sqlite.php`).
- "Encode outgoing query with base64".
- Authentication and Proxy sub-tabs.

### Compatibility tab
MySQL only:
- Force lower_case_table_names as
- Force sql_mode as
- Force NDB cluster support as
- Force database listing method as (SHOW DATABASES or INFORMATION_SCHEMA)
- Force view listing method as (SHOW FULL TABLES or INFORMATION_SCHEMA)

### Connection Profiles
- New Profile / Duplicate Profile inside the connection window. Choose the default active profile there.
- Right-click a connection -> Switch Connection Profile.

### Connection Colorings
- Right-click -> Color. The colour shows in the Navigation pane and in the menu bar or tab of object windows.

## 5. Server objects per engine
- **MySQL/MariaDB:**
  - Databases: New/Edit Database (no rename).
  - Tables: Designer and Viewer. Transactions only on InnoDB.
  - Views: Designer and Viewer (transactions only on updatable views).
  - Procedures/Functions:
    - Function Wizard: choose Procedure or Function; parameters with Mode, Name, Type; Return Type with Length, Decimals, Character set, Enum.
    - Designer has a Definition tab, Execute, and an Input Parameter dialog with Raw Mode. Up to 20 result sets.
  - Tablespaces: Designer covers engine and data file.
  - Events: Event Designer with a Definition tab.
- **Oracle:**
  - Schemas (= users). Identifiers are always quoted.
  - Tables: Normal / External / Index Organized.
  - Views, Materialized Views (Refresh Materialized View).
  - Procedures/Functions: Wizard has Name, Type, Mode, Default Value and Return Type. Output appears in a DBMS Output tab. Code Outline pane (Non-Essentials). Debug via Save As Debug, then Debug.
  - Packages: Package and Package Body designers; Design Package Body / Specification.
  - Recycle Bin: Flashback Table, Purge Object, Purge Recycle Bin, Purge DBA Recycle Bin.
  - Others: Database Link, Index, Java, Materialized View Log, Sequence, Synonym, Trigger, Type, XML Schema, Directory, Public Database Link, Public Synonym, Tablespace.
- **PostgreSQL:**
  - Databases and Schemas: New/Edit Database, New/Edit Schema.
  - Tables: Normal / Foreign / Partitioned.
  - Views. Materialized Views: Refresh Materialized View With -> Data / No Data.
  - Procedures (PG11+) and Functions: Wizard has Mode, Type Schema, Type, Name, Default Value, Schema and Return Type. Debug needs the `pldbgapi` extension; there is a right-click "Install pldbgapi Extension" (PG 9.1+).
  - Types: Base / Composite / Enum / Range.
  - Foreign Servers: designer, plus "Install postgres_fdw Extension".
  - Others: Aggregate, Conversion, Domain, Index, Operator, Operator Class, Sequence, Trigger, Tablespace, Cast, Language.
  - Manage Extensions: dual list Available / Installed.
- **SQL Server:**
  - Databases and Schemas. Tables. Views.
  - Procedures/Functions: Wizard has Name, Type Schema, Type, Default Value, Output, Read Only, Function Type, Return Type and advanced options. Code Outline pane.
  - Others: Index, Synonym, Trigger, Backup Device, Linked Server, Server Trigger, Assembly, Database Trigger, Partition Function, Partition Scheme.
- **SQLite:**
  - The General-tab file is the "main" database.
  - Attach Database / Detach Database.
  - Encrypt Database / Decrypt Database.
  - View Master Table (`sqlite_master`).
  - Tables, Views, Index, Trigger.
- **MongoDB:**
  - Databases: New Database.
  - Collections: Collection Designer covers properties, indexes, validation and storage engine. Empty Collection.
  - Views: Pipeline tab where you add, insert or delete stages; picking an Operator generates an Expression template. Preview and Explain buttons.
  - Functions: stored JS; Execute Function dialog.
  - Indexes.
  - MapReduce: Run, Stop, Preview (Input / Mapper / Reducer / Finalizer), Import Function. Output inline or to a collection. Can be scheduled.
  - GridFS:
    - Buckets: New Bucket, Open Bucket.
    - Bucket Viewer: Open File, Delete File, Upload File, Download File, Filter, Preview (images under 1 MB), Progress.
    - File actions: Modify File Name / Content Type / Alias / Metadata.
    - Progress pane: pause, resume, stop, Clear All Finished.
- **View Designer** (all RDBMS): Definition tab, File -> Import SQL. Buttons Preview, Explain, View Builder (the SQL Builder) and Beautify SQL. Results either Below Editor or in a New Page.

### Maintain menu (right-click -> Maintain; results open in a pop-up)
- **MySQL/MariaDB:**
  - Tables: Analyze, Check, Optimize, Repair, Get Rows Count.
  - Undo tablespace: Set Active / Set Inactive.
- **Oracle:**
  - Table: Enable/Disable Table Lock, Enable/Disable Row Movement, Shrink Space, Move, Collect Statistics, Validate Structure.
  - View: Compile.
  - Procedure/Function, Package, Trigger, Type: Compile and Compile for Debug. Trigger also has Enable/Disable.
  - Index: Rebuild, Make Unusable, Coalesce, Compute Statistics, Monitoring Usage, No Monitoring Usage.
  - Java: Compile or Resolve, Set AuthID Current User / Definer.
  - Materialized View: Row Movement, Shrink, Compile, Force Refresh.
  - Materialized View Log: Row Movement, Shrink Space.
  - XML Schema: Compile, Purge.
  - Tablespace: Read Only, Read Write, Online, Offline, Coalesce, Shrink Space.
  - User: Expire Password, Lock Account, Unlock Account.
- **PostgreSQL:**
  - Database: Allow / Disallow connections, Analyze, Vacuum, Reindex.
  - Table and Materialized View: Analyze, Vacuum, Reindex.
- **SQL Server:**
  - Database: Read Write, Read Only, Online, Offline, Emergency, Multi User, Single User, Restricted User.
  - Assembly: Visible / Invisible.
  - Index: Rebuild, Reorganize, Disable.
  - Triggers: Enable / Disable.
- **SQLite:**
  - Database: Analyze, Vacuum (main database only), Reindex.
  - Table: Analyze, Reindex, Get Rows Count.
  - Index: Reindex.
- **MongoDB:**
  - Database: Repair.
  - Collection: Compact, Validate, Reindex.

## 6. Data Viewer (Table, View and Query results)

### RDBMS toolbar
- Begin Transaction, Commit, Rollback. If "Auto begin transaction" is set in Options, a transaction starts automatically.
- Text (assistant editors)
- Filter
- Sort (custom multi-column)
- Columns (show/hide)
- Import
- Export
- Data Generation
- Create Chart (new Charts workspace from the table)

### Navigation bar (bottom)
- Add Record, Delete Records, **Apply Changes**, **Discard Changes**, Refresh, Stop (loading).
- First Page, Previous Page, Next Page, Last Page.
- First Record, Previous Record, Next Record, Last Record.
- Limit Record Setting: a "Limit records [N] records per page" checkbox. It is per-object; the global value is in Options.
- Grid View / Form View toggles.
- Status text "Record a of b in page c".

### How editing and applying works
- **Grid View add:** type into the first blank row, or press Add / CTRL+N / INSERT.
- **Row indicator:** the record selector icon changes from "current" to "editing".
- **Commit a row:** move to another row (implicit save), click Apply, or press CTRL+S. Closing the table also saves.
- **Discard:** the Discard button or ESC removes the edits to the current record.
- **Delete:** right-click -> Delete Record, the Delete button, or CTRL+DELETE.
- **Multi-cell edit:** select a block and type; it applies to all compatible cells.
- **Form View** (Non-Essentials): one record at a time. Add, then type, then Apply. It only supports Show/Hide Columns formatting.
- **Undo:** paste cannot be undone unless a transaction is active.

### Special cell editing
- Set to Empty String / Set to NULL.
- View -> Display -> Show Image In Grid (MySQL, Oracle, PostgreSQL, SQL Server, MariaDB).
- Date/Time/DateTime picker via the cell button or CTRL+ENTER (same engines).
- Enum drop-down (MySQL, PostgreSQL, MariaDB).
- Set checklist editor (MySQL, MariaDB).
- Preview BFile (Oracle).
- Generate UUID (PostgreSQL, SQL Server).
- **Foreign Key Data Selection** (Non-Essentials): CTRL+ENTER on an FK cell opens a picker of the referenced table.
  - Shows 1000 rows by default, with a show-all button and refresh (F5).
  - Column chooser pane, Sort Ascending / Descending, Filter box (press ENTER).
  - Double-click a row to select it.
- **Raw Mode** (View -> Display -> Raw Mode): input is sent unquoted, so server functions work (MySQL, PostgreSQL, SQLite, SQL Server, MariaDB).

### Copy and paste
- Copy is tab-delimited (CTRL+C). Selection: CTRL+A, SHIFT+arrows, CTRL-click rows, SHIFT-click a range, block select.
- Paste (CTRL+V) replaces the selected cells or appends rows.
- **Copy As:** Insert Statement, Update Statement, Tab Separated Values (Field Name only / Data only / Field Name and Data).
- **Save Data As** (single cell to a file).

### Sort, find and replace
- **Sort:** the column header drop-down offers Sort Ascending, Sort Descending, Remove Sort. The Sort toolbar button builds a multi-column sort. Server order is not changed.
- **Find bar** (CTRL+F): Find Data or Find Field. Next / F3.
- **Replace:** check Replace, then Replace or Replace All. After Replace All you choose Apply or Cancel.
- **Options:** Highlight All, Incremental Search, Match Case.

### Filtering
1. Right-click a cell -> Filter -> "Field xxx Value" (quick filter by the cell value).
2. Right-click -> Filter -> **Custom Filter** dialog: `_` matches one character, `%` matches any run of characters.
3. Right-click -> Filter -> Filter, or the toolbar **Filter** button, opens the **Filter Wizard** at the top of the grid.
   - Each condition has a checkbox to enable or disable it.
   - Steps: click + (or + with brackets), then field, then operator (or [Custom]), then `<?>` value, then and/or.
   - Click the apply icon (or CTRL+R "Apply Filter / Apply Sort") to run it.
   - Right-click options: Group with Bracket, Delete Bracket, Delete Bracket and Conditions, Toggle Negator (RDBMS only).
   - Profiles: Load Profile, Delete Profile, Save Profile, Save Profile As.
   - Operators:
     - Comparison: = != < <= > >=
     - Text: contains, does not contain (case-insensitive variants on PostgreSQL), begin with, does not begin with, end with, does not end with
     - Null and empty: is null, is not null, is empty, is not empty
     - Ranges and lists: is between, is not between, is in list, is not in list
     - MongoDB only: exists, does not exist, is field type, is not field type
   - The value editor depends on the field type. MongoDB lets you switch the editor type.

### Formatting
- Drag columns to reorder.
- Display -> Freeze Selected Column / Unfreeze Columns. Frozen columns move to the left and become read-only.
- Set Column Width (drag, double-click for best fit, or a dialog).
- Set Row Height.
- Show/Hide Columns.
- Show/Hide ROWID (Oracle, SQLite).
- Field comments appear as header tooltips (MySQL, Oracle, PostgreSQL, SQL Server, MariaDB).

### Assistant editors (toolbar Text / Hex / Image / Web)
- **Text:** plain text with a Language (syntax highlight) setting. Changes are applied with the navigation-bar Apply button.
- **Hex:** INSERT key toggles Insert / Overwrite.
- **Image:** Load, Save to disk, Clear.
- **Web:** renders HTML.
- Oracle BFile is read-only. Not available in MongoDB JSON View.

### MongoDB Data Viewer
- Modes: Grid / Tree / JSON View.
- Toolbar adds Expand All, Collapse All, Type Color and Analyze (Schema Analysis). The navigation bar uses "Document" wording.
- **Add or edit documents:**
  - Grid: inline, or right-click -> Add Document / Edit Document opens a pop-up with **Validate** plus Add or Update.
  - Tree: click a name, value or type to edit; + adds a field or item; Delete Value.
  - JSON: everything goes through the pop-up.
- **Cell options:** Value Type (change BSON type), DateTime editor.
- **Grid layout:**
  - Expand/collapse embedded documents inline.
  - Expand arrays into a sub-grid, with a breadcrumb back to the collection.
  - Type Color pane with "Enable Coloring".
- **Custom Filter** takes a JSON query or projection. Find Data, Find Value or Find Field.

## 7. Query
- **Query objects:**
  - New Query works without opening a connection.
  - Open with External Editor. Open Containing Folder.
  - File -> Open External File -> Query (with encoding choice), Save to Navicat, Save As External File.
- **Query Designer buttons:**
  - RDBMS: Query Builder, Beautify SQL, Create Chart.
  - MongoDB: Find Builder, Aggregate Builder, Beautify Script, Type Color.
  - Common: Code Snippet, Text, Export Result, Run (Run / Run Current Statement / Run Selected; Mongo has no Current Statement), Stop, Explain (Explain / Explain Selected).
- **SQL Editor:**
  - Identifiers pane: drag or double-click to insert.
  - Format menu: Indent/Outdent, Comment/Uncomment (CTRL+/), Convert case (upper/lower), Beautify SQL, Minify SQL.
  - Beautify SQL With dialog: Single Line Braces Word/Symbol Limit, Upper case keywords, Beautify.
  - Code Completion:
    - Triggers: `.`, any character, or ESC. TAB or ENTER inserts; resizable list.
    - Edit -> Code Completion -> Update Code Completion Info.
    - Snippets appear in the list.
  - Clipboard Stack: last 10 copies, CTRL+SHIFT+V cycles through them.
  - Code folding, brace highlight.
  - Find/Replace with Highlight All, Incremental, Match Case, Regular Expression, Whole Word.
  - Copy with quotes (choose a format).
  - Word Wrap.
  - Zoom: CTRL+= / CTRL+- / CTRL+0, per editor.
- **SQL Builder** (SELECT only):
  - Panes: Object pane, Diagram, Criteria tabs, read-only SQL pane.
  - Criteria tabs: SELECT (DISTINCT, aliases, aggregates, order), FROM (aliases, joins), WHERE, GROUP BY, HAVING, ORDER BY, LIMIT (Offset / Limit; MySQL, PostgreSQL, SQLite, MariaDB).
  - Joins:
    - Auto-joins follow FKs. Drag field to field for a manual join. Default is INNER JOIN; join type can be changed or customised.
    - Table Relation / Field Relation views.
    - Edit Join, Clear and Convert to USING / ON.
  - Conditions: Toggle Negator, Insert, Insert Custom, Insert Bracket, Group/Ungroup.
  - Expressions and subqueries, built in layers. "Show Current Layer Only".
  - Zoom.
- **Query Parameters:** written as `[$name]`. An Input Parameter dialog appears at run time, with Raw Mode.
- **Results:**
  - Target connection, DB and schema chosen from toolbar drop-downs.
  - Up to 20 result tabs (Mongo returns the last result only), shown in Grid or Form view.
  - Results Below Editor or in a New Page.
  - Name a result tab with `-- NAME:tab_name` or `/*NAME:tab_name*/`.
  - MySQL/MariaDB: View -> Show Profile and Status adds Profile and Status tabs.
- **Oracle query debug:** the Debug button.
- **MongoDB:**
  - Script Editor offers the same editor features (completion covers DBs, collections, fields and shell methods).
  - Find Builder: Collection/View, then Filter (Filter Wizard), Projection, Sort, Limit (limit and skip).
  - Aggregate Builder: Pipeline stages with Operator and Expression templates.
- **Code Snippets** (Non-Essentials):
  - Library pane with Label filter and Search. Options "Show Snippets For Other Database Type" and "Show Preset Snippets".
  - Built-in snippets are read-only.
  - Insert by completion or drag. Placeholders are navigated with TAB.
  - Create Snippet (from the selection, or New): name, Database Type, Label, Remarks, Code, add/remove placeholder.
  - Stored as `.nsnippet`.
- **History Log** (CTRL+L, Tools -> History Log):
  - Lists all executed statements. Show Error Only, Pause.
  - View options: Show Date, Show Time, Show Server Name, Show Session ID, Show Connection Type, Show Execute Time.

## 8. Debugger (Non-Essentials)
- **Oracle PL/SQL:** functions, procedures, packages and queries. **PostgreSQL PL/pgSQL:** functions and procedures.
- **Controls:** Run (F9), Step Over (F10), Step In (F11), Step Out (SHIFT+F11), Step End (Oracle), Stop.
- **Panes:**
  - Code: breakpoints in the gutter; Add to Watch List.
  - Call Stack: Goto Function.
  - Breakpoints: enable/disable, Remove Breakpoint, Remove All Breakpoints.
  - Local Variables: values editable.
  - Parameters.
  - Watch List (Oracle): Add Variable, Remove Variable, Remove All Variables.
  - Log.

## 9. Model / ER (Premium and Enterprise)
- **Engines and model types:** MySQL, Oracle, PostgreSQL, SQLite, SQL Server, MariaDB. Physical, Logical and Conceptual models. Files `.ndm2` / `.ndml2` / `.ndmc2`; open or save as external files.
- **Window:**
  - Multiple diagrams per model.
  - Explorer pane (Model tab, Diagram tab).
  - History pane (click an entry to revert).
  - Properties pane (Model / Diagram / Object tabs; many style properties: colours, fonts, cardinality, show comments and names, page width and height…).
  - Overview pane, canvas.
- **Physical models:**
  - Databases/schemas: a "Default" is created automatically; New, Delete, Rename, Set as Default.
  - Tables: Design Table, Add Related Objects, Add Field, cut/copy/paste, select all, Delete from Diagram or from Diagram and Model, Rename, Color, Size to Fit, front/back.
  - Views, and Foreign Keys (drag child to parent; Design Relation, Cardinality on each end, vertices, Go to Source/Target).
- **Logical models:** entities, attributes and relations. **Conceptual models:** entities and relations.
- **Canvas:**
  - Show Grid, Snap To Grid, Keep Orthogonal Layout.
  - Diagram Notation: Default, Simple, IE (Crow's Foot), IDEF1X, UML, Classic. Black and White, Show Database/Schema Names, Show View Relationships.
  - Diagram Dimensions, Align (Left / Center / Right / Top / Middle / Bottom), Distribute (Horizontal / Vertical), Page Setup.
  - Auto Layout with Auto Diagram Dimension, Auto Size Tables To Fit, Quality, Object Distance.
- **Annotation objects:**
  - Labels and Notes.
  - Images (BMP, JPG, PNG).
  - Shapes: line, arrow, rectangle, ellipse, user, database, cloud, trigger, server, desktop, mobile.
  - Layers.
- **Reverse engineering:** File -> Import from Database, or right-click Reverse Database/Schema/Tables/Views to Model.
- **Synchronize to Database:**
  - Compare options: auto increment, charset, checks, collation, definers, excludes, FKs, identity last value, indexes, owners, partitions, PKs, rules, storage, table options, tables, triggers, uniques, views. Drop with CASCADE. Identifier Case Sensitivity.
  - Result tree grouped by object type or by operation (modify / create / drop / identical). DDL Comparison and Deployment Script tabs.
  - Deployment: Continue on error, Edit Script (reorder), Copy Script to Clipboard, Open Script in Query Editor, Start.
- **Export SQL:**
  - File and Objects selection.
  - Include options: auto increment, charset, checks, collation, Drop SQL, Drop with CASCADE, excludes, FKs, indexes, PKs, rules, schema, triggers, uniques.
  - Server Version.
- **Other:**
  - Convert Model To (other DB type or version, or other model type).
  - Print, Print Preview. Export To PDF / PNG / SVG / JPG.
  - Inline field typing with type prediction by name (`*` marks the PK, `name:type`). SHIFT-drag reorders or removes fields. SHIFT+DELETE deletes from the model.

## 10. Data migration tools

### Import Wizard
- **Source formats:** CSV, TXT, XML, JSON, DBF/dBase, Excel, Access, Paradox, ODBC. Essentials edition: text formats only.
- **Entry:** drag a file onto the Objects tab or onto a DB. If a table is highlighted the data goes into it; otherwise a new table is created.
- **Steps:**
  1. Format.
  2. Source: Add File(s) / Add URL, Encoding. Excel sheets appear as tables. Access has Security options (System DB File, DB Password, Logon Name and Password). ODBC uses Add Data Source; Access and ODBC also offer Add/Modify/Delete Query.
  3. Delimiter:
     - TXT/CSV: Record Delimiter, Delimited or Fixed Width (click to add break lines), Field Delimiter, Text Qualifier.
     - XML/JSON: row tag, "Consider tag attributes as table field".
  4. Options: Field Name Row, First Data Row, Last Data Row, Date Order, Date Delimiter, Time Delimiter, Decimal Symbol, Date Time Order, Binary Data Encoding (Base64 / none).
  5. Target table: new or existing. Typing a new name checks "New Table".
  6. Field structure and mapping: guessed types; right-click Smart Match All / Direct Match All / Unmatch All. ODBC has Condition Query (WHERE).
  7. Import Mode. Some modes need a Primary Key.
     - Advanced: Run multiple queries in each execution, Use extended insert statements, Use empty string as NULL, Ignore foreign key constraint, Continue on error, Include Unique/Index/FK (Access, ODBC), Create Auto Increment Fields, Import Deleted Records (dBase).
  8. Start, Log, Save profile.

### Export Wizard
- **Sources:** tables, collections, views, query results (the query must be saved first).
- **Formats:** TXT, CSV, HTML, XML, SQL, JSON, Excel, `.mdb` (not in the 64-bit build); others implied. Essentials edition: text formats only.
- **Steps:**
  1. Format.
  2. Saving path per object: Select All / Unselect All, "Export selected to same file" (Excel: one sheet per object), "Export selected to same folder". Advanced: Encoding, "Add timestamp" (format).
  3. Columns: "All Fields" checkbox (skipped for query results).
  4. Options: Include column titles, Blank if zero, Append, Continue on error, "Use Attributes Format" (XML), "Export as legacy format (with RECORDS key)" (JSON), Record/Field Delimiter, Text Qualifier, Zero padding date, Date Order, Date Delimiter, Time Delimiter, Decimal Symbol, Binary Data Encoding.
  5. Start, Open file or log, Save profile.

### Data Transfer (Non-Essentials)
- **Opening:** Tools -> Data Transfer. Dragging tables to a DB in another connection also opens it; within the same connection it copies directly.
- **Step 1:** Source and Target (connection, DB, schema), swap button, or target File (SQL Format, Encoding). Cross-engine for RDBMS; MongoDB transfers only to MongoDB.
  - Options:
    - Run control: Continue on error, Convert object name to (Lower/Upper), Create tables/collections, Create records, Create target database/schema if not exist, Drop target objects before create, Drop with CASCADE.
    - Include: auto increment, character set, checks, definers, engine/table type, excludes, FKs, indexes, other table/collection options, owners, rules, triggers, uniques.
    - Locking: Lock source tables, Lock target tables.
    - Insert style: complete inserts, DDL from SHOW CREATE TABLE, DDL from sqlite_master, delayed inserts, extended inserts, hexadecimal format for BLOB.
    - Transactions: Use single transaction, Use transaction.
- **Step 2:** objects. "All <objects> during execution (*)" also picks up objects added later; "Custom" transfers only the checked ones.
  - Transfer Mode per table/view: Auto or Advanced. Advanced offers Target Name, All or Custom fields (rename), All rows, Number of rows per batch, Custom recordsets (expressions), Recordset Generator (Field, Start, End, count), SQL Preview, Use transaction for each recordset, Transfer as table (views).
- **Step 3:** summary, Continue on error, Drop target objects, Start.
- **Profiles:** Load Profile.

### Data Synchronization (Non-Essentials)
- **Limits:** not SQL Server 2000. Oracle skips LOB and LONG columns. MySQL and MariaDB can sync with each other.
- **Step 1:** connections and options: Insert records, Delete records, Update records.
- **Step 2:** table/collection mapping (same names auto-mapped), Key Mapping, Field Mapping. Then Compare & Preview, or Compare & Deploy.
- **Step 3:** per-table counts of insert/update/delete.
  - "Show identical table and others" toggle.
  - Source and target grids with differences highlighted. Show Assistant Viewer.
  - Per-record checkboxes.
  - Show filter: Difference / Insert / Update / Delete / Same / All Rows.
- **Step 4:** Deployment Script. Options Continue on error, Run multiple queries, Use transaction. Edit Script, Copy, Open in Query Editor, Start.

### Structure Synchronization (Non-Essentials)
- **Engines:** MySQL, Oracle, PostgreSQL, SQL Server, MariaDB.
- **Compare options:** the model list plus events, functions, sequences, and tablespace and physical attributes.
- **Results:** tree grouped by type or operation, DDL Comparison, Deployment Script, then the same deploy steps.

### Dump and Execute files
- **Dump SQL File / Dump Script File** (Mongo `.js`): Structure Only or Structure And Data, from a DB, schema or selected tables.
- **Execute SQL File / Execute Script File:**
  - Encoding, Continue on error, Run multiple queries in each execution, SET AUTOCOMMIT=0 / No Auto Commit.
  - Drag a `.sql` file onto a connection to open it.

### MongoImport / MongoExport
- External executables required.
- General and Advanced tabs, Message Log, profiles.
- Formats: import JSON / CSV / TSV; export JSON / CSV.

## 11. Data Generation (Non-Essentials)
- **Step 1:** target connection. Options: Continue on error, Empty tables/collections before generating data, Run multiple insert statements, Use extended inserts, Use transaction.
- **Step 2:** objects, rows per table, Table Generation Order. Copy and paste settings between objects.
  - Generator is auto-assigned per column. Common options: Include default values (%), Include null values (%), Set unique, Convert value to (lower / upper / proper), Disable data linkage between fields.
- **Generators:**
  - Number (Start, End, Integer or Decimal)
  - Date / Time / DateTime (ranges, Whole Day, Day of the Week)
  - Sequence (Start, Increment, Min, Max, Cycle)
  - Enum (values)
  - Text (character count range)
  - Image/Binary (width, height, format, or random pick from a folder)
  - Foreign Key (schema, table, field; mode random / no repeat / N repetitions)
  - UUID (hyphens, regex)
  - Regular Expression (full syntax table)
  - Name (first / last / full; EN, ZH, JA)
  - Gender, Title, Marital Status
  - Phone Number (format, delimiters, regions)
  - Email (domains)
  - Job Title, Social Network ID
  - Payment Method, Credit Card Type, Credit Card Number, Credit Card Date
  - Company Name, Department, Industry
  - Address (line1, line2 or full; US, UK, CN, JP), City, Region (names or ISO 3166 alpha-2/3)
  - Product Name (keywords), Product Category
  - Color, Size, Weight Unit
  - Barcode, SKU
  - IP Address (v4/v6), MAC Address
  - File Path (Windows / macOS / Linux), File Name, File Extension
  - URL, Hostname
- **Step 3:** Preview, then Start.

## 12. Charts and Dashboards (Premium and Enterprise)
- **Engines and files:** MySQL, Oracle, PostgreSQL, SQLite, SQL Server, MariaDB. Workspace files are `.ncharts`.
- **Workspace:** Content pane (Detail or Icon view; filter All / Data Source / Chart / Dashboard). Properties pane (General / Using / Used By).
- **Data Source:**
  - Inputs: DB connections, files or ODBC (import or link), or connections from existing data sources.
  - Drag tables onto the Design pane and join them (join type and fields).
  - Live mode or Archive mode. Apply and Refresh Data.
  - New Data Source Query (SQL or Query Builder; Import Query).
  - Connection management: Convert to Workspace Connection, Recreate Connection, Edit Connection, Change Connection Password, Edit File Connection Settings, Change Linked File Path.
  - Filter, Sort and Projection panes.
  - Custom fields: Type-Changed, Concatenated, Mapped, Custom-Sorted (edit or remove).
- **Chart:**
  - Panes: Data Source pane, Metric pane (chart type plus shelves), Preview, Properties.
  - Field options: Set Alias, Data Filter / Display Filter, Sort.
  - Aggregate functions:
    - Number: Sum, Average, Count, Count(Distinct), Min, Max, Median.
    - DateTime: Year, Quarter, Month, Week, Day, Hour, Minute, Second, and Extract variants.
    - Text: First, Last, Count.
  - Add Range (Fixed bin size or Custom stops). Value Operation (Cumulative / Subtract).
  - Trend lines, rich formatting properties, DateTime format specifiers.
- **Chart types:**
  - Bar: Vertical, Vertical Stacked, Horizontal, Horizontal Stacked.
  - Line, Area, Stacked Area. Bar and Line, Stacked Bar and Line.
  - Waterfall, Tornado, Pie, Donut, Scatter, Heatmap, Treemap.
  - Value, Trend, KPI, Gauge, Table, Pivot Table.
  - Control: an interactive filter shown as a list, range or combo box.
- **Dashboard:**
  - Pages pane (page size, background or image, opacity, notes). Add Chart, Add Text (URL link), Add Image, Add Shape.
  - Arrange: front/back, align, distribute, Group (Link Scrolling, Link Highlight, Show Hints) / Ungroup, Lock / Unlock.
  - Show Grid, Snap to Grid. Refresh Data.
  - Present mode (F11; ESC exits). Sort within charts while presenting.
  - Print, Export To PDF / PNG / SVG / JPG. Switch Theme (dark).

## 13. Automation (Non-Essentials)
- **Job types:** Query, Backup (including Backup Server), Data Transfer, Data Synchronization, Data Generation, Charts (export a dashboard to PDF, with "Append timestamp to file name"), Import, Export, MongoDump, MongoImport, MongoExport, MapReduce.
- **Batch job:** Available Jobs moved to Selected Jobs (drag or double-click), Move Up / Move Down, jobs can span servers. Attach to Email / Detach from Email.
- **Options:** Continue on error. Send Email settings:
  - When (success / failure)
  - From, To, CC, Subject, Body
  - Host (SMTP), Port, Use authentication, Secure connection (TLS / SSL / Never)
  - Send Test Mail
- **Running:** Save, Start (manual run with Message Log). Set Task Schedule / Delete Task Schedule uses Windows Task Scheduler ("Run whether user is logged on or not" needs the OS password).
- **CLI:** `navicat.exe -batchjob <Name>`.

## 14. Backup and Restore
- **Built-in tool** (MySQL, PostgreSQL, SQLite, MariaDB; Non-Essentials):
  - Backup tabs: General (comment), Object Selection (All during execution or Custom), Advanced (Lock All Tables, Use Single Transaction (InnoDB), Use specify file name; default name format is YYYYMMDDhhmmss).
  - Restore: drops, recreates and inserts. Restore Backup into the current DB, or "Restore Backup from" a file.
    - Options: Continue on error; Create indexes, tables, records, triggers; Empty table; Lock tables for write; Insert auto increment values.
    - Overwrite existing: events, functions, indexes, sequences, tables, triggers, types, views.
    - Run multiple queries, extended inserts, Use transaction.
  - Extract SQL (from the current DB or "Extract SQL from" a file). Can restore macOS and Linux backups.
- **Oracle Data Pump** (10g+, needs SYSDBA):
  - Export: Generate SQL, then Execute.
    - Job Name, Mode (FULL / TABLESPACE / SCHEMAS / TABLE), Content, objects, Dump Files.
    - Metadata Filter, Data Filter (Query, Sample), Remap Data.
    - Encryption (content, algorithm, mode Transparent / Password / Dual, password).
    - Advanced: threads, reuse file, XMLCLOBS, cluster, service, edition, version, compression, transportable, DB link, estimate, access method, log file, flashback SCN or time.
    - "Show Advanced Options" toggle. Change Directory.
  - Import: similar, plus Table Exists Action, Network (DB link…), Filter, Remap DataFiles, Schemas, TableSpaces and Tables, and transforms.
- **SQL Server Backup:**
  - General: Backup Set Name, Description, Type (Full / Differential / Transaction-Log), Copy-only, New or Existing Media Set (Add device).
  - Components: whole or partial database, files, filegroups.
  - Advanced: expiry, passwords, format media, overwrite, checks, truncate log, tail-log backup, verify, checksum, continue on error, compression.
  - Backup From This Setting.
- **SQL Server Restore:**
  - Restore to database, source devices, Latest possible / Specific time / Marked transaction, restore plan.
  - Options: Restore files to, WITH REPLACE, WITH RESTRICTED_USER, WITH KEEP_REPLICATION, RECOVERY / NORECOVERY / STANDBY.
  - Restore From File.
- **MongoDump / MongoRestore:** external executables, General and Advanced tabs, profiles.

## 15. Users, roles and privileges
- **MySQL/MariaDB:**
  - User: User Name, Host (`%`), Plugin, Password and confirm, Expire Password Policy.
  - Advanced: Max queries/updates/connections per hour, Max user connections, Use OLD_PASSWORD, SSL Type (ANY / SPECIFIED / X509).
  - Tabs: Member Of (Granted / Default; MySQL 8+, MariaDB 10.0.5+), Server Privileges (Grant All / Revoke All), Privileges (Add Privilege tree, then grid State).
  - Role Designer: Member Of, Members, Server Privileges, Privileges.
- **Oracle:**
  - User: Authentication (PASSWORD with Expire / EXTERNAL / GLOBAL), Default and Temporary Tablespace, Profile, Lock Account.
  - Tabs: Member Of (Granted / Admin Option / As Default), Quotas, Server Privileges (Grant All With Grant Option), Privileges.
  - Role: PASSWORD / EXTERNAL / GLOBAL / NOT IDENTIFIED.
- **PostgreSQL:**
  - User and Group (versions 8.0 and earlier): User ID, Password Encryption, Expiry Date, Superuser, Can create database.
  - Role (8.1+): Role ID, Can login, Password, Connection Limit (-1), Expiry, Superuser, Create DB, Create Role, Inherit, Update catalog, Replicate, Bypass RLS. Member Of, Members, Privileges.
- **SQL Server:**
  - Login: Name, Authentication Type, Password, Old Password, Enforce Policy / Expiration, Must Change, Default DB / Language, Certificate, Asymmetric Key, Enabled, Credential.
  - Login tabs: Roles, User Mapping, Server Permissions (Grant / With Grant / Deny), Endpoint Permissions, Login Permissions (click the checkbox 1, 2 or 3 times for grant, grant-with-option, deny).
  - Server Role (Members).
  - Database User: Authentication, Login, Default Schema, Certificate or Key, Roles, Owned Schemas, Database Permissions, Object Permissions.
  - Database Role: Owner, Member Of, Members, Owned Schemas, permissions.
  - Application Role.
- **SQLite:** User Name, Password, Administrator. Creating a user makes the DB require authentication.
- **MongoDB:**
  - User: Password, Password Digestor, Mechanisms (SCRAM), Custom Data, Built-In Roles, User-Defined Roles, Authentication Restrictions (Client Source, Server Address).
  - Role: Members (Roles / Users), Privileges.
- **Privilege Manager:** object tree, then Add Privilege, then check users and set privileges (all engines except SQLite).

## 16. Other tools
- **Server Monitor** (Non-Essentials; MySQL, Oracle, PostgreSQL, SQL Server, MariaDB, MongoDB):
  - Process List across servers (server, ID, serial, user, host, DB, command, time, state, info, CPU). Auto Refresh with interval, On Top, End Process.
  - Variables: editable for MySQL, MariaDB and Oracle.
  - Status.
- **Schema Analysis** (MongoDB): Filter, Projection, Sample; Analyze / Stop; type and distribution charts.
- **Virtual Grouping:** applies to connections, tables, collections, views, materialized views, functions, indexes, triggers, MapReduce, GridFS, queries, backups, automation, models and charts. New Group, Move To, Exclude From Group, drag and drop. Stored in `vgroup.json`.
- **Find in Database/Schema** (Tools; CTRL+SHIFT+F):
  - Connection, DB and schema; Find what; Look in Data or Structure.
  - Search Mode: Contains / Whole Word / Prefix / Regular Expression. Case Insensitive.
  - Structure object types: Tables, Collections, Views, Functions, Queries, Indexes, Triggers, Events, Materialized Views.
  - Double-click a result to open it.
- **Print Structure:** Print Database / Schema / Tables produces a report that can be printed or saved as PDF or HTML (RDBMS).
- **Console** (F6): multiple windows. Oracle needs SQL*Plus; MongoDB needs the Mongo Shell.
- **Favorites:** Add to Favorites (CTRL+SHIFT+1..9, or tab right-click), open with CTRL+1..9, Clear Favorites.
- **Dark Theme**, and search filters in the trees, the Objects tab and the Model window.
- **Trace logs:** `history.log`, `cmdline.log`, `LogImport.txt`, `navicat.log`.

## 17. Options (Tools -> Options)
- **General:**
  - Theme (System or Dark)
  - Allow opening multiple forms for same object
  - Show objects under schema in navigation pane
  - Show toolbar caption
  - Show function wizard
  - Ask to save new queries/profiles before closing
  - Check for updates on startup
  - Use safe confirm dialog
  - Share Usage Data
- **Tabs:** Open new tab in (Main Window / Last Tab Window / Last Tab Window (Except Main Window) / New Window). On Startup (Open Objects tab only / Continue where you left off / Open a specific tab or set of tabs).
- **Code Completion:** Use code completion, Auto update code completion info, Auto select first suggestion, Clear Code Completion Info.
- **Editor:**
  - Show line number, Use code folding, Use brace highlighting, Use syntax highlighting (disable above N MB), Use word wrap, Tab Width.
  - Editor Font. Colors for Common, Keyword, String, Number, Comment and Background.
- **Records:** Limit records per page (global), Auto Begin Transaction, Grid Font, Display Format (date/time/datetime specifiers), Show Thousands Separator.
- **Auto Recovery:** Auto Save Interval for Query, Model and Charts.
- **File Locations:** Logs Location, Profiles Location (extension table listed in the manual), Export Wizard default directory.
- **Connectivity:** Verify server certificate against CA. Proxy (HTTP / SOCKS5; Host, Port, User, Password). Test Connectivity.
- **Environment:** SQL*Plus, Mongo Shell, and mongodump / mongorestore / mongoimport / mongoexport paths. External Editor. OCI library (`oci.dll`).
- **Advanced:** Enable diagnostic logging, Allow multiple Navicat instances, Register on "Open with" List, Register Navicat URI Protocol.

## 18. Hot keys
I re-paired these by hand because the PDF table extracts with columns shifted.

| Area | Key | Action |
|---|---|---|
| Common | CTRL+SHIFT+1..9 | Add to Favorites |
| Common | F8 | Navicat Main Window / Objects Tab |
| Common | CTRL+TAB, SHIFT+CTRL+TAB | Next Window / Tab |
| Common | CTRL+Q | New Query |
| Common | F1 | Help |
| Main window | CTRL+1..9 | Open Favorites Link |
| Main window | F6 | Console |
| Main window | CTRL+L | History Log |
| Main window | F12 | Show Only Active Objects |
| Main window | CTRL+N | New Object |
| Main window | CTRL+SHIFT+F | Find in Database/Schema |
| ER Diagram view | F5 | Refresh |
| ER Diagram view | ESC | Select |
| ER Diagram view | H | Move Diagram |
| ER Diagram view | R | New Foreign Key |
| ER Diagram view | DELETE | Delete Selected Foreign Key |
| ER Diagram view | CTRL+= / CTRL+- / CTRL+0 | Zoom In / Zoom Out / Reset Zoom |
| Table/Collection Designer | CTRL+O | Open Table / Collection |
| Table/Collection Designer | CTRL+F | Find Field |
| Table/Collection Designer | F3 / SHIFT+F3 | Find Next Field / Find Previous Field |
| Data Viewer | CTRL+D | Design Object |
| Data Viewer | CTRL+Q | Query Object |
| Data Viewer | CTRL+F / F3 | Find Text / Find Next Text |
| Data Viewer | CTRL+G | Go to Row |
| Data Viewer | CTRL+LEFT / CTRL+RIGHT | First / Last Column of Current Record |
| Data Viewer | CTRL+HOME / CTRL+END | First / Last Row of Current Column |
| Data Viewer | CTRL+PAGE UP or CTRL+UP / CTRL+PAGE DOWN or CTRL+DOWN | First / Last Row of Current Page |
| Data Viewer | **CTRL+R** | **Apply Filter / Apply Sort** |
| Data Viewer | SHIFT+ARROW | Select Cells |
| Data Viewer | CTRL+ENTER | Edit Data with Opening Editor |
| Data Viewer | INSERT or CTRL+N | Add Record |
| Data Viewer | CTRL+DELETE | Delete Records |
| Data Viewer | **CTRL+S** | **Apply Record Changes** |
| Data Viewer | **ESC** | **Discard Record Changes** |
| Data Viewer | CTRL+T | Stop Loading Data |
| View / Materialized View Designer | CTRL+O | Import SQL |
| View / Materialized View Designer | CTRL+E | Switch to Definition |
| View / Materialized View Designer | CTRL+R | Preview |
| View / Materialized View Designer | ALT+1 | Switch to Result Tab |
| Query Designer | CTRL+O | Open External File |
| Query Designer | CTRL+E | Select Current Statement |
| Query Designer | CTRL+R | Run / Run Selected |
| Query Designer | CTRL+SHIFT+R | Run Current Statement |
| Query Designer | CTRL+T | Stop |
| Query Designer | ALT+0..9 | Switch to Result Tab |
| Query Editor | CTRL+SHIFT+V | Paste from Clipboard Stack |
| Query Editor | CTRL+/ | Comment / Uncomment Line |
| Query Editor | CTRL+F / F3 | Find Text / Find Next Text |
| Query Editor | CTRL+= / CTRL+- / CTRL+0 | Zoom In / Zoom Out / Reset Zoom |
| SQL Builder | CTRL+= / CTRL+- / CTRL+0 | Zoom In / Zoom Out / Reset Zoom |
| Debugger | F9 / F10 / F11 / SHIFT+F11 | Run / Step Over / Step In / Step Out |
| Model | CTRL+D | New Diagram |
| Model | CTRL+P | Print Diagram |
| Model | ESC | Select |
| Model | H | Move Diagram |
| Model | T / V / R | New Table/Entity / New View / New Foreign Key/Relation |
| Model | A / N / I / L | New Label / New Note / New Image / New Layer |
| Model | CTRL+B | Bold selected object |
| Model | CTRL+= / CTRL+- / CTRL+0 | Zoom In / Zoom Out / Reset Zoom |
| Charts | CTRL+D | Design Object |
| Charts | F5 | Refresh Data |
| Charts | F11 | Present Dashboard |
| Charts | CTRL+P | Print Dashboard |