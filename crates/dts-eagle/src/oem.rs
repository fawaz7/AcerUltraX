//! Acer's OEM tuning configuration (`dts_apo4_oem_config_<SSID>.xml`): the factory settings of every DTS mode for
//! every output type, and how they become engine controls.

use std::fmt;
use std::str::FromStr;

use crate::Error;
use crate::engine::Location;

/// The DTS:X Ultra modes offered by the Windows DTS app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Mode {
    Off,
    Music,
    Movie,
    Voice,
    Game1,
    Game2,
    Game3,
    CustomAudio,
}

impl Mode {
    pub const ALL: [Mode; 8] =
        [Mode::Off, Mode::Music, Mode::Movie, Mode::Voice, Mode::Game1, Mode::Game2, Mode::Game3, Mode::CustomAudio];

    /// The identifier Windows uses (`APO4-Movie`, ...).
    pub fn apo_id(self) -> &'static str {
        match self {
            Mode::Off => "APO4-Off",
            Mode::Music => "APO4-Music",
            Mode::Movie => "APO4-Movie",
            Mode::Voice => "APO4-Voice",
            Mode::Game1 => "APO4-Game1",
            Mode::Game2 => "APO4-Game2",
            Mode::Game3 => "APO4-Game3",
            Mode::CustomAudio => "APO4-Custom Audio",
        }
    }

    /// Short machine name (`Movie`, `Custom_Audio`).
    pub fn key(self) -> &'static str {
        match self {
            Mode::CustomAudio => "Custom_Audio",
            m => &m.apo_id()[5..],
        }
    }

    fn from_apo_id(s: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.apo_id() == s)
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Mode::CustomAudio => "Custom Audio",
            m => m.key(),
        })
    }
}

impl FromStr for Mode {
    type Err = Error;
    fn from_str(s: &str) -> Result<Mode, Error> {
        let norm = |x: &str| x.to_ascii_lowercase().replace([' ', '_', '-'], "");
        Mode::ALL
            .into_iter()
            .find(|m| norm(m.key()) == norm(s))
            .ok_or_else(|| Error::Config(format!("unknown mode {s:?}")))
    }
}

/// The output types the OEM configuration has tunings for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Output {
    /// The laptop's built-in speakers.
    Speakers,
    /// Wired headphones.
    Headphones,
    /// External speakers (tuned like the built-in ones on the PH16-71).
    ExternalSpeakers,
}

impl Output {
    fn speaker_type(self) -> &'static str {
        match self {
            Output::Speakers => "APO4-Internal Stereo",
            Output::Headphones => "APO4-Headphone",
            Output::ExternalSpeakers => "APO4-External Stereo",
        }
    }

    /// Name used in the engine's `Eagle-Apo Mode Speaker string` control.
    fn engine_name(self) -> &'static str {
        match self {
            Output::Speakers => "APO4-InternalSpeaker",
            Output::Headphones => "APO4-Headphone",
            Output::ExternalSpeakers => "APO4-ExternalSpeaker",
        }
    }
}

/// Which engines a control applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Both,
    Sfx,
    Mfx,
}

impl Stage {
    pub fn applies_to(self, loc: Location) -> bool {
        matches!((self, loc), (Stage::Both, _) | (Stage::Sfx, Location::Sfx) | (Stage::Mfx, Location::Mfx))
    }
}

/// One engine control setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting {
    pub stage: Stage,
    pub name: String,
    pub value: String,
}

/// The settings of one mode for one output.
#[derive(Debug, Clone)]
pub struct ModeTuning {
    pub mode: Mode,
    pub output: Output,
    /// Engine controls in file order.
    pub settings: Vec<Setting>,
}

/// Value of `Eagle-Product Name` for DTS:X Ultra laptops. The engine runs the lower "DTS Audio" tier otherwise.
pub const PRODUCT_NAME: &str = "Eagle-DTS:X Ultra";

impl ModeTuning {
    /// The value of the engine's `Eagle-Apo Mode Speaker string` control, which Windows sets before the mode's
    /// other controls.
    pub fn mode_ident(&self, loc: Location) -> String {
        format!("{}:{}:{}", loc.as_str(), self.mode.apo_id(), self.output.engine_name())
    }

    /// Controls to apply to an engine at `loc`, in order, starting with the mode identifier.
    ///
    /// The licence file control is left out (the engine only checks that the file opens) and the product tier is
    /// always DTS:X Ultra.
    pub fn engine_settings(&self, loc: Location) -> Vec<(String, String)> {
        let mut out = vec![("Eagle-Apo Mode Speaker string".to_string(), self.mode_ident(loc))];
        for s in self.settings.iter().filter(|s| s.stage.applies_to(loc)) {
            match s.name.as_str() {
                "Eagle-DSEC File" => {}
                "Eagle-Product Name" => out.push((s.name.clone(), PRODUCT_NAME.into())),
                _ => out.push((s.name.clone(), s.value.clone())),
            }
        }
        out
    }

    /// Files named by the settings (tuning blobs the engine reads from the tuning directory).
    pub fn files(&self) -> Vec<&str> {
        self.settings
            .iter()
            .filter(|s| (s.name.contains("Blob") && !s.name.ends_with("Enable")) || s.name.ends_with("File"))
            .filter(|s| s.name != "Eagle-DSEC File" && s.value.contains('.'))
            .map(|s| s.value.as_str())
            .collect()
    }
}

