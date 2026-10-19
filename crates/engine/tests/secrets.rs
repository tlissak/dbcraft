//! Saved passwords in the OS keychain (ROADMAP 1.3), against an in-memory store: migration out of
//! the file, reload, change, delete, failures. One ignored test touches the real OS keychain.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dbcraft_core::{Error, Result};
use dbcraft_engine::Session;
use dbcraft_engine::secrets::{MemoryStore, SecretStore};
use serde_json::json;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("dbcraft-secrets-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A file-mode workspace with a database password and an SSH password.
fn file_workspace(d: &std::path::Path) -> PathBuf {
    let path = d.join("workspace.json");
    let (mut s, _) = Session::with_workspace_file(path.clone());
    s.dispatch(
        "connection.save",
        &json!({"id": "pg", "kind": "postgres", "name": "pg", "host": "db", "user": "app", "password": "s3cret-db",
                "ssh": {"host": "bastion", "user": "me", "password": "s3cret-ssh"}}),
    )
    .unwrap();
    s.dispatch("connection.save", &json!({"id": "lite", "kind": "sqlite", "name": "lite", "database": d.join("x.db").to_string_lossy()})).unwrap();
    path
}

fn file_text(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn file_mode_keeps_passwords_in_the_file() {
    let d = dir("file");
    let path = file_workspace(&d);
    assert!(file_text(&path).contains("s3cret-db"), "dev/test workspaces are unchanged");
    // Attaching a store without migrating leaves a file-mode workspace alone.
    let store = Arc::new(MemoryStore::default());
    let (mut s, _) = Session::with_workspace_file(path.clone());
    s.attach_secrets(store.clone(), false).unwrap();
    s.save_workspace().unwrap();
    assert!(file_text(&path).contains("s3cret-db"));
    assert!(store.entries().is_empty());
}

#[test]
fn migrate_reload_change_and_delete() {
    let d = dir("migrate");
    let path = file_workspace(&d);
    let store = Arc::new(MemoryStore::default());

    let (mut s, _) = Session::with_workspace_file(path.clone());
    assert!(s.attach_secrets(store.clone(), true).unwrap().is_empty());
    let text = file_text(&path);
    assert!(!text.contains("s3cret"), "no password left in the file:\n{text}");
    assert!(text.contains("\"password_storage\": \"keychain\""));
    let ws_id = s.workspace.id.clone();
    assert!(!ws_id.is_empty());
    let entries = store.entries();
    assert_eq!(entries.get(&format!("{ws_id}/pg/password")).map(String::as_str), Some("s3cret-db"));
    assert_eq!(entries.get(&format!("{ws_id}/pg/ssh_password")).map(String::as_str), Some("s3cret-ssh"));
    assert!(!entries.keys().any(|k| k.contains("/lite/")), "no entries for connections without passwords");
    // In memory the session still has the passwords (it can connect).
    assert_eq!(s.workspace.connection("pg").unwrap().password.as_deref(), Some("s3cret-db"));

    // Next run: passwords come back from the keychain.
    let (mut s2, _) = Session::with_workspace_file(path.clone());
    assert_eq!(s2.workspace.connection("pg").unwrap().password, None);
    assert!(s2.attach_secrets(store.clone(), false).unwrap().is_empty());
    let pg = s2.workspace.connection("pg").unwrap();
    assert_eq!(pg.password.as_deref(), Some("s3cret-db"));
    assert_eq!(pg.ssh.as_ref().unwrap().password.as_deref(), Some("s3cret-ssh"));

    // Change the password, drop the SSH section: the keychain follows; the file never sees them.
    s2.dispatch("connection.save", &json!({"id": "pg", "kind": "postgres", "name": "pg", "host": "db", "user": "app", "password": "n3w"})).unwrap();
    let entries = store.entries();
    assert_eq!(entries.get(&format!("{ws_id}/pg/password")).map(String::as_str), Some("n3w"));
    assert!(!entries.contains_key(&format!("{ws_id}/pg/ssh_password")), "stale SSH password removed");
    assert!(!file_text(&path).contains("n3w"));

    // Clearing a password deletes its entry; deleting the connection deletes the rest.
    s2.dispatch("connection.save", &json!({"id": "pg", "kind": "postgres", "name": "pg", "host": "db", "user": "app"})).unwrap();
    assert!(!store.entries().contains_key(&format!("{ws_id}/pg/password")));
    s2.dispatch("connection.save", &json!({"id": "pg", "kind": "postgres", "name": "pg", "host": "db", "user": "app", "password": "again"})).unwrap();
    s2.dispatch("connection.delete", &json!({"id": "pg"})).unwrap();
    assert!(store.entries().is_empty(), "{:?}", store.entries());
}

/// Counts writes, to prove saves don't rewrite unchanged passwords.
#[derive(Default)]
struct Counting {
    inner: MemoryStore,
    writes: AtomicUsize,
}

impl SecretStore for Counting {
    fn get(&self, account: &str) -> Result<Option<String>> {
        self.inner.get(account)
    }
    fn set(&self, account: &str, secret: &str) -> Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.set(account, secret)
    }
    fn delete(&self, account: &str) -> Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.delete(account)
    }
}

