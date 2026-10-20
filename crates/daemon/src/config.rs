//! User settings (`~/.config/acerultrax/config.toml`) and daemon state (`~/.local/state/acerultrax/state.toml`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use dts_eagle::{Mode, Output};
use serde::{Deserialize, Serialize};

/// What kind of output a sink is; decides the default profile and the DTS tuning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Speakers,
    Headphones,
    Bluetooth,
    Other,
}

impl Kind {
    /// DTS tuning for this kind of output: the laptop speakers get the speaker tuning, everything you wear (and
    /// anything else) the headphone tuning.
    pub fn tuning(self) -> Output {
        if self == Kind::Speakers { Output::Speakers } else { Output::Headphones }
    }
}

/// A sound profile. For now a DTS mode, or off.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub name: String,
    /// DTS mode (`music`, `movie`, ...), or `off` for unprocessed sound.
    pub mode: String,
}

impl Profile {
    pub fn dts_mode(&self) -> Option<Mode> {
        self.mode.parse().ok().filter(|m| *m != Mode::Off)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Profile per kind of output, used for devices without their own choice.
    pub defaults: BTreeMap<String, String>,
    /// Profile chosen per output device (PipeWire node name).
    pub devices: BTreeMap<String, String>,
    /// User-made profiles (the built-in ones are always available).
    pub profiles: Vec<Profile>,
}

impl Default for Config {
    fn default() -> Config {
        let defaults = [("speakers", "Music"), ("headphones", "Music"), ("bluetooth", "Music"), ("other", "Off")];
        Config {
            defaults: defaults.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            devices: BTreeMap::new(),
            profiles: Vec::new(),
        }
    }
}

pub fn builtin_profiles() -> Vec<Profile> {
    let mut v: Vec<Profile> = Mode::ALL
        .iter()
        .filter(|m| **m != Mode::Off)
        .map(|m| Profile { name: m.to_string(), mode: m.key().to_lowercase() })
        .collect();
    v.push(Profile { name: "Off".into(), mode: "off".into() });
    v
}

fn kind_key(k: Kind) -> &'static str {
    match k {
        Kind::Speakers => "speakers",
        Kind::Headphones => "headphones",
        Kind::Bluetooth => "bluetooth",
        Kind::Other => "other",
    }
}

impl Config {
    pub fn path() -> PathBuf {
        xdg("XDG_CONFIG_HOME", ".config").join("acerultrax/config.toml")
    }

    pub fn load() -> Config {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| toml::from_str(&t).map_err(|e| log::warn!("config: {e}")).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        save(&Self::path(), &toml::to_string_pretty(self).unwrap());
    }

    pub fn all_profiles(&self) -> Vec<Profile> {
        let mut v = builtin_profiles();
        v.extend(self.profiles.iter().filter(|p| !v.iter().any(|b| b.name == p.name)).cloned().collect::<Vec<_>>());
        v
    }

    pub fn profile(&self, name: &str) -> Option<Profile> {
        self.all_profiles().into_iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// The profile for a device: its own choice, else the default for its kind, else Off.
    pub fn profile_for(&self, device: &str, kind: Kind) -> Profile {
        self.devices
            .get(device)
            .or_else(|| self.defaults.get(kind_key(kind)))
            .and_then(|n| self.profile(n))
            .unwrap_or_else(|| Profile { name: "Off".into(), mode: "off".into() })
    }
}

/// Which output the user chose most recently comes first (as WirePlumber remembers default devices).
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub history: Vec<String>,
}

impl State {
    fn path() -> PathBuf {
        xdg("XDG_STATE_HOME", ".local/state").join("acerultrax/state.toml")
    }

    pub fn load() -> State {
        std::fs::read_to_string(Self::path()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        save(&Self::path(), &toml::to_string_pretty(self).unwrap());
    }

    pub fn chose(&mut self, device: &str) {
        self.history.retain(|d| d != device);
        self.history.insert(0, device.to_string());
        self.history.truncate(32);
    }
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback))
}

fn save(path: &std::path::Path, text: &str) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    // Write then rename, so a crash never leaves a half-written file.
    let tmp = path.with_extension("tmp");
    if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, path)) {
        log::warn!("cannot save {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_lookup() {
        let mut c = Config::default();
        assert_eq!(c.profile_for("bluez_output.x", Kind::Bluetooth).name, "Music");
        assert_eq!(c.profile_for("hdmi", Kind::Other).dts_mode(), None);
        c.devices.insert("spk".into(), "movie".into());
        assert_eq!(c.profile_for("spk", Kind::Speakers).dts_mode(), Some(Mode::Movie));
        assert_eq!(c.profile("custom audio").unwrap().dts_mode(), Some(Mode::CustomAudio));
        let mut s = State::default();
        s.chose("a");
        s.chose("b");
        s.chose("a");
        assert_eq!(s.history, ["a", "b"]);
    }
}
