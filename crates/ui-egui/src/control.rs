//! Control channel (ROADMAP 1.2): drive and see the running app from outside, for agents and
//! scripts. `dbcraft --control` listens on 127.0.0.1 (never another interface), writes the address
//! and a random token to `control.json` next to the workspace, and answers one JSON request per
//! line:
//!
//! ```text
//! {"token":"…","method":"ping"}
//! {"token":"…","method":"dispatch","id":"connection.open","params":{"id":"pg"}}
//! {"token":"…","method":"action","action":{"type":"open_table","connection":"pg","database":"db","table":"film"}}
//! {"token":"…","method":"ui","id":"ui.newQuery"}
//! {"token":"…","method":"input","events":[{"text":"select 1"},{"key":"R","ctrl":true},{"click":[400,300]}]}
//! {"token":"…","method":"wait_idle","timeout_ms":10000}
//! {"token":"…","method":"state"}
//! {"token":"…","method":"snapshot","path":"C:/tmp/dbcraft.png"}
//! ```
//!
//! Every reply is `{"ok":true,…}` or `{"ok":false,"error":"…"}`. Requests are handled on the UI
//! thread between frames, so they see and change exactly what the user sees. Passwords never leave
//! the app (`state` redacts them; `connection.list` never had them).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::Duration;

use serde_json::{Value as Json, json};

/// A request line is capped (1 MiB); longer input closes the connection.
const MAX_LINE: usize = 1 << 20;
/// How long a client waits for the UI thread to answer.
const REPLY_TIMEOUT: Duration = Duration::from_secs(300);

/// One authenticated request, waiting for the UI thread.
pub struct Request {
    pub body: Json,
    reply: SyncSender<Json>,
}

impl Request {
    pub fn method(&self) -> &str {
        self.body.get("method").and_then(Json::as_str).unwrap_or("")
    }

    pub fn reply(self, v: Json) {
        let _ = self.reply.send(v);
    }

    pub fn ok(self, mut extra: Json) {
        if let Json::Object(o) = &mut extra {
            o.insert("ok".into(), Json::Bool(true));
            self.reply(extra);
        } else {
            self.reply(json!({ "ok": true, "result": extra }));
        }
    }

    pub fn err(self, e: impl std::fmt::Display) {
        self.reply(json!({ "ok": false, "error": e.to_string() }));
    }
}

/// The running listener. Dropping it doesn't stop the thread; the process exit does.
pub struct ControlServer {
    pub addr: SocketAddr,
    pub token: String,
    /// Where the address and token were written (removed on exit).
    pub info_file: Option<PathBuf>,
    rx: Receiver<Request>,
}

