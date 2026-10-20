//! Control socket (`$XDG_RUNTIME_DIR/acerultrax.sock`): one JSON request per line, one JSON reply per line.
//!
//! Requests: `{"cmd":"status"}`, `{"cmd":"profiles"}`, `{"cmd":"devices"}`,
//! `{"cmd":"set-profile","name":"Movie"}` (optional `"device"`: a device's node name; default the current one),
//! `{"cmd":"enable","on":false}` (DTS everywhere), `{"cmd":"compare","on":true}`, `{"cmd":"reload"}`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::router::Router;

pub type Request = (Value, mpsc::Sender<Value>);

pub fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join("acerultrax.sock")
}

/// Accept connections on a background thread and pass requests to the main loop.
pub fn serve(tx: pipewire::channel::Sender<Request>) -> std::io::Result<()> {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut out = conn.try_clone().expect("socket clone");
                for line in BufReader::new(conn).lines().map_while(Result::ok) {
                    let req = serde_json::from_str(&line)
                        .unwrap_or_else(|e| json!({"cmd": "invalid", "error": e.to_string()}));
                    let (rtx, rrx) = mpsc::channel();
                    let reply = if tx.send((req, rtx)).is_ok() {
                        rrx.recv_timeout(Duration::from_secs(5))
                            .unwrap_or_else(|_| json!({"ok": false, "error": "timeout"}))
                    } else {
                        json!({"ok": false, "error": "daemon stopping"})
                    };
                    if writeln!(out, "{reply}").is_err() {
                        break;
                    }
                }
            });
        }
    });
    Ok(())
}

/// Handle one request on the main loop.
pub fn handle(r: &mut Router, req: &Value) -> Value {
    let ok = |v: Value| json!({"ok": true, "result": v});
    let err = |e: String| json!({"ok": false, "error": e});
    match req.get("cmd").and_then(Value::as_str).unwrap_or("") {
        "status" => ok(serde_json::to_value(r.status()).unwrap()),
        "profiles" => ok(serde_json::to_value(r.config.all_profiles()).unwrap()),
        "devices" => ok(serde_json::to_value(r.devices()).unwrap()),
        "set-profile" => match req.get("name").and_then(Value::as_str) {
            Some(n) => {
                r.set_profile(n, req.get("device").and_then(Value::as_str)).map_or_else(err, |_| ok(Value::Null))
            }
            None => err("missing name".into()),
        },
        "enable" => {
            r.set_enabled(req.get("on").and_then(Value::as_bool).unwrap_or(true));
            ok(Value::Null)
        }
        "compare" => {
            r.set_compare(req.get("on").and_then(Value::as_bool).unwrap_or(false));
            ok(Value::Null)
        }
        "reload" => {
            r.reload_config();
            ok(Value::Null)
        }
        other => err(format!("unknown command {other:?}")),
    }
}
