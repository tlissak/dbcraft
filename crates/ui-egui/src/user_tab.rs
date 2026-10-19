//! User / Role designer (Navicat: General, Member Of, Server Privileges, Privileges, SQL Preview).

use dbcraft_core::{DbKind, Grant, UserInfo};
use dbcraft_engine::Session;
use egui::{RichText, Ui};
use egui_phosphor::regular as ph;

use crate::app::Action;
use crate::icons;
use crate::state::{UserSubTab, UserTab};
use crate::theme::Tokens;

pub fn show(ui: &mut Ui, idx: usize, tab: &mut UserTab, session: &Session, users: &[UserInfo], actions: &mut Vec<Action>) {
    let Some(kind) = session.workspace.connection(&tab.connection).ok().map(|c| c.kind) else {
        ui.colored_label(Tokens::error(), "Connection not found.");
        return;
    };
    let Ok(cat) = dbcraft_engine::privilege_catalog(kind) else {
        ui.colored_label(Tokens::error(), format!("User management is not available for {}.", kind.label()));
        return;
    };
    let mysql = matches!(kind, DbKind::Mysql | DbKind::Mariadb);

    egui::Frame::new().fill(Tokens::toolbar_bg()).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.button(RichText::new(format!("{} Save", ph::FLOPPY_DISK)).strong()).clicked() {
                actions.push(Action::UserSave(idx));
            }
            ui.separator();
            for (t, label) in [
                (UserSubTab::General, "General"),
                (UserSubTab::MemberOf, "Member Of"),
                (UserSubTab::ServerPrivileges, if mysql { "Server Privileges" } else { "Role Attributes" }),
                (UserSubTab::Privileges, "Privileges"),
                (UserSubTab::SqlPreview, "SQL Preview"),
            ] {
                if ui.add(egui::Button::selectable(tab.sub == t, label)).clicked() {
                    tab.sub = t;
                    if t == UserSubTab::SqlPreview {
                        actions.push(Action::UserPreview(idx));
                    }
                }
            }
            if tab.saved {
                ui.colored_label(Tokens::success(), format!("{} Saved", icons::CHECK));
            }
        });
    });
    if let Some(e) = &tab.error {
        ui.colored_label(Tokens::error(), format!("{} {e}", icons::X));
    }
    ui.add_space(6.0);
    let u = &mut tab.user;
    egui::ScrollArea::vertical().id_salt(("user", idx)).show(ui, |ui| match tab.sub {
        UserSubTab::General => {
            egui::Grid::new(("ug", idx)).num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(if u.is_role { "Role Name:" } else { "User Name:" });
                ui.add(egui::TextEdit::singleline(&mut u.name).desired_width(260.0));
                ui.end_row();
                if mysql {
                    ui.label("Host:");
                    let mut host = u.host.clone().unwrap_or_else(|| "%".into());
                    if ui.add(egui::TextEdit::singleline(&mut host).desired_width(260.0)).changed() {
                        u.host = Some(host);
                    }
                    ui.end_row();
                    if !u.is_role {
                        ui.label("Plugin:");
                        let mut plugin = u.settings.get("plugin").cloned().unwrap_or_default();
                        egui::ComboBox::from_id_salt(("plugin", idx))
                            .width(260.0)
                            .selected_text(if plugin.is_empty() { "(server default)" } else { &plugin })
                            .show_ui(ui, |ui| {
                                for p in ["", "caching_sha2_password", "mysql_native_password", "sha256_password", "ed25519"] {
                                    ui.selectable_value(&mut plugin, p.to_string(), if p.is_empty() { "(server default)" } else { p });
                                }
                            });
                        u.settings.insert("plugin".into(), plugin);
                        ui.end_row();
                    }
                } else {
                    ui.label("");
                    let mut login = !u.is_role;
                    if ui.checkbox(&mut login, "Can login").changed() {
                        u.is_role = !login;
                    }
                    ui.end_row();
                    ui.label("Connection Limit:");
                    let mut l = u.settings.get("connection_limit").cloned().unwrap_or_default();
                    if ui.add(egui::TextEdit::singleline(&mut l).desired_width(80.0).hint_text("-1")).changed() {
                        u.settings.insert("connection_limit".into(), l);
                    }
                    ui.end_row();
                    ui.label("Expiry Date:");
                    let mut v = u.settings.get("valid_until").cloned().unwrap_or_default();
                    if ui.add(egui::TextEdit::singleline(&mut v).desired_width(200.0).hint_text("YYYY-MM-DD (empty: never)")).changed() {
                        u.settings.insert("valid_until".into(), v);
                    }
                    ui.end_row();
                }
                if !u.is_role || !mysql {
                    ui.label(if tab.original.is_some() { "New Password:" } else { "Password:" });
                    ui.add(egui::TextEdit::singleline(&mut tab.password).password(true).desired_width(260.0).hint_text(if tab.original.is_some() {
                        "(unchanged)"
                    } else {
                        ""
                    }));
                    ui.end_row();
                    ui.label("Confirm Password:");
                    ui.add(egui::TextEdit::singleline(&mut tab.confirm).password(true).desired_width(260.0));
                    ui.end_row();
                    if !tab.password.is_empty() && tab.password != tab.confirm {
                        ui.label("");
                        ui.colored_label(Tokens::error(), "Passwords do not match");
                        ui.end_row();
                    }
                }
                if mysql && kind == DbKind::Mysql && !u.is_role {
                    ui.label("");
                    let mut locked = u.attributes.get("ACCOUNT LOCK").copied().unwrap_or(false);
                    if ui.checkbox(&mut locked, "Lock account").changed() {
                        u.attributes.insert("ACCOUNT LOCK".into(), locked);
                    }
                    ui.end_row();
                }
            });
        }
        UserSubTab::MemberOf => {
            ui.weak("Roles this account is a member of.");
            let roles: Vec<String> = users
                .iter()
                .filter(|r| r.is_role && r.name != u.name)
                .map(|r| {
                    if mysql && r.host.as_deref().is_some_and(|h| !h.is_empty()) {
                        format!("{}@{}", r.name, r.host.as_deref().unwrap_or("%"))
                    } else {
                        r.name.clone()
                    }
                })
                .collect();
            if roles.is_empty() {
                ui.weak("(no roles on this server)");
            }
            for r in roles {
                let mut on = u.member_of.contains(&r);
                if ui.checkbox(&mut on, format!("{}  {r}", ph::USERS_THREE)).changed() {
                    if on {
                        u.member_of.push(r);
                    } else {
                        u.member_of.retain(|m| m != &r);
                    }
                }
            }
        }
        UserSubTab::ServerPrivileges => {
            if mysql {
                ui.weak("Global privileges (ON *.*).");
                let g = global_grant(u);
                privilege_checkboxes(ui, cat.server, &mut g.privileges);
                ui.checkbox(&mut g.grant_option, "Grant Option");
                u.grants.retain(|g| !(g.object == "*.*" && g.privileges.is_empty() && !g.grant_option));
            } else {
                ui.weak("Role attributes.");
                for a in cat.server {
                    let mut on = u.attributes.get(*a).copied().unwrap_or(*a == "INHERIT");
                    if ui.checkbox(&mut on, *a).changed() {
                        u.attributes.insert((*a).to_string(), on);
                    }
                }
            }
        }
        UserSubTab::Privileges => {
            ui.weak(if mysql {
                "Object privileges: `db.*` for a database, `db.table` for a table."
            } else {
                "Object privileges: `DATABASE name`, `SCHEMA name` or `TABLE schema.table`."
            });
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut tab.new_object).desired_width(260.0).hint_text(if mysql {
                    "sakila.*  or  sakila.film"
                } else {
                    "TABLE public.film"
                }));
                let ok = !tab.new_object.trim().is_empty() && tab.new_object.trim() != "*.*";
                if ui.add_enabled(ok, egui::Button::new(format!("{} Add Privilege", icons::PLUS))).clicked() {
                    let obj = tab.new_object.trim().to_string();
                    if !u.grants.iter().any(|g| g.object == obj) {
                        u.grants.push(Grant { object: obj, ..Grant::default() });
                    }
                    tab.new_object.clear();
                }
            });
            let mut remove = None;
            for (i, g) in u.grants.iter_mut().enumerate().filter(|(_, g)| g.object != "*.*") {
                ui.add_space(4.0);
                egui::Frame::new().stroke(egui::Stroke::new(1.0, Tokens::border())).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong(format!("{}  {}", ph::TABLE, g.object));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button(icons::TRASH).on_hover_text("Revoke all on this object").clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                    let list = object_privileges(&cat, mysql, &g.object);
                    privilege_checkboxes(ui, list, &mut g.privileges);
                    ui.checkbox(&mut g.grant_option, "Grant Option");
                });
            }
            if let Some(i) = remove {
                // Keep the object with no privileges so the diff revokes them, then drop it on save.
                if let Some(g) = u.grants.get_mut(i) {
                    g.privileges.clear();
                    g.grant_option = false;
                }
            }
        }
        UserSubTab::SqlPreview => {
            if tab.preview.is_empty() {
                ui.weak("No changes.");
            }
            let mut text = tab.preview.iter().map(|s| format!("{s};")).collect::<Vec<_>>().join("\n");
            ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_width(f32::INFINITY).interactive(true));
        }
    });
}

