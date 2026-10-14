//! Render the README screenshots offscreen from the real app (no window, no desktop capture):
//!
//!   cargo run -p dbcraft-ui-egui --example screenshots -- docs/images
//!
//! Builds a demo SQLite database of classic public-domain films, drives the UI with egui_kittest
//! (clicks, filters, edits) and saves each screen as a PNG. Deterministic and repeatable.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // a dev tool, not app code

use std::path::{Path, PathBuf};

use dbcraft_core::{Grant, UserInfo};
use dbcraft_engine::Session;
use dbcraft_model::Schedule;
use dbcraft_ui_egui::state::{ConnTab, FilterRow, Tab, UserSubTab, UserTab};
use dbcraft_ui_egui::{DbCraftApp, StartupAction};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

const FILMS: &[(&str, i64, &str, &str, i64, &str, bool)] = &[
    ("The Cabinet of Dr. Caligari", 1920, "Robert Wiene", "Germany", 76, "Horror", true),
    ("The Kid", 1921, "Charlie Chaplin", "USA", 68, "Comedy", true),
    ("Nosferatu", 1922, "F. W. Murnau", "Germany", 94, "Horror", true),
    ("Safety Last!", 1923, "Fred C. Newmeyer", "USA", 73, "Comedy", true),
    ("Sherlock Jr.", 1924, "Buster Keaton", "USA", 45, "Comedy", true),
    ("Battleship Potemkin", 1925, "Sergei Eisenstein", "USSR", 75, "Drama", true),
    ("The Gold Rush", 1925, "Charlie Chaplin", "USA", 95, "Comedy", true),
    ("The Phantom of the Opera", 1925, "Rupert Julian", "USA", 93, "Horror", true),
    ("The General", 1926, "Buster Keaton", "USA", 78, "Comedy", true),
    ("Metropolis", 1927, "Fritz Lang", "Germany", 153, "Sci-Fi", true),
    ("Sunrise", 1927, "F. W. Murnau", "USA", 94, "Drama", true),
    ("Wings", 1927, "William A. Wellman", "USA", 144, "War", true),
    ("The Passion of Joan of Arc", 1928, "Carl Theodor Dreyer", "France", 82, "Drama", true),
    ("Pandora's Box", 1929, "G. W. Pabst", "Germany", 133, "Drama", true),
    ("Man with a Movie Camera", 1929, "Dziga Vertov", "USSR", 68, "Documentary", true),
    ("City Lights", 1931, "Charlie Chaplin", "USA", 87, "Comedy", true),
    ("M", 1931, "Fritz Lang", "Germany", 117, "Thriller", false),
    ("Frankenstein", 1931, "James Whale", "USA", 70, "Horror", false),
    ("King Kong", 1933, "Merian C. Cooper", "USA", 100, "Adventure", false),
    ("Modern Times", 1936, "Charlie Chaplin", "USA", 87, "Comedy", true),
];

