//! Talking to the background service over its control socket (see the daemon's `control.rs`).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

pub fn request(req: Value) -> Result<Value> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let mut s = UnixStream::connect(dir.join("acerultrax.sock")).context("the AcerUltraX service is not running")?;
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    writeln!(s, "{req}")?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line)?;
    let reply: Value = serde_json::from_str(&line)?;
    if reply["ok"].as_bool() != Some(true) {
        bail!("{}", reply["error"].as_str().unwrap_or("request failed"));
    }
    Ok(reply["result"].clone())
}

pub fn status() -> Option<Value> {
    request(json!({"cmd": "status"})).ok()
}

pub fn set_profile(name: &str, device: Option<&str>) {
    let _ = request(json!({"cmd": "set-profile", "name": name, "device": device}));
}

pub fn set_enabled(on: bool) {
    let _ = request(json!({"cmd": "enable", "on": on}));
}

pub fn set_compare(on: bool) {
    let _ = request(json!({"cmd": "compare", "on": on}));
}

/// Profile names the service knows.
pub fn profiles() -> Vec<String> {
    request(json!({"cmd": "profiles"}))
        .ok()
        .and_then(|v| v.as_array().map(|a| a.iter().filter_map(|p| p["name"].as_str().map(String::from)).collect()))
        .unwrap_or_default()
}

pub fn headphones() -> Vec<Value> {
    request(json!({"cmd": "headphones"})).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

pub fn set_headphone(id: Option<&str>, device: &str) {
    let _ = request(json!({"cmd": "set-headphone", "id": id, "device": device}));
}

pub fn devices() -> Vec<Value> {
    request(json!({"cmd": "devices"})).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

/// How a profile is shown: "Game1" -> "Game 1".
pub fn label(profile: &str) -> String {
    match profile.strip_prefix("Game") {
        Some(n) if n.len() == 1 => format!("Game {n}"),
        _ => profile.to_string(),
    }
}
