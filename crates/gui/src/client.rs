//! Talking to the background service over its control socket (see the daemon's `control.rs`).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

pub fn request(req: Value) -> Result<Value> {
    let mut s = UnixStream::connect(importer::socket_path()).context("the AcerUltraX service is not running")?;
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

pub fn set_spatial(on: bool) {
    let _ = request(json!({"cmd": "spatial", "on": on}));
}

pub fn set_spatial_power_saving(on: bool) {
    let _ = request(json!({"cmd": "spatial-power-saving", "on": on}));
}

pub fn set_compare(on: bool) {
    let _ = request(json!({"cmd": "compare", "on": on}));
}

/// Every profile (built-in and the user's), with mode and adjustments.
pub fn profiles_full() -> Vec<Value> {
    request(json!({"cmd": "profiles"})).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

pub fn save_profile(p: &Value) -> Result<(), String> {
    request(json!({"cmd": "save-profile", "profile": p})).map(|_| ()).map_err(|e| e.to_string())
}

pub fn delete_profile(name: &str) {
    let _ = request(json!({"cmd": "delete-profile", "name": name}));
}

pub fn mode_defaults(mode: &str) -> Value {
    request(json!({"cmd": "mode-defaults", "mode": mode})).unwrap_or(Value::Null)
}

/// The measured response of a profile on the current output: (Hz, dB) points.
pub fn response(p: &Value) -> Vec<(f64, f64)> {
    request(json!({"cmd": "response", "profile": p}))
        .ok()
        .and_then(|v| v.as_array().map(|a| a.iter().filter_map(|x| Some((x[0].as_f64()?, x[1].as_f64()?))).collect()))
        .unwrap_or_default()
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
