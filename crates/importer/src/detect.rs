//! Finding the laptop's sound chip from ALSA's codec descriptions (`/proc/asound/card*/codec#*`), or on newer
//! laptops without an HDA sound chip, from the audio controller (SoundWire) or Realtek's USB audio in `/sys`.

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

    /// The speaker/headphone codec (Realtek, or Senary/Conexant on some models), not an HDMI one.
    fn is_analog(&self) -> bool {
        !["8086", "10DE", "1002"].iter().any(|v| self.vendor_id.starts_with(v)) && !self.name.contains("HDMI")
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

/// The Acer laptop's sound chip, if there is one: its analog HDA codec, or on newer laptops without one the
/// SoundWire audio controller or Realtek's USB audio, with the ID Acer's tuning folders use for them.
pub fn laptop_codec() -> Option<Codec> {
    detect_codecs().into_iter().find(|c| c.is_acer() && c.is_analog()).or_else(|| sys_audio(Path::new("/sys")))
}

// shortcut: Acer's 2025 packages name SoundWire tuning "<model>1025" and list Realtek USB audio as "0BDA4E75";
// that the audio controller's PCI subsystem gives <model> is inferred, not checked on such a laptop. Check it
// when someone with one reports back.
fn sys_audio(sys: &Path) -> Option<Codec> {
    let read = |p: &Path| std::fs::read_to_string(p).map(|s| s.trim().to_ascii_uppercase()).unwrap_or_default();
    if !read(&sys.join("class/dmi/id/sys_vendor")).starts_with("ACER") {
        return None;
    }
    for d in std::fs::read_dir(sys.join("bus/pci/devices")).into_iter().flatten().flatten() {
        let p = d.path();
        if read(&p.join("class")).starts_with("0X0403") && read(&p.join("subsystem_vendor")) == "0X1025" {
            let model = read(&p.join("subsystem_device")).trim_start_matches("0X").to_string();
            let vendor_id = format!("{}{}", read(&p.join("vendor")), read(&p.join("device"))).replace("0X", "");
            return Some(Codec {
                card: 0,
                name: "SoundWire audio".into(),
                vendor_id,
                ssid: format!("{model:0>4}1025"),
            });
        }
    }
    let usb = std::fs::read_dir(sys.join("bus/usb/devices")).into_iter().flatten().flatten();
    usb.map(|d| d.path()).find(|p| read(&p.join("idVendor")) == "0BDA" && read(&p.join("idProduct")) == "4E75").map(
        |_| Codec { card: 0, name: "Realtek USB audio".into(), vendor_id: "0BDA4E75".into(), ssid: "0BDA4E75".into() },
    )
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
        assert!(!hdmi.is_realtek() && !hdmi.is_analog());
    }

    #[test]
    fn soundwire_and_usb_audio() {
        let sys = std::env::temp_dir().join(format!("acerultrax-sys-{}", std::process::id()));
        let w = |p: &str, v: &str| {
            let f = sys.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, format!("{v}\n")).unwrap();
        };
        w("class/dmi/id/sys_vendor", "Acer");
        w("bus/usb/devices/3-1/idVendor", "0bda");
        w("bus/usb/devices/3-1/idProduct", "4e75");
        assert_eq!(sys_audio(&sys).unwrap().ssid, "0BDA4E75");
        for (f, v) in
            [("class", "0x040380"), ("vendor", "0x8086"), ("device", "0xa828"), ("subsystem_vendor", "0x1025")]
        {
            w(&format!("bus/pci/devices/0000:00:1f.3/{f}"), v);
        }
        w("bus/pci/devices/0000:00:1f.3/subsystem_device", "0x1939");
        let c = sys_audio(&sys).unwrap();
        assert_eq!((c.ssid.as_str(), c.vendor_id.as_str()), ("19391025", "8086A828"));
        w("class/dmi/id/sys_vendor", "Dell Inc.");
        assert!(sys_audio(&sys).is_none());
        std::fs::remove_dir_all(&sys).unwrap();
    }
}