/// A parsed OEM configuration.
#[derive(Debug, Clone)]
pub struct OemConfig {
    /// Codec subsystem ID the file is for (e.g. `1025166C`).
    pub ssid: Option<String>,
    pub tunings: Vec<ModeTuning>,
}

impl OemConfig {
    pub fn parse(xml: &str) -> Result<OemConfig, Error> {
        let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Config(e.to_string()))?;
        let controls = |n: roxmltree::Node| -> Vec<(String, String)> {
            n.children()
                .filter(|c| c.has_tag_name("control"))
                .map(|c| (c.attribute("name").unwrap_or("").to_string(), c.text().unwrap_or("").trim().to_string()))
                .collect()
        };
        let root = doc.root_element();
        let ssid = root
            .children()
            .find(|n| n.has_tag_name("properties"))
            .and_then(|p| controls(p).into_iter().find(|(k, _)| k == "SSID").map(|(_, v)| v));
        let mut tunings = Vec::new();
        for om in root.descendants().filter(|n| n.has_tag_name("operating_mode")) {
            let ctl = controls(om);
            let get = |k: &str| ctl.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
            let (Some(mode), Some(speaker)) = (get("APO4-Operating Mode"), get("APO4-Speaker Type")) else {
                continue;
            };
            let Some(mode) = Mode::from_apo_id(mode) else { continue };
            let Some(output) = [Output::Speakers, Output::Headphones, Output::ExternalSpeakers]
                .into_iter()
                .find(|o| o.speaker_type() == speaker)
            else {
                continue;
            };
            let settings = ctl.iter().filter_map(|(n, v)| to_setting(n, v)).collect();
            // A later entry for the same mode and output replaces an earlier one.
            tunings.retain(|t: &ModeTuning| !(t.mode == mode && t.output == output));
            tunings.push(ModeTuning { mode, output, settings });
        }
        Ok(OemConfig { ssid, tunings })
    }

    pub fn tuning(&self, mode: Mode, output: Output) -> Option<&ModeTuning> {
        self.tunings.iter().find(|t| t.mode == mode && t.output == output)
    }
}

/// Engine settings are the `Eagle-*` controls, optionally prefixed with the stage (`SFX:`/`MFX:`); controls for
/// other stages or for the Windows APO itself are not engine settings.
fn to_setting(name: &str, value: &str) -> Option<Setting> {
    let (stage, bare) = match name.split_once(':') {
        None => (Stage::Both, name),
        Some(("SFX", b)) => (Stage::Sfx, b),
        Some(("MFX", b)) => (Stage::Mfx, b),
        Some(_) => return None,
    };
    bare.starts_with("Eagle-").then(|| Setting { stage, name: bare.into(), value: value.into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<srs_configuration>
  <properties>
    <control name="APO4-Operating Mode">APO4-Music</control>
    <control name="SSID">1025ABCD</control>
  </properties>
  <operating_mode>
    <control name="APO4-Operating Mode">APO4-Movie</control>
    <control name="APO4-Speaker Type">APO4-Headphone</control>
    <control name="APO4-OperatingMode-LastSetMode">APO4-Automatic</control>
    <control name="Eagle-DSEC File">x.lic</control>
    <control name="Eagle-Product Name">Eagle-DTS Audio</control>
    <control name="SFX:Eagle-LC Enable">1</control>
    <control name="MFX:Eagle-CTC Blob">ctc.bin</control>
    <control name="EFX:Eagle-LC Enable">1</control>
  </operating_mode>
</srs_configuration>"#;

    #[test]
    fn parses_modes_and_stages() {
        let c = OemConfig::parse(XML).unwrap();
        assert_eq!(c.ssid.as_deref(), Some("1025ABCD"));
        assert_eq!(c.tunings.len(), 1);
        let t = c.tuning(Mode::Movie, Output::Headphones).unwrap();
        assert_eq!(t.settings.len(), 4);
        assert_eq!(
            t.engine_settings(Location::Sfx),
            vec![
                ("Eagle-Apo Mode Speaker string".to_string(), "SFX:APO4-Movie:APO4-Headphone".to_string()),
                ("Eagle-Product Name".to_string(), PRODUCT_NAME.to_string()),
                ("Eagle-LC Enable".to_string(), "1".to_string()),
            ]
        );
        let mfx = t.engine_settings(Location::Mfx);
        assert_eq!(mfx[0].1, "MFX:APO4-Movie:APO4-Headphone");
        assert_eq!(mfx.last().unwrap(), &("Eagle-CTC Blob".to_string(), "ctc.bin".to_string()));
        assert_eq!(t.files(), vec!["ctc.bin"]);
    }

    #[test]
    fn mode_names() {
        assert_eq!("custom audio".parse::<Mode>().unwrap(), Mode::CustomAudio);
        assert_eq!("Custom_Audio".parse::<Mode>().unwrap(), Mode::CustomAudio);
        assert_eq!(Mode::Game2.key(), "Game2");
        assert_eq!(Mode::CustomAudio.to_string(), "Custom Audio");
    }
}