#[test]
fn unchanged_passwords_are_not_rewritten() {
    let d = dir("count");
    let path = file_workspace(&d);
    let store = Arc::new(Counting::default());
    let (mut s, _) = Session::with_workspace_file(path.clone());
    s.attach_secrets(store.clone(), true).unwrap();
    let after_migration = store.writes.load(Ordering::SeqCst);
    // Unrelated saves (a saved query, another connection) don't touch the keychain.
    s.dispatch("query.save", &json!({"name": "q", "connection": "lite", "sql": "select 1"})).unwrap();
    s.dispatch("connection.save", &json!({"id": "lite2", "kind": "sqlite", "name": "lite2", "database": d.join("y.db").to_string_lossy()})).unwrap();
    assert_eq!(store.writes.load(Ordering::SeqCst), after_migration);
    // Nor does the next run's first save.
    let (mut s2, _) = Session::with_workspace_file(path);
    s2.attach_secrets(store.clone(), false).unwrap();
    s2.save_workspace().unwrap();
    assert_eq!(store.writes.load(Ordering::SeqCst), after_migration);
}

/// A keychain that refuses everything.
struct Broken;

impl SecretStore for Broken {
    fn get(&self, _: &str) -> Result<Option<String>> {
        Err(Error::Io("locked".into()))
    }
    fn set(&self, _: &str, _: &str) -> Result<()> {
        Err(Error::Io("locked".into()))
    }
    fn delete(&self, _: &str) -> Result<()> {
        Err(Error::Io("locked".into()))
    }
}

#[test]
fn a_broken_keychain_never_loses_passwords() {
    let d = dir("broken");
    let path = file_workspace(&d);
    let (mut s, _) = Session::with_workspace_file(path.clone());
    assert!(s.attach_secrets(Arc::new(Broken), true).is_err(), "the failure is reported");
    assert!(file_text(&path).contains("s3cret-db"), "passwords stay in the file");
    assert_eq!(s.workspace.password_storage, dbcraft_model::PasswordStorage::File);

    // A keychain-mode workspace whose keychain can't be read starts without passwords, not broken.
    let store = Arc::new(MemoryStore::default());
    let (mut s, _) = Session::with_workspace_file(path.clone());
    s.attach_secrets(store, true).unwrap();
    let (mut s2, _) = Session::with_workspace_file(path);
    let failed = s2.attach_secrets(Arc::new(Broken), false).unwrap();
    assert_eq!(failed.len(), 3, "pg password, pg SSH password, lite password: {failed:?}");
    assert_eq!(s2.workspace.connection("pg").unwrap().password, None);
    assert!(s2.dispatch("connection.list", &json!({})).is_ok());
}

#[test]
#[ignore = "writes to the real OS keychain (creates and deletes one DbCraft test entry)"]
fn os_keychain_round_trip() {
    let store = dbcraft_engine::secrets::OsKeychain::available().expect("OS keychain available");
    let acct = format!("dbcraft-test-{}/conn/password", std::process::id());
    store.set(&acct, "round-trip \u{2713}").unwrap();
    assert_eq!(store.get(&acct).unwrap().as_deref(), Some("round-trip \u{2713}"));
    store.delete(&acct).unwrap();
    assert_eq!(store.get(&acct).unwrap(), None);
    store.delete(&acct).unwrap(); // deleting twice is fine
}
