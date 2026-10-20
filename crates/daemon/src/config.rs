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
    /// Wired headphones (3.5 mm jack, USB headsets).
    Headphones,
    /// Bluetooth headphones and headsets.
    Bluetooth,
    #[serde(rename = "bluetooth-speaker")]
    BluetoothSpeaker,
    Other,
}

impl Kind {
    /// DTS tuning for this kind of output: Acer's tunings for the laptop speakers, for headphones (anything you
    /// wear), and for external speakers (everything else).
    pub fn tuning(self) -> Output {
        match self {
            Kind::Speakers => Output::Speakers,
            Kind::Headphones | Kind::Bluetooth => Output::Headphones,
            Kind::BluetoothSpeaker | Kind::Other => Output::ExternalSpeakers,
        }
    }
}

/// A sound profile: a DTS mode, optionally with the adjustments Windows' "Custom Audio" offers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub name: String,
    /// DTS mode (`music`, `movie`, ...) it starts from, or `off` for unprocessed sound.
    pub mode: String,
    #[serde(default, skip_serializing_if = "Tweaks::is_empty")]
    pub tweaks: Tweaks,
}

/// Changes to a mode: the equaliser (AcerUltraX's, after DTS: the engine's own equaliser has no effect in this
/// version) and the DTS engine's own controls. `None` keeps the mode's factory value.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(default)]
pub struct Tweaks {
    /// 10-band equaliser gains in tenths of a dB (-150..=150), 31 Hz to 16 kHz.
    pub eq: Option<[i32; 10]>,
    /// Bass enhancement (DTS TruBass), 0 = off, 1..=100.
    pub bass: Option<i32>,
    /// Dialogue clarity, 0 = off, 1..=100.
    pub dialog: Option<i32>,
    /// Virtual surround on/off.
    pub surround: Option<bool>,
    /// Surround room: entertainment, game, sports, entertainment-2022.
    pub room: Option<String>,
    /// Loudness control (evens out loud and quiet parts).
    pub loudness: Option<bool>,
}

/// Engine names of the surround rooms.
pub const ROOMS: [(&str, &str); 4] = [
    ("entertainment", "Eagle-I3DA Room Entertainment"),
    ("game", "Eagle-I3DA Room Game"),
    ("sports", "Eagle-I3DA Room Sports"),
    ("entertainment-2022", "Eagle-I3DA Room Entertainment 2022"),
];

impl Tweaks {
    pub fn is_empty(&self) -> bool {
        *self == Tweaks::default()
    }

    /// Apply the DTS controls to a mode's factory settings (later settings win, so these go last). The equaliser
    /// is not an engine setting.
    pub fn apply(&self, t: &mut dts_eagle::ModeTuning) {
        use dts_eagle::oem::{Setting, Stage};
        let mut set = |stage, name: &str, value: String| t.settings.push(Setting { stage, name: name.into(), value });
        let level = |v: i32| v.clamp(0, 100);
        for (v, stage, enable, name) in [
            (self.bass, Stage::Mfx, "Eagle-TBHDX Enable", "Eagle-TBHDX Bass Level"),
            (self.dialog, Stage::Sfx, "Eagle-DE Enable", "Eagle-DE Level"),
        ] {
            if let Some(v) = v {
                set(stage, enable, ((level(v) > 0) as i32).to_string());
                if v > 0 {
                    set(stage, name, level(v).to_string());
                }
            }
        }
        if let Some(on) = self.surround {
            set(Stage::Sfx, "Eagle-I3DA Enable", (on as i32).to_string());
        }
        if let Some(room) = self.room.as_deref().and_then(|r| ROOMS.iter().find(|(k, _)| *k == r)) {
            set(Stage::Sfx, "Eagle-I3DA Room Index", room.1.into());
        }
        if let Some(on) = self.loudness {
            set(Stage::Sfx, "Eagle-LC Enable", (on as i32).to_string());
        }
    }
}