impl ControlServer {
    /// Listen on 127.0.0.1:`port` (0: any free port). Requests wake `ctx`.
    pub fn start(ctx: egui::Context, port: u16, info_dir: Option<PathBuf>) -> std::io::Result<ControlServer> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let addr = listener.local_addr()?;
        let token = new_token()?;
        let (tx, rx) = sync_channel::<Request>(64);
        let expected = token.clone();
        std::thread::Builder::new().name("dbcraft-control".into()).spawn(move || {
            for stream in listener.incoming().flatten() {
                let (tx, expected, ctx) = (tx.clone(), expected.clone(), ctx.clone());
                let _ = std::thread::Builder::new().name("dbcraft-control-conn".into()).spawn(move || serve(stream, &tx, &expected, &ctx));
            }
        })?;
        let info_file = match info_dir {
            Some(dir) => {
                let path = dir.join("control.json");
                std::fs::create_dir_all(&dir)?;
                std::fs::write(&path, json!({ "addr": addr.to_string(), "token": token, "pid": std::process::id() }).to_string())?;
                Some(path)
            }
            None => None,
        };
        Ok(ControlServer { addr, token, info_file, rx })
    }

    /// Requests that arrived since the last frame.
    pub fn pending(&self) -> Vec<Request> {
        self.rx.try_iter().collect()
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        if let Some(p) = &self.info_file {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn new_token() -> std::io::Result<String> {
    let mut buf = [0u8; 24];
    getrandom::fill(&mut buf).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Same-length comparison that doesn't stop at the first differing byte.
fn token_matches(given: &str, expected: &str) -> bool {
    given.len() == expected.len() && given.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

fn serve(stream: TcpStream, tx: &SyncSender<Request>, expected: &str, ctx: &egui::Context) {
    let Ok(write_half) = stream.try_clone() else { return };
    let mut out = std::io::BufWriter::new(write_half);
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        match (&mut reader).take(MAX_LINE as u64 + 1).read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(n) if n > MAX_LINE => {
                let _ = writeln!(out, "{}", json!({ "ok": false, "error": "request too large" }));
                let _ = out.flush();
                return;
            }
            Ok(_) => {}
        }
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Json>(&line) {
            Err(e) => json!({ "ok": false, "error": format!("bad JSON: {e}") }),
            Ok(body) if !body.get("token").and_then(Json::as_str).is_some_and(|t| token_matches(t, expected)) => {
                json!({ "ok": false, "error": "bad or missing token (see control.json)" })
            }
            Ok(body) => {
                let (rtx, rrx) = sync_channel(1);
                if tx.send(Request { body, reply: rtx }).is_err() {
                    return; // the app is gone
                }
                ctx.request_repaint();
                rrx.recv_timeout(REPLY_TIMEOUT).unwrap_or_else(|_| json!({ "ok": false, "error": "the app did not answer in time" }))
            }
        };
        if writeln!(out, "{reply}").and_then(|()| out.flush()).is_err() {
            return;
        }
    }
}

/// `state` replies never carry secrets: any string under a password-like key is masked.
pub fn redact(v: &mut Json) {
    match v {
        Json::Object(o) => {
            for (k, val) in o.iter_mut() {
                let key = k.to_ascii_lowercase();
                let secret = key.contains("password") || key.contains("passphrase") || key == "confirm" || key == "token";
                if secret && val.as_str().is_some_and(|s| !s.is_empty()) {
                    *val = Json::String("***".into());
                } else {
                    redact(val);
                }
            }
        }
        Json::Array(a) => a.iter_mut().for_each(redact),
        _ => {}
    }
}

/// Input events from an `input` request: `{"key":"R","ctrl":true,"shift":false}`, `{"text":"…"}`,
/// `{"click":[x,y]}` (logical points), `{"copy":true}`.
pub fn parse_events(spec: &Json) -> Result<Vec<egui::Event>, String> {
    let list = spec.as_array().ok_or("`events` must be an array")?;
    if list.len() > 1000 {
        return Err("at most 1000 events per request".into());
    }
    let mut out = Vec::new();
    for e in list {
        let flag = |k: &str| e.get(k).and_then(Json::as_bool).unwrap_or(false);
        let modifiers = egui::Modifiers { alt: flag("alt"), ctrl: flag("ctrl"), shift: flag("shift"), command: flag("ctrl"), mac_cmd: false };
        if let Some(name) = e.get("key").and_then(Json::as_str) {
            let key = egui::Key::from_name(name).ok_or_else(|| format!("unknown key `{name}` (egui key names: A, Enter, F5, ArrowDown…)"))?;
            for pressed in [true, false] {
                out.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
            }
        } else if let Some(text) = e.get("text").and_then(Json::as_str) {
            out.push(egui::Event::Text(text.to_string()));
        } else if let Some([x, y]) = e.get("click").and_then(Json::as_array).map(Vec::as_slice) {
            let (Some(x), Some(y)) = (x.as_f64(), y.as_f64()) else { return Err("`click` needs [x, y] numbers".into()) };
            let pos = egui::pos2(x as f32, y as f32);
            out.push(egui::Event::PointerMoved(pos));
            for pressed in [true, false] {
                out.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers });
            }
        } else if flag("copy") {
            out.push(egui::Event::Copy);
        } else {
            return Err(format!("unknown event {e}"));
        }
    }
    Ok(out)
}

/// Encode a screenshot as PNG.
pub fn write_png(image: &egui::ColorImage, path: &std::path::Path) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let [w, h] = image.size;
    let (w, h) = (u32::try_from(w).map_err(|e| e.to_string())?, u32::try_from(h).map_err(|e| e.to_string())?);
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    writer.write_image_data(&bytes).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secrets_anywhere() {
        let mut v = json!({"connection_form": {"password": "pw", "ssh_password": "x", "host": "h"}, "tabs": [{"user": {"confirm": "c", "name": "n"}}], "empty": {"password": ""}});
        redact(&mut v);
        assert_eq!(v["connection_form"]["password"], "***");
        assert_eq!(v["connection_form"]["ssh_password"], "***");
        assert_eq!(v["connection_form"]["host"], "h");
        assert_eq!(v["tabs"][0]["user"]["confirm"], "***");
        assert_eq!(v["tabs"][0]["user"]["name"], "n");
        assert_eq!(v["empty"]["password"], "", "nothing to hide");
    }

    #[test]
    fn parses_input_events_and_rejects_junk() {
        let ev = parse_events(&json!([{"key": "R", "ctrl": true}, {"text": "hi"}, {"click": [10, 20]}, {"copy": true}])).unwrap();
        assert_eq!(ev.len(), 2 + 1 + 3 + 1);
        assert!(matches!(ev[0], egui::Event::Key { key: egui::Key::R, pressed: true, modifiers, .. } if modifiers.command));
        for bad in [json!({}), json!([{"key": "NoSuchKey"}]), json!([{"click": [1]}]), json!([{"click": ["a", 2]}]), json!([{"wat": 1}])] {
            assert!(parse_events(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn token_comparison() {
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("abd", "abc"));
        assert!(!token_matches("ab", "abc"));
        assert!(!token_matches("", "abc"));
    }
}