fn global_grant(u: &mut UserInfo) -> &mut Grant {
    if !u.grants.iter().any(|g| g.object == "*.*") {
        u.grants.push(Grant { object: "*.*".into(), ..Grant::default() });
    }
    let pos = u.grants.iter().position(|g| g.object == "*.*").unwrap_or(0);
    // `pos` exists: pushed above if missing.
    &mut u.grants[pos]
}

fn object_privileges(cat: &dbcraft_engine::PrivilegeCatalog, mysql: bool, object: &str) -> &'static [&'static str] {
    if mysql {
        if object.ends_with(".*") { cat.database } else { cat.table }
    } else {
        match object.split_whitespace().next().unwrap_or("").to_ascii_uppercase().as_str() {
            "DATABASE" => cat.database,
            "SCHEMA" => cat.schema,
            _ => cat.table,
        }
    }
}

fn privilege_checkboxes(ui: &mut Ui, list: &[&str], privs: &mut Vec<String>) {
    let all = privs.iter().any(|p| p == "ALL PRIVILEGES" || p == "ALL");
    if all {
        ui.label("ALL PRIVILEGES");
    }
    egui::Grid::new(ui.next_auto_id()).num_columns(4).show(ui, |ui| {
        for (i, p) in list.iter().enumerate() {
            let mut on = all || privs.iter().any(|x| x.eq_ignore_ascii_case(p));
            if ui.checkbox(&mut on, *p).changed() {
                if on {
                    privs.push((*p).to_string());
                } else {
                    privs.retain(|x| !x.eq_ignore_ascii_case(p));
                }
            }
            if i % 4 == 3 {
                ui.end_row();
            }
        }
    });
}