impl Profile {
    /// The DTS mode to run; `None` for plain sound. (DTS's own "Off" mode still runs the engines: on the laptop
    /// speakers it keeps Acer's speaker protection, as on Windows.)
    pub fn dts_mode(&self) -> Option<Mode> {
        self.mode.parse().ok()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// DTS on or off everywhere (off = plain sound on every device).
    pub enabled: bool,
    /// Profile per kind of output, used for devices without their own choice.
    pub defaults: BTreeMap<String, String>,
    /// Profile chosen per output device (PipeWire node name).
    pub devices: BTreeMap<String, String>,
    /// User-made profiles (the built-in ones are always available).
    pub profiles: Vec<Profile>,
    /// Headphone model per output (`@headphones` for the 3.5 mm jack, else the device's node name): a model id
    /// from the headphone catalog, or `default` for Acer's own headphone EQ. Unset: recognised automatically
    /// for Bluetooth/USB headphones, else Acer's default.
    pub headphones: BTreeMap<String, String>,
    /// Windows' spatial sound (DTS:X Ultra / Headphone:X) per output, same keys as `headphones`. Off unless set.
    pub spatial: BTreeMap<String, bool>,
    /// Spatial sound's "Best power" option (lighter presets) instead of "Best sound".
    pub spatial_power_saving: bool,
}

impl Default for Config {
    fn default() -> Config {
        let defaults = [
            ("speakers", "Music"),
            ("headphones", "Music"),
            ("bluetooth", "Music"),
            ("bluetooth-speaker", "Plain"),
            ("other", "Plain"),
        ];
        Config {
            enabled: true,
            defaults: defaults.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            devices: BTreeMap::new(),
            profiles: Vec::new(),
            headphones: BTreeMap::new(),
            spatial: BTreeMap::new(),
            spatial_power_saving: false,
        }
    }
}

/// Unprocessed sound.
fn plain() -> Profile {
    Profile { name: "Plain".into(), mode: "plain".into(), tweaks: Tweaks::default() }
}

pub fn builtin_profiles() -> Vec<Profile> {
    let mut v: Vec<Profile> = Mode::ALL
        .iter()
        .map(|m| Profile { name: m.to_string(), mode: m.key().to_lowercase(), tweaks: Tweaks::default() })
        .collect();
    v.push(plain());
    v
}

pub fn kind_key(k: Kind) -> &'static str {
    match k {
        Kind::Speakers => "speakers",
        Kind::Headphones => "headphones",
        Kind::Bluetooth => "bluetooth",
        Kind::BluetoothSpeaker => "bluetooth-speaker",
        Kind::Other => "other",
    }
}

impl Config {
    pub fn path() -> PathBuf {
        xdg("XDG_CONFIG_HOME", ".config").join("acerultrax/config.toml")
    }

    pub fn load() -> Config {
        let mut c: Config = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| toml::from_str(&t).map_err(|e| log::warn!("config: {e}")).ok())
            .unwrap_or_default();
        // Before "Plain" existed, "Off" meant unprocessed; keep that for outputs Windows never processed.
        for k in ["other", "bluetooth-speaker"] {
            if c.defaults.get(k).is_some_and(|v| v == "Off") {
                c.defaults.insert(k.into(), "Plain".into());
            }
        }
        c
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

    /// The laptop's own outputs (speakers, wired headphones) are set per kind, so the headphone profile can be
    /// chosen before anything is plugged in; everything else per device.
    pub fn set_device_profile(&mut self, device: &str, kind: Kind, profile: &str) {
        if matches!(kind, Kind::Speakers | Kind::Headphones) {
            self.devices.remove(device);
            self.defaults.insert(kind_key(kind).into(), profile.into());
        } else {
            self.devices.insert(device.into(), profile.into());
        }
    }