fn demo_session(dir: &Path) -> Session {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let (mut s, _) = Session::with_workspace_file(dir.join("workspace.json"));
    let db = |n: &str| dir.join(n).to_string_lossy().into_owned();
    s.dispatch("connection.save", &json!({"id": "studio", "kind": "sqlite", "name": "Film Studio (SQLite)", "database": db("studio.db")})).unwrap();
    s.dispatch("connection.save", &json!({"id": "archive", "kind": "sqlite", "name": "Archive (SQLite)", "database": db("archive.db")})).unwrap();
    s.dispatch(
        "connection.save",
        &json!({"id": "prod", "kind": "postgres", "name": "Production (PostgreSQL)", "host": "db.internal", "port": 5432, "user": "app", "database": "films",
        "ssh": {"host": "bastion.example.com", "user": "deploy"}}),
    )
    .unwrap();
    s.dispatch(
        "connection.save",
        &json!({"id": "shop", "kind": "mysql", "name": "Shop (MySQL)", "host": "localhost", "port": 3306, "user": "root", "database": "shop"}),
    )
    .unwrap();
    s.dispatch(
        "connection.save",
        &json!({"id": "legacy", "kind": "mariadb", "name": "Legacy (MariaDB)", "host": "legacy.internal", "port": 3306, "user": "report"}),
    )
    .unwrap();
    s.dispatch("connection.open", &json!({"id": "studio"})).unwrap();
    let mut sql = String::from(
        "create table directors (id integer primary key, name text not null, born int, country text);
         create table films (id integer primary key, title varchar(120) not null, year int, director_id int references directors(id),
            country text, runtime_min int, genre text, silent boolean, restored_at datetime);
         create table actors (id integer primary key, name text not null, born int);
         create table film_cast (film_id int, actor_id int, role text, primary key (film_id, actor_id));
         create table screenings (id integer primary key, film_id int, venue text, starts_at datetime, seats int, sold int);
         create view film_list as select f.title, f.year, d.name as director from films f join directors d on d.id = f.director_id;
         create index films_year on films(year);",
    );
    let mut directors: Vec<&str> = Vec::new();
    for (i, (title, year, director, country, runtime, genre, silent)) in FILMS.iter().enumerate() {
        let did = match directors.iter().position(|d| d == director) {
            Some(p) => p + 1,
            None => {
                directors.push(director);
                sql.push_str(&format!(
                    "insert into directors (id, name, country) values ({}, '{}', '{country}');",
                    directors.len(),
                    director.replace('\'', "''")
                ));
                directors.len()
            }
        };
        let restored = if i % 3 == 0 { format!("'20{:02}-0{}-1{} 10:30:00'", 10 + i % 12, 1 + i % 9, i % 9) } else { "NULL".into() };
        sql.push_str(&format!(
            "insert into films (title, year, director_id, country, runtime_min, genre, silent, restored_at) values ('{}', {year}, {did}, '{country}', {runtime}, '{genre}', {}, {restored});",
            title.replace('\'', "''"),
            i32::from(*silent)
        ));
    }
    for (i, a) in [
        "Max Schreck",
        "Brigitte Helm",
        "Buster Keaton",
        "Charlie Chaplin",
        "Louise Brooks",
        "Renée Falconetti",
        "Harold Lloyd",
        "Lon Chaney",
        "Fay Wray",
        "Boris Karloff",
        "Peter Lorre",
    ]
    .iter()
    .enumerate()
    {
        sql.push_str(&format!("insert into actors (id, name) values ({}, '{}');", i + 1, a));
    }
    sql.push_str("insert into film_cast values (3, 1, 'Count Orlok'), (10, 2, 'Maria'), (9, 3, 'Johnnie Gray'), (5, 3, 'Projectionist'), (2, 4, 'The Tramp'), (7, 4, 'The Lone Prospector'), (16, 4, 'A Tramp'), (14, 5, 'Lulu'), (13, 6, 'Jeanne'), (4, 7, 'The Boy'), (8, 8, 'The Phantom'), (19, 9, 'Ann Darrow'), (18, 10, 'The Monster'), (17, 11, 'Hans Beckert');");
    for i in 0..40 {
        let film = 1 + (i * 7) % 20;
        let venue = ["Cinémathèque", "Roxy", "Odeon", "Paramount"][i % 4];
        sql.push_str(&format!(
            "insert into screenings (film_id, venue, starts_at, seats, sold) values ({film}, '{venue}', '2026-11-{:02} {}:00:00', {}, {});",
            1 + i % 28,
            18 + i % 4,
            120 + (i % 3) * 40,
            40 + (i * 13) % 120
        ));
    }
    s.dispatch("query.run", &json!({"connection": "studio", "sql": sql})).unwrap();
    s
}

fn harness(app: DbCraftApp) -> Harness<'static, DbCraftApp> {
    Harness::builder().with_size(egui::vec2(1440.0, 860.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|_| app)
}

fn save(h: &mut Harness<DbCraftApp>, out: &Path, name: &str) {
    // No mouse pointer or hover highlight in the picture.
    h.event(egui::Event::PointerGone);
    for _ in 0..4 {
        h.step();
    }
    let img = h.render().unwrap_or_else(|e| panic!("render {name}: {e}"));
    let path = out.join(name);
    img.save(&path).unwrap();
    println!("saved {}", path.display());
}

