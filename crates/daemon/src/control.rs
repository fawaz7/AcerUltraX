//! Control socket (`$XDG_RUNTIME_DIR/acerultrax.sock`): one JSON request per line, one JSON reply per line.
//!
//! Requests: `{"cmd":"status"}`, `{"cmd":"profiles"}`, `{"cmd":"devices"}`,
//! `{"cmd":"set-profile","name":"Movie"}` (optional `"device"`: a device's node name; default the current one),
//! `{"cmd":"headphones"}` (models), `{"cmd":"set-headphone","id":"<model id>"|"default"|null,"device":...}`,
//! `{"cmd":"save-profile","profile":{"name":..,"mode":..,"tweaks":{..}}}`, `{"cmd":"delete-profile","name":..}`,
//! `{"cmd":"mode-defaults","mode":"movie"}` (factory values of the adjustable controls),
//! `{"cmd":"response","profile":{..}}` (measured response curve: [[Hz, dB], ...]),
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

/// Start a response-curve measurement (`{"cmd":"response","profile":{...}}` or `"name":...`) on a worker thread;
/// it replies by itself. Returns false for other requests.
pub fn start_response(r: &Router, req: &Value, reply: &mpsc::Sender<Value>) -> bool {
    if req.get("cmd").and_then(Value::as_str) != Some("response") {
        return false;
    }
    let profile = match (req.get("profile"), req.get("name").and_then(Value::as_str)) {
        (Some(p), _) => serde_json::from_value(p.clone()).map_err(|e| format!("bad profile: {e}")),
        (None, Some(n)) => r.config.profile(n).ok_or(format!("no profile named {n:?}")),
        _ => Err("missing profile".into()),
    };
    let job = profile.and_then(|p| r.response_job(&p));
    let (lib, reply) = (r.lib(), reply.clone());
    let run = move || {
        let v = match job {
            Err(e) => json!({"ok": false, "error": e}),
            Ok(None) => json!({"ok": true, "result": [[20.0, 0.0], [20000.0, 0.0]]}),
            Ok(Some((t, rate, eq))) => match audio_graph::response(lib, &t, rate, eq) {
                Ok(points) => json!({"ok": true, "result": points}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            },
        };
        let _ = reply.send(v);
    };
    // DTS code needs a large stack.
    if std::thread::Builder::new().name("dts-response".into()).stack_size(8 << 20).spawn(run).is_err() {
        log::error!("cannot start a worker thread");
    }
    true
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
        "save-profile" => match req.get("profile").cloned().map(serde_json::from_value::<crate::config::Profile>) {
            Some(Ok(p)) => r.save_profile(p).map_or_else(err, |_| ok(Value::Null)),
            Some(Err(e)) => err(format!("bad profile: {e}")),
            None => err("missing profile".into()),
        },
        "mode-defaults" => match req.get("mode").and_then(Value::as_str) {
            Some(m) => r.mode_defaults(m).map_or_else(err, |t| ok(serde_json::to_value(t).unwrap())),
            None => err("missing mode".into()),
        },
        "delete-profile" => match req.get("name").and_then(Value::as_str) {
            Some(n) => r.delete_profile(n).map_or_else(err, |_| ok(Value::Null)),
            None => err("missing name".into()),
        },
        "headphones" => ok(serde_json::to_value(r.headphones()).unwrap()),
        "set-headphone" => {
            let id = req.get("id").and_then(Value::as_str);
            r.set_headphone(id, req.get("device").and_then(Value::as_str)).map_or_else(err, |_| ok(Value::Null))
        }
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
