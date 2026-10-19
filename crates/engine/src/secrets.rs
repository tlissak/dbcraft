//! Where saved passwords live (ROADMAP 1.3). A workspace keeps them either in `workspace.json`
//! (dev and test workspaces) or in the OS keychain: Windows Credential Manager, macOS Keychain,
//! the Secret Service on Linux. In keychain mode the file never contains a password.
//!
//! Entries are named `DbCraft` / `<workspace id>/<connection id>/<field>`, so two workspaces with
//! the same connection ids don't overwrite each other's passwords.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use dbcraft_core::{ConnectionConfig, Error, Result};
use dbcraft_model::{PasswordStorage, Workspace};

/// The keychain service name entries are filed under.
pub const SERVICE: &str = "DbCraft";

/// A place to keep secrets by account name.
pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>>;
    fn set(&self, account: &str, secret: &str) -> Result<()>;
    /// Deleting a missing entry is not an error.
    fn delete(&self, account: &str) -> Result<()>;
}

/// The operating system's credential store.
pub struct OsKeychain;

impl OsKeychain {
    /// `Some` when the platform keychain works on this machine.
    pub fn available() -> Option<Arc<dyn SecretStore>> {
        keyring::Entry::store_status().is_ok().then(|| Arc::new(OsKeychain) as Arc<dyn SecretStore>)
    }

    fn entry(account: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, account).map_err(keychain_err)
    }
}

fn keychain_err(e: keyring::Error) -> Error {
    Error::Io(format!("OS keychain: {e}"))
}

impl SecretStore for OsKeychain {
    fn get(&self, account: &str) -> Result<Option<String>> {
        match Self::entry(account)?.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keychain_err(e)),
        }
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        Self::entry(account)?.set_password(secret).map_err(keychain_err)
    }

    fn delete(&self, account: &str) -> Result<()> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keychain_err(e)),
        }
    }
}

/// In-memory store for tests (and a model of the keychain's behaviour).
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl MemoryStore {
    pub fn entries(&self) -> HashMap<String, String> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap_or_else(PoisonError::into_inner).get(account).cloned())
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).insert(account.to_string(), secret.to_string());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<()> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).remove(account);
        Ok(())
    }
}

/// The secret fields of a connection, by name.
const FIELDS: [&str; 2] = ["password", "ssh_password"];

fn account(ws: &Workspace, conn: &str, field: &str) -> String {
    format!("{}/{conn}/{field}", ws.id)
}

fn field_mut<'a>(c: &'a mut ConnectionConfig, field: &str) -> Option<&'a mut Option<String>> {
    match field {
        "password" => Some(&mut c.password),
        "ssh_password" => c.ssh.as_mut().map(|s| &mut s.password),
        _ => None,
    }
}

/// Fill passwords from the keychain into a freshly loaded keychain-mode workspace. Returns the
/// accounts that could not be read (the connection then simply asks for no password).
pub fn load(ws: &mut Workspace, store: &dyn SecretStore) -> Vec<String> {
    if ws.password_storage != PasswordStorage::Keychain {
        return Vec::new();
    }
    let mut failed = Vec::new();
    let id = ws.id.clone();
    for c in &mut ws.connections {
        for f in FIELDS {
            let acct = format!("{id}/{}/{f}", c.id);
            // Only fields the connection has (no SSH section: no SSH password to look up).
            let Some(slot) = field_mut(c, f) else { continue };
            match store.get(&acct) {
                Ok(v) => *slot = v,
                Err(e) => failed.push(format!("{acct}: {e}")),
            }
        }
    }
    failed
}

/// What goes into `workspace.json`: in keychain mode, a copy without passwords, after writing
/// changed passwords to the keychain. `stored` remembers what the keychain holds, so unchanged
/// passwords aren't rewritten on every save.
pub fn for_file(ws: &Workspace, store: Option<&dyn SecretStore>, stored: &mut HashMap<String, Option<String>>) -> Result<Workspace> {
    let (PasswordStorage::Keychain, Some(store)) = (ws.password_storage, store) else {
        return Ok(ws.clone());
    };
    let mut out = ws.clone();
    for c in &mut out.connections {
        for f in FIELDS {
            let acct = account(ws, &c.id, f);
            let Some(slot) = field_mut(c, f) else {
                // No SSH section: make sure no stale SSH password stays behind.
                if stored.get(&acct).is_some_and(Option::is_some) {
                    store.delete(&acct)?;
                    stored.insert(acct, None);
                }
                continue;
            };
            let value = slot.take();
            // An account never stored needs no delete.
            let known = stored.get(&acct).cloned().unwrap_or(None);
            if known != value {
                match &value {
                    Some(v) => store.set(&acct, v)?,
                    None => store.delete(&acct)?,
                }
                stored.insert(acct, value);
            }
        }
    }
    Ok(out)
}

/// The accounts and values a keychain-mode workspace holds in memory.
pub fn snapshot(ws: &Workspace) -> HashMap<String, Option<String>> {
    let mut out = HashMap::new();
    for c in &ws.connections {
        let mut c = c.clone();
        for f in FIELDS {
            let acct = account(ws, &c.id, f);
            if let Some(slot) = field_mut(&mut c, f) {
                out.insert(acct, slot.clone());
            }
        }
    }
    out
}

/// Forget a deleted connection's passwords.
pub fn forget(ws: &Workspace, store: Option<&dyn SecretStore>, stored: &mut HashMap<String, Option<String>>, conn: &str) -> Result<()> {
    let (PasswordStorage::Keychain, Some(store)) = (ws.password_storage, store) else { return Ok(()) };
    for f in FIELDS {
        let acct = account(ws, conn, f);
        store.delete(&acct)?;
        stored.remove(&acct);
    }
    Ok(())
}

/// A workspace id that is unique enough to name keychain entries (time + process).
pub fn new_workspace_id() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("ws-{:x}{:x}", nanos, std::process::id())
}
