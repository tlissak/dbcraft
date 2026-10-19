//! New / Edit Connection dialog (Navicat: General, Advanced, Databases, SSL, SSH tabs).

use std::sync::{Arc, Mutex};

use dbcraft_core::{ConnectionConfig, DbKind, SshAuth};
use egui::{RichText, Ui};
use serde_json::{Value as Json, json};

use crate::icons;
use crate::state::{ConnTab, ConnectionForm, TestSlot};
use crate::theme::Tokens;

pub enum Outcome {
    Open,
    Cancel,
    Save(Json),
}

impl ConnectionForm {
    pub fn new(kind: DbKind) -> Self {
        let mut f = ConnectionForm { save_password: true, ssh_port: "22".into(), ssh_auth: "password".into(), ..ConnectionForm::default() };
        f.set_kind(kind);
        f
    }

    pub fn edit(cfg: &ConnectionConfig) -> Self {
        let ssh = cfg.ssh.clone();
        ConnectionForm {
            id: Some(cfg.id.clone()),
            tab: ConnTab::General,
            kind: cfg.kind.id().into(),
            name: cfg.name.clone(),
            host: cfg.host.clone(),
            port: cfg.port.map(|p| p.to_string()).unwrap_or_default(),
            user: cfg.user.clone(),
            password: cfg.password.clone().unwrap_or_default(),
            save_password: cfg.password.is_some(),
            database: cfg.database.clone(),
            ssh_enabled: ssh.is_some(),
            ssh_host: ssh.as_ref().map(|s| s.host.clone()).unwrap_or_default(),
            ssh_port: ssh.as_ref().map_or(22, |s| s.port).to_string(),
            ssh_user: ssh.as_ref().map(|s| s.user.clone()).unwrap_or_default(),
            ssh_auth: if ssh.as_ref().is_some_and(|s| s.auth == SshAuth::PublicKey) { "publickey" } else { "password" }.into(),
            ssh_password: ssh.as_ref().and_then(|s| s.password.clone()).unwrap_or_default(),
            ssh_key: ssh.as_ref().and_then(|s| s.private_key.clone()).unwrap_or_default(),
            test: None,
            test_result: None,
        }
    }

    fn set_kind(&mut self, kind: DbKind) {
        self.kind = kind.id().into();
        self.port = kind.default_port().map(|p| p.to_string()).unwrap_or_default();
        if self.host.is_empty() && kind != DbKind::Sqlite {
            self.host = "localhost".into();
        }
        self.user = match kind {
            DbKind::Postgres => "postgres",
            DbKind::Mysql | DbKind::Mariadb => "root",
            DbKind::Sqlserver => "sa",
            _ => "",
        }
        .into();
        if self.name.is_empty() || DbKind::ALL.iter().any(|k| k.label() == self.name) {
            self.name = kind.label().into();
        }
        if kind == DbKind::Sqlite && self.tab == ConnTab::Ssh {
            self.tab = ConnTab::General;
        }
    }

    fn kind(&self) -> DbKind {
        DbKind::parse(&self.kind).unwrap_or(DbKind::Mysql)
    }

    /// Params for `connection.save` / `connection.test`.
    pub fn params(&self, for_test: bool) -> Json {
        let ssh = (self.ssh_enabled && self.kind() != DbKind::Sqlite).then(|| {
            json!({
                "host": self.ssh_host.trim(), "port": self.ssh_port.trim().parse::<u64>().ok(), "user": self.ssh_user.trim(),
                "auth": self.ssh_auth,
                "password": (!self.ssh_password.is_empty()).then(|| self.ssh_password.clone()),
                "private_key": (self.ssh_auth == "publickey" && !self.ssh_key.is_empty()).then(|| self.ssh_key.clone()),
            })
        });
        let keep_pw = !self.password.is_empty() && (self.save_password || for_test);
        json!({
            "id": self.id, "kind": self.kind, "name": self.name, "host": self.host.trim(),
            "port": self.port.trim().parse::<u64>().ok(), "user": self.user,
            "password": keep_pw.then(|| self.password.clone()), "database": self.database, "ssh": ssh,
        })
    }

