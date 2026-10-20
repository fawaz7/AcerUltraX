//! Device volume, through WirePlumber's `wpctl` (it knows whether a device has a hardware mixer, Bluetooth
//! absolute volume, ... and remembers the volume per device).

use std::process::Command;
use std::sync::{Condvar, Mutex, OnceLock};

use pipewire::spa::pod::deserialize::PodDeserializer;
use pipewire::spa::pod::{Pod, Value, ValueArray};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Volume {
    /// Linear, 1.0 = 100 %.
    pub level: f32,
    pub muted: bool,
}

pub fn get(node: u32) -> Option<Volume> {
    let out = Command::new("wpctl").args(["get-volume", &node.to_string()]).output().ok()?;
    parse_wpctl(&String::from_utf8_lossy(&out.stdout))
}

fn parse_wpctl(s: &str) -> Option<Volume> {
    let level = s.strip_prefix("Volume:")?.split_whitespace().next()?.parse().ok()?;
    Some(Volume { level, muted: s.contains("[MUTED]") })
}

pub fn set(node: u32, v: Volume) {
    let id = node.to_string();
    let level = format!("{:.4}", v.level.max(0.0));
    let ok = Command::new("wpctl").args(["set-volume", &id, &level]).status().is_ok_and(|s| s.success())
        && Command::new("wpctl")
            .args(["set-mute", &id, if v.muted { "1" } else { "0" }])
            .status()
            .is_ok_and(|s| s.success());
    if !ok {
        log::warn!("wpctl could not set the volume of node {node}");
    }
}

/// Set a volume from a background thread; while a slider moves, only the latest value is applied.
pub fn set_async(node: u32, v: Volume) {
    static LATEST: Mutex<Option<(u32, Volume)>> = Mutex::new(None);
    static WAKE: Condvar = Condvar::new();
    static WORKER: OnceLock<()> = OnceLock::new();
    WORKER.get_or_init(|| {
        std::thread::spawn(|| {
            loop {
                let mut g = LATEST.lock().unwrap();
                while g.is_none() {
                    g = WAKE.wait(g).unwrap();
                }
                let (n, v) = g.take().unwrap();
                drop(g);
                set(n, v);
            }
        });
    });
    *LATEST.lock().unwrap() = Some((node, v));
    WAKE.notify_one();
}

/// Volume and mute from a node's Props param.
pub fn parse_props(pod: &Pod) -> Option<Volume> {
    let (_, value) = PodDeserializer::deserialize_any_from(pod.as_bytes()).ok()?;
    let Value::Object(obj) = value else { return None };
    let (mut level, mut muted) = (None, false);
    for p in obj.properties {
        match (p.key, p.value) {
            (libspa_sys::SPA_PROP_channelVolumes, Value::ValueArray(ValueArray::Float(v))) if !v.is_empty() => {
                level = Some(v.iter().sum::<f32>() / v.len() as f32);
            }
            (libspa_sys::SPA_PROP_mute, Value::Bool(m)) => muted = m,
            _ => {}
        }
    }
    // wpctl works in the cubic scale the desktop shows; node Props carry the linear gain.
    level.map(|l| Volume { level: l.cbrt(), muted })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wpctl_output() {
        assert_eq!(parse_wpctl("Volume: 0.40\n"), Some(Volume { level: 0.4, muted: false }));
        assert_eq!(parse_wpctl("Volume: 1.00 [MUTED]\n"), Some(Volume { level: 1.0, muted: true }));
        assert_eq!(parse_wpctl("garbage"), None);
    }
}