    /// The profile for a device: its own choice, else the default for its kind, else Off.
    pub fn profile_for(&self, device: &str, kind: Kind) -> Profile {
        // A device's own choice may name a profile that was deleted since: then the default for its kind.
        self.devices
            .get(device)
            .and_then(|n| self.profile(n))
            .or_else(|| self.defaults.get(kind_key(kind)).and_then(|n| self.profile(n)))
            .unwrap_or_else(plain)
    }

    /// Add or replace a user profile. Built-in names are reserved; the mode must be a DTS mode.
    pub fn save_profile(&mut self, p: Profile) -> Result<(), String> {
        let name = p.name.trim();
        if name.is_empty() || builtin_profiles().iter().any(|b| b.name.eq_ignore_ascii_case(name)) {
            return Err(format!("{name:?} cannot be used as a profile name"));
        }
        if p.dts_mode().is_none() {
            return Err(format!("{:?} is not a DTS mode", p.mode));
        }
        let p = Profile { name: name.to_string(), ..p };
        self.profiles.retain(|x| !x.name.eq_ignore_ascii_case(&p.name));
        self.profiles.push(p);
        Ok(())
    }

    pub fn delete_profile(&mut self, name: &str) -> bool {
        let before = self.profiles.len();
        self.profiles.retain(|x| !x.name.eq_ignore_ascii_case(name));
        before != self.profiles.len()
    }
}

/// Which output the user chose most recently comes first (as WirePlumber remembers default devices).
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub history: Vec<String>,
    /// Bluetooth devices seen before (node name -> name shown), so their profiles can be set while away.
    pub bluetooth: BTreeMap<String, (String, Kind)>,
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
    fn tweaks_become_engine_settings() {
        let mut t = dts_eagle::ModeTuning { mode: Mode::Music, output: dts_eagle::Output::Speakers, settings: vec![] };
        let tw = Tweaks {
            eq: Some([10, 0, 0, 0, 0, 0, 0, 0, 0, -200]),
            bass: Some(0),
            dialog: Some(60),
            room: Some("game".into()),
            ..Default::default()
        };
        tw.apply(&mut t);
        let s: Vec<(String, String)> = t.engine_settings(dts_eagle::Location::Mfx).into_iter().skip(1).collect();
        assert!(!s.iter().any(|(n, _)| n.contains("GEQ")), "the equaliser is not an engine setting");
        assert!(s.contains(&("Eagle-TBHDX Enable".into(), "0".into())));
        let sfx = t.engine_settings(dts_eagle::Location::Sfx);
        assert!(sfx.contains(&("Eagle-DE Level".into(), "60".into())));
        assert!(sfx.contains(&("Eagle-I3DA Room Index".into(), "Eagle-I3DA Room Game".into())));
        assert!(Tweaks::default().is_empty());
    }

    #[test]
    fn profile_lookup() {
        let mut c = Config::default();
        assert_eq!(c.profile_for("bluez_output.x", Kind::Bluetooth).name, "Music");
        assert_eq!(c.profile_for("hdmi", Kind::Other).dts_mode(), None);
        assert_eq!(c.profile_for("bluez_output.speaker", Kind::BluetoothSpeaker).dts_mode(), None);
        assert_eq!(c.profile("off").unwrap().dts_mode(), Some(Mode::Off), "DTS's Off mode runs the engines");
        let mine = Profile {
            name: "Night".into(),
            mode: "movie".into(),
            tweaks: Tweaks { bass: Some(10), ..Default::default() },
        };
        assert!(
            c.save_profile(Profile { name: "music".into(), ..mine.clone() }).is_err(),
            "built-in names are reserved"
        );
        c.save_profile(mine.clone()).unwrap();
        c.devices.insert("spk".into(), "Night".into());
        assert_eq!(c.profile_for("spk", Kind::Speakers), mine);
        assert!(c.delete_profile("night"));
        assert_eq!(c.profile_for("spk", Kind::Speakers).name, "Music", "falls back to the default for its kind");
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
