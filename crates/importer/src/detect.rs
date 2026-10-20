//! Finding the laptop's sound chip from ALSA's codec descriptions (`/proc/asound/card*/codec#*`).

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Codec {
    pub card: u32,
    pub name: String,
    /// PCI vendor/device of the codec, e.g. `10EC0245` (Realtek ALC245).
    pub vendor_id: String,
    /// Subsystem ID, e.g. `1025166C`: the laptop maker (1025 = Acer) and model. Acer's tuning folders are named
    /// after it.
    pub ssid: String,
}

impl Codec {
    pub fn is_acer(&self) -> bool {
        self.ssid.starts_with("1025")
    }

    /// Realtek codecs drive the speakers and headphone jack; HDMI codecs share the subsystem ID but are not
    /// what DTS tunes.
    pub fn is_realtek(&self) -> bool {
        self.vendor_id.starts_with("10EC")
    }
}

fn parse_codec(card: u32, text: &str) -> Option<Codec> {
    let field = |key: &str| {
        text.lines().find_map(|l| l.strip_prefix(key)).map(|v| v.trim().trim_start_matches("0x").to_ascii_uppercase())
    };
    let name = text.lines().find_map(|l| l.strip_prefix("Codec:"))?.trim().to_string();
    Some(Codec { card, name, vendor_id: field("Vendor Id:")?, ssid: field("Subsystem Id:")? })
}

/// Every HDA codec ALSA knows about.
pub fn detect_codecs() -> Vec<Codec> {
    detect_in(Path::new("/proc/asound"))
}

fn detect_in(asound: &Path) -> Vec<Codec> {
    let mut out = Vec::new();
    let Ok(cards) = std::fs::read_dir(asound) else { return out };
    for card in cards.flatten() {
        let name = card.file_name().to_string_lossy().into_owned();
        let Some(n) = name.strip_prefix("card").and_then(|n| n.parse::<u32>().ok()) else { continue };
        let Ok(entries) = std::fs::read_dir(card.path()) else { continue };
        for e in entries.flatten() {
            if !e.file_name().to_string_lossy().starts_with("codec#") {
                continue;
            }
            if let Some(c) = std::fs::read_to_string(e.path()).ok().and_then(|t| parse_codec(n, &t)) {
                out.push(c);
            }
        }
    }
    out.sort_by(|a, b| (a.card, &a.name).cmp(&(b.card, &b.name)));
    out
}

/// The Acer laptop's analog codec, if there is one.
pub fn laptop_codec() -> Option<Codec> {
    detect_codecs().into_iter().find(|c| c.is_acer() && c.is_realtek())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_codec() {
        let text = "Codec: Realtek ALC245\nAddress: 0\nVendor Id: 0x10ec0245\nSubsystem Id: 0x1025166c\nRevision Id: 0x100001\n";
        let c = parse_codec(0, text).unwrap();
        assert_eq!(c.ssid, "1025166C");
        assert_eq!(c.vendor_id, "10EC0245");
        assert!(c.is_acer() && c.is_realtek());
        let hdmi =
            parse_codec(0, "Codec: Nvidia GPU a7 HDMI/DP\nVendor Id: 0x10de00a7\nSubsystem Id: 0x1025166c\n").unwrap();
        assert!(!hdmi.is_realtek());
    }
}