    fn start_test(&mut self, ctx: &egui::Context) {
        let slot: TestSlot = Arc::new(Mutex::new(None));
        let params = self.params(true);
        let (out, ctx) = (slot.clone(), ctx.clone());
        // A throwaway session on a worker thread: the dialog stays responsive while connecting.
        std::thread::spawn(move || {
            let r = dbcraft_engine::Session::new().dispatch("connection.test", &params);
            let msg = r.map(|v| v["version"].as_str().unwrap_or("").to_string()).map_err(|e| e.to_string());
            *out.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(msg);
            ctx.request_repaint();
        });
        self.test = Some(slot);
        self.test_result = None;
    }

    fn poll_test(&mut self) {
        let done = self.test.as_ref().and_then(|s| s.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take());
        if let Some(r) = done {
            self.test_result = Some(r);
            self.test = None;
        }
    }
}

pub fn show(ctx: &egui::Context, form: &mut ConnectionForm) -> Outcome {
    form.poll_test();
    let mut outcome = Outcome::Open;
    let mut open = true;
    let kind = form.kind();
    let title = if form.id.is_some() { format!("Edit Connection - {}", kind.label()) } else { format!("New Connection - {}", kind.label()) };
    egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(520.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            // Tabs
            ui.horizontal(|ui| {
                for (t, label) in [
                    (ConnTab::General, "General"),
                    (ConnTab::Advanced, "Advanced"),
                    (ConnTab::Databases, "Databases"),
                    (ConnTab::Ssl, "SSL"),
                    (ConnTab::Ssh, "SSH"),
                ] {
                    let enabled = !(t == ConnTab::Ssh && kind == DbKind::Sqlite);
                    let mut text = RichText::new(label);
                    if t == ConnTab::Ssh && form.ssh_enabled {
                        text = RichText::new(format!("{label} {}", icons::LOCK_KEY));
                    }
                    if ui.add_enabled(enabled, egui::Button::selectable(form.tab == t, text)).clicked() {
                        form.tab = t;
                    }
                }
            });
            ui.separator();
            ui.set_min_height(300.0);
            match form.tab {
                ConnTab::General => general(ui, form),
                ConnTab::Ssh => ssh(ui, form),
                ConnTab::Advanced | ConnTab::Databases | ConnTab::Ssl => {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| ui.weak("Not implemented yet (see ROADMAP.md)."));
                }
            }
            ui.add_space(8.0);
            ui.separator();
            // Test result
            if form.test.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Connecting…");
                });
            } else if let Some(r) = &form.test_result {
                match r {
                    Ok(v) => ui.colored_label(Tokens::success(), format!("{} Connection Successful  ({v})", icons::CHECK)),
                    Err(e) => ui.colored_label(Tokens::error(), format!("{} {e}", icons::X)),
                };
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(form.test.is_none(), egui::Button::new(format!("{} Test Connection", icons::PLUG))).clicked() {
                    form.start_test(ctx);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        outcome = Outcome::Cancel;
                    }
                    if ui.add(egui::Button::new(RichText::new("  OK  ").strong())).clicked() {
                        outcome = Outcome::Save(form.params(false));
                    }
                });
            });
        });
    if !open { Outcome::Cancel } else { outcome }
}