fn wait(h: &mut Harness<DbCraftApp>, done: impl Fn(&DbCraftApp) -> bool) {
    for _ in 0..400 {
        h.step();
        if done(h.state()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    h.run_steps(3);
}

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "docs/images".into()));
    std::fs::create_dir_all(&out).unwrap();
    let dir = std::env::temp_dir().join("dbcraft-screenshots");

    // 1. Objects tab with the information pane. Uses the docker MySQL from dev/docker-compose.yml
    //    when it is running (richer table details), the demo SQLite otherwise.
    let mut s = demo_session(&dir);
    s.dispatch("connection.save", &json!({"id": "cinema", "kind": "mysql", "name": "Cinema (MySQL)", "host": "127.0.0.1", "port": 13306, "user": "dbcraft", "password": "dbcraft", "database": "sakila_lite"}))
        .unwrap();
    let mysql_up = s.dispatch("connection.open", &json!({"id": "cinema"})).is_ok();
    let (conn, db, table) = if mysql_up { ("cinema", "sakila_lite", "film") } else { ("studio", "main", "films") };
    let mut app = DbCraftApp::new(s, None);
    app.startup(StartupAction::Select { connection: conn.into(), database: Some(db.into()) });
    let mut h = harness(app);
    h.run_steps(6);
    h.get_by_label(table).click();
    h.run_steps(4);
    save(&mut h, &out, "dbcraft-objects.png");
    drop(h);

    // 2. Table viewer: filter, sort, pending edits, a new record, a deleted record, a selected row.
    let mut app = DbCraftApp::new(demo_session(&dir), None);
    app.startup(StartupAction::OpenTable { connection: "studio".into(), database: "main".into(), schema: None, table: "films".into() });
    let mut h = harness(app);
    h.run_steps(6);
    if let Some(Tab::Table(t)) = h.state_mut().state.tabs.get_mut(1) {
        t.show_filter = true;
        t.filters = vec![
            FilterRow { enabled: true, column: "year".into(), op: ">=".into(), value: "1924".into() },
            FilterRow { enabled: true, column: "silent".into(), op: "=".into(), value: "1".into() },
        ];
        t.sort = Some(("year".into(), false));
    }
    h.run_steps(2);
    h.get_by_label_contains("Apply Filter").click();
    h.run_steps(6);
    if let Some(Tab::Table(t)) = h.state_mut().state.tabs.get_mut(1) {
        t.edits.insert((2, 6), Some("Drama".into()));
        t.edits.insert((3, 5), Some("152".into()));
        t.deleted.insert(6);
        t.inserted.push([(1, Some("Faust".into())), (2, Some("1926".into())), (4, Some("Germany".into()))].into_iter().collect());
        t.grid = dbcraft_ui_egui::grid::GridSel::Rows { anchor: 4, rows: [4].into() };
    }
    save(&mut h, &out, "dbcraft-table.png");
    drop(h);

    // 3. Query editor with two result tabs and a selected row.
    let mut app = DbCraftApp::new(demo_session(&dir), None);
    app.startup(StartupAction::NewQuery {
        connection: "studio".into(),
        database: Some("main".into()),
        sql: "-- Screenings per film, best sellers first\nSELECT f.title, f.year, d.name AS director,\n       count(s.id) AS screenings, sum(s.sold) AS tickets\nFROM films f\nJOIN directors d ON d.id = f.director_id\nLEFT JOIN screenings s ON s.film_id = f.id\nGROUP BY f.id\nORDER BY tickets DESC;\n\nSELECT genre, count(*) AS films FROM films GROUP BY genre ORDER BY films DESC;".into(),
    });
    let mut h = harness(app);
    h.run_steps(4);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::R);
    h.run_steps(4);
    h.get_by_label("Row 2").click();
    h.run_steps(2);
    save(&mut h, &out, "dbcraft-query.png");
    drop(h);

    // 4. New Connection dialog, SSH tab.
    let mut app = DbCraftApp::new(demo_session(&dir), None);
    app.startup(StartupAction::Select { connection: "studio".into(), database: Some("main".into()) });
    app.startup(StartupAction::NewConnection);
    let mut h = harness(app);
    h.run_steps(4);
    if let Some(f) = h.state_mut().state.connection_form.as_mut() {
        f.kind = "postgres".into();
        f.name = "Production (PostgreSQL)".into();
        f.host = "10.0.4.12".into();
        f.port = "5432".into();
        f.user = "app".into();
        f.password = "secret".into();
        f.database = "films".into();
        f.ssh_enabled = true;
        f.ssh_host = "bastion.example.com".into();
        f.ssh_user = "deploy".into();
        f.ssh_auth = "publickey".into();
        f.ssh_key = "C:\\Users\\me\\.ssh\\id_ed25519".into();
        f.tab = ConnTab::Ssh;
        f.test_result = Some(Ok("PostgreSQL 17.6 on x86_64-pc-linux-gnu".into()));
    }
    save(&mut h, &out, "dbcraft-connection.png");
    drop(h);

    // 5. Data Transfer: SQLite studio -> archive, finished with its log.
    let mut app = DbCraftApp::new(demo_session(&dir), None);
    app.startup(StartupAction::Select { connection: "studio".into(), database: Some("main".into()) });
    app.startup(StartupAction::Tool("transfer".into()));
    let mut h = harness(app);
    h.run_steps(6);
    if let Some(Tab::Transfer(f)) = h.state_mut().state.tabs.get_mut(1) {
        f.target = Some("archive".into());
        f.drop_existing = true;
    }
    h.run_steps(2);
    h.get_by_label_contains(" Start").click();
    wait(&mut h, |a| matches!(a.state.tabs.get(1), Some(Tab::Transfer(f)) if f.status.is_some()));
    save(&mut h, &out, "dbcraft-transfer.png");
    drop(h);

    // 6. Automation: a nightly batch job with a schedule and a successful run.
    let mut s = demo_session(&dir);
    s.dispatch(
        "automation.save",
        &json!({"id": "job-1", "name": "Nightly backup + archive", "steps": [
            {"command": "backup.create", "params": {"connection": "studio", "database": "main"}, "label": "Backup main on Film Studio (SQLite)"},
            {"command": "tools.dataTransfer", "params": {"source": {"connection": "studio", "database": "main"}, "target": {"connection": "archive", "database": "main"}, "options": {"drop_existing": true}}, "label": "Data Transfer from Film Studio to Archive"},
            {"command": "query.run", "params": {"connection": "archive", "sql": "delete from screenings where starts_at < date('now', '-1 year')"}, "label": "Query on Archive (SQLite): purge old screenings"},
        ]}),
    )
    .unwrap();
    if let Some(j) = s.workspace.automations.get_mut(0) {
        j.schedule = Some(Schedule { frequency: "daily".into(), time: "02:00".into(), day: None });
    }
    let mut app = DbCraftApp::new(s, None);
    app.startup(StartupAction::Select { connection: "studio".into(), database: Some("main".into()) });
    app.startup(StartupAction::Tool("automation".into()));
    let mut h = harness(app);
    h.run_steps(4);
    h.get_by_label_contains("Nightly backup").click();
    h.run_steps(3);
    h.get_by_label_contains(" Start").click();
    wait(&mut h, |a| matches!(a.state.tabs.get(1), Some(Tab::Automation(v)) if v.status.is_some()));
    save(&mut h, &out, "dbcraft-automation.png");
    drop(h);

    // 7. Console.
    let mut app = DbCraftApp::new(demo_session(&dir), None);
    app.startup(StartupAction::Console { connection: "studio".into(), database: None });
    let mut h = harness(app);
    h.run_steps(4);
    for line in [
        "select genre, count(*) as films, round(avg(runtime_min)) as avg_min",
        "from films group by genre order by films desc;",
        "select title, year from films where director_id = 9;",
    ] {
        h.event(egui::Event::Text(line.into()));
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
    }
    save(&mut h, &out, "dbcraft-console.png");
    drop(h);

    // 8. User designer (PostgreSQL role, Privileges tab). Nothing is sent to a server.
    let mut app = DbCraftApp::new(demo_session(&dir), None);
    app.startup(StartupAction::Select { connection: "studio".into(), database: Some("main".into()) });
    let mut h = harness(app);
    h.run_steps(3);
    let user = UserInfo {
        name: "analyst".into(),
        attributes: [("INHERIT".to_string(), true), ("CREATEDB".to_string(), false)].into_iter().collect(),
        grants: vec![
            Grant { object: "DATABASE films".into(), privileges: vec!["CONNECT".into(), "TEMPORARY".into()], grant_option: false },
            Grant { object: "SCHEMA public".into(), privileges: vec!["USAGE".into()], grant_option: false },
            Grant { object: "TABLE public.films".into(), privileges: vec!["SELECT".into(), "UPDATE".into()], grant_option: false },
            Grant { object: "TABLE public.screenings".into(), privileges: vec!["SELECT".into(), "INSERT".into()], grant_option: true },
        ],
        ..UserInfo::default()
    };
    h.state_mut().state.tabs.push(Tab::User(Box::new(UserTab { connection: "prod".into(), user, sub: UserSubTab::Privileges, ..UserTab::default() })));
    h.state_mut().state.active_tab = 1;
    save(&mut h, &out, "dbcraft-users.png");
}
