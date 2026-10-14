//! SSH tunnel (local port forwarding) for server connections: `127.0.0.1:<ephemeral>` on this
//! machine forwards to `db_host:db_port` as seen from the SSH server, like Navicat's SSH tab.

use std::sync::Arc;

use dbcraft_core::{Error, Result, SshAuth, SshConfig};
use russh::client::{self, Config, Handle};
use russh::keys::{PrivateKeyWithHashAlg, PublicKeyOrCertificate, load_secret_key};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::common::{CONNECT_TIMEOUT, block_on};

struct Client;

impl client::Handler for Client {
    type Error = russh::Error;

    // Host keys are accepted as-is for now (Navicat does the same); known_hosts verification is
    // tracked in ROADMAP.md.
    async fn check_server_key(&mut self, _key: &PublicKeyOrCertificate) -> std::result::Result<bool, Self::Error> {
        Ok(true)
    }
}

/// A running tunnel. Dropping it stops forwarding and closes the SSH session.
pub struct Tunnel {
    pub local_port: u16,
    accept_loop: JoinHandle<()>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.accept_loop.abort();
    }
}

fn ssh_err(e: impl std::fmt::Display) -> Error {
    Error::Database(format!("SSH: {e}"))
}

/// Connect and authenticate to the SSH server, then forward a local port to `db_host:db_port`.
pub fn open(ssh: &SshConfig, db_host: &str, db_port: u16) -> Result<Tunnel> {
    if ssh.host.trim().is_empty() || ssh.user.trim().is_empty() {
        return Err(Error::InvalidParam("SSH tunnel needs a host and a user name".into()));
    }
    let key = match ssh.auth {
        SshAuth::Password => None,
        SshAuth::PublicKey => {
            let path = ssh
                .private_key
                .as_deref()
                .filter(|p| !p.trim().is_empty())
                .ok_or_else(|| Error::InvalidParam("SSH public key auth needs a private key file".into()))?;
            Some(load_secret_key(path, ssh.password.as_deref()).map_err(|e| ssh_err(format!("cannot load private key `{path}`: {e}")))?)
        }
    };
    let ssh = ssh.clone();
    let db_host = db_host.to_string();
    block_on(async move {
        let config = Arc::new(Config { nodelay: true, ..Config::default() });
        let mut session = tokio::time::timeout(CONNECT_TIMEOUT, client::connect(config, (ssh.host.as_str(), ssh.port), Client))
            .await
            .map_err(|_| ssh_err(format!("timed out connecting to {}:{}", ssh.host, ssh.port)))?
            .map_err(ssh_err)?;
        let auth = match key {
            None => session.authenticate_password(ssh.user.as_str(), ssh.password.clone().unwrap_or_default()).await.map_err(ssh_err)?,
            Some(k) => {
                let hash = session.best_supported_rsa_hash().await.map_err(ssh_err)?.flatten();
                session.authenticate_publickey(ssh.user.as_str(), PrivateKeyWithHashAlg::new(Arc::new(k), hash)).await.map_err(ssh_err)?
            }
        };
        if !auth.success() {
            return Err(ssh_err(format!("authentication failed for user `{}`", ssh.user)));
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let local_port = listener.local_addr()?.port();
        let session = Arc::new(session);
        let accept_loop = tokio::spawn(accept_loop(listener, session, db_host, db_port));
        Ok(Tunnel { local_port, accept_loop })
    })?
}

async fn accept_loop(listener: TcpListener, session: Arc<Handle<Client>>, db_host: String, db_port: u16) {
    let mut forwards = Vec::new();
    while let Ok((mut socket, peer)) = listener.accept().await {
        let session = session.clone();
        let db_host = db_host.clone();
        forwards.retain(|h: &JoinHandle<()>| !h.is_finished());
        forwards.push(tokio::spawn(async move {
            let Ok(channel) = session.channel_open_direct_tcpip(db_host, u32::from(db_port), peer.ip().to_string(), u32::from(peer.port())).await else {
                return;
            };
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
        }));
    }
}

// When the accept loop is aborted, in-flight forwards keep running until their socket closes
// (the driver's pool closes them when the connection drops); the session `Arc` goes with the last.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_are_errors() {
        let mut cfg = SshConfig { host: String::new(), port: 22, user: "u".into(), auth: SshAuth::Password, password: None, private_key: None };
        assert!(open(&cfg, "db", 5432).is_err());
        cfg.host = "h".into();
        cfg.auth = SshAuth::PublicKey;
        assert!(open(&cfg, "db", 5432).is_err());
        cfg.private_key = Some("C:/definitely/not/here".into());
        assert!(open(&cfg, "db", 5432).is_err());
    }
}