fn general(ui: &mut Ui, form: &mut ConnectionForm) {
    let kind = form.kind();
    egui::Grid::new("conn_general").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        ui.label("Connection Type:");
        let mut picked = None;
        egui::ComboBox::from_id_salt("kind").width(220.0).selected_text(format!("{}  {}", icons::connection(kind, true).text(), kind.label())).show_ui(
            ui,
            |ui| {
                for k in DbKind::ALL {
                    let supported = dbcraft_engine::supported_kinds().contains(&k);
                    let label = if supported { k.label().to_string() } else { format!("{} (coming soon)", k.label()) };
                    if ui.add_enabled(supported, egui::Button::selectable(k == kind, label)).clicked() {
                        picked = Some(k);
                    }
                }
            },
        );
        if let Some(k) = picked {
            form.set_kind(k);
        }
        ui.end_row();
        ui.label("Connection Name:");
        ui.add(egui::TextEdit::singleline(&mut form.name).desired_width(300.0));
        ui.end_row();
        if kind == DbKind::Sqlite {
            ui.label("Database File:");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut form.database).desired_width(260.0).hint_text("C:\\data\\app.db"));
                if ui.button("…").clicked()
                    && let Some(p) = rfd::FileDialog::new().add_filter("SQLite", &["db", "sqlite", "sqlite3", "db3"]).pick_file()
                {
                    form.database = p.to_string_lossy().into_owned();
                }
            });
            ui.end_row();
        } else {
            ui.label("Host:");
            ui.add(egui::TextEdit::singleline(&mut form.host).desired_width(300.0));
            ui.end_row();
            ui.label("Port:");
            ui.add(egui::TextEdit::singleline(&mut form.port).desired_width(80.0));
            ui.end_row();
            ui.label(if kind == DbKind::Postgres { "Initial Database:" } else { "Default Database:" });
            ui.add(egui::TextEdit::singleline(&mut form.database).desired_width(300.0).hint_text("(optional)"));
            ui.end_row();
            ui.label("User Name:");
            ui.add(egui::TextEdit::singleline(&mut form.user).desired_width(300.0));
            ui.end_row();
            ui.label("Password:");
            ui.add(egui::TextEdit::singleline(&mut form.password).password(true).desired_width(300.0));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut form.save_password, "Save password");
            ui.end_row();
        }
    });
    if form.ssh_enabled && kind != DbKind::Sqlite {
        ui.add_space(6.0);
        ui.weak(format!("{} Connecting through SSH {}: Host is resolved from the SSH server.", icons::LOCK_KEY, form.ssh_host));
    }
}

fn ssh(ui: &mut Ui, form: &mut ConnectionForm) {
    ui.checkbox(&mut form.ssh_enabled, "Use SSH Tunnel");
    ui.add_space(6.0);
    ui.add_enabled_ui(form.ssh_enabled, |ui| {
        egui::Grid::new("conn_ssh").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Host:");
            ui.add(egui::TextEdit::singleline(&mut form.ssh_host).desired_width(300.0));
            ui.end_row();
            ui.label("Port:");
            ui.add(egui::TextEdit::singleline(&mut form.ssh_port).desired_width(80.0));
            ui.end_row();
            ui.label("User Name:");
            ui.add(egui::TextEdit::singleline(&mut form.ssh_user).desired_width(300.0));
            ui.end_row();
            ui.label("Authentication Method:");
            egui::ComboBox::from_id_salt("ssh_auth").selected_text(if form.ssh_auth == "publickey" { "Public Key" } else { "Password" }).show_ui(ui, |ui| {
                ui.selectable_value(&mut form.ssh_auth, "password".to_string(), "Password");
                ui.selectable_value(&mut form.ssh_auth, "publickey".to_string(), "Public Key");
            });
            ui.end_row();
            if form.ssh_auth == "publickey" {
                ui.label("Private Key:");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut form.ssh_key).desired_width(260.0).hint_text("~/.ssh/id_ed25519"));
                    if ui.button("…").clicked()
                        && let Some(p) = rfd::FileDialog::new().pick_file()
                    {
                        form.ssh_key = p.to_string_lossy().into_owned();
                    }
                });
                ui.end_row();
                ui.label("Passphrase:");
            } else {
                ui.label("Password:");
            }
            ui.add(egui::TextEdit::singleline(&mut form.ssh_password).password(true).desired_width(300.0));
            ui.end_row();
        });
    });
    ui.add_space(10.0);
    ui.weak("The database Host on the General tab is resolved on the SSH server (e.g. localhost or a private IP).");
}
