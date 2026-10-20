//! First-run setup: detect the laptop's sound chip and lay out the DTS engine and Acer tuning files from the
//! user's own copy of Acer's driver package.
//!
//! Layout under the data directory (`$XDG_DATA_HOME/acerultrax`, usually `~/.local/share/acerultrax`):
//!
//! ```text
//! dts/                 dtstech64.dll and the tuning files for this laptop (the engine's tuning directory)
//! dts/headphones/      headphone models and their EQs (see [`headphones`])
//! dts/spatial/         the spatial sound encoder and its presets (see [`spatial`])
//! manifest.toml        what was imported, from where, with checksums
//! ```
//!
//! Sources can be folders or Acer's zip downloads, as they are.

mod detect;
pub mod headphones;
mod sources;
pub mod spatial;

pub use detect::{Codec, detect_codecs, laptop_codec};
pub use headphones::{Catalog, Headphone};
use sources::Entry;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("no dtstech64.dll found under {0}")]
    NoEngine(PathBuf),
    #[error("no DTS tuning for sound chip {ssid} found under {dir} (looked for dts_apo4_oem_config_{ssid}.xml)")]
    NoTuning { ssid: String, dir: PathBuf },
    #[error("tuning for {ssid} is incomplete: {what}")]
    Incomplete { ssid: String, what: String },
    #[error(transparent)]
    Config(#[from] dts_eagle::Error),
    #[error("manifest: {0}")]
    Manifest(String),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io { path: path.to_path_buf(), source }
}

/// How well a laptop is supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Support {
    /// Tested end to end against Windows.
    Verified,
    /// Acer ships a DTS tuning for it, but nobody has checked it under AcerUltraX yet.
    Experimental,
}

/// Sound-chip subsystem IDs (as in the Acer tuning folder names) that have been verified.
pub const VERIFIED_MODELS: &[(&str, &str)] = &[("1025166C", "Acer Predator Helios 16 PH16-71")];

pub fn support(ssid: &str) -> Support {
    if VERIFIED_MODELS.iter().any(|(s, _)| s.eq_ignore_ascii_case(ssid)) {
        Support::Verified
    } else {
        Support::Experimental
    }
}

/// SHA-256 of `dtstech64.dll` builds that have been verified (Acer "XPERI DTS" 1.12.4.0).
pub const VERIFIED_ENGINES: &[&str] = &["ea3da98e0224ef9d41c16d1a384a41c65c56e983fb10f6e73d73d74fa4e63200"];

/// Where AcerUltraX keeps its data.
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(d).join("acerultrax");
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join(".local/share/acerultrax")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub ssid: String,
    pub support: Support,
    /// Where the files were imported from.
    pub source: PathBuf,
    pub engine_sha256: String,
    pub engine_verified: bool,
    /// Imported tuning file names (in `dts/`) and their SHA-256.
    pub files: Vec<(String, String)>,
    /// Files the configuration names for external speakers that the package does not contain.
    #[serde(default)]
    pub missing_optional: Vec<String>,
    /// Headphone models imported (0 without the DTS app package).
    #[serde(default)]
    pub headphones: usize,
    /// Spatial sound presets imported (0 without the 2023 DTS app package).
    #[serde(default)]
    pub spatial_presets: usize,
}

impl Manifest {
    pub const FILE: &'static str = "manifest.toml";

    pub fn load(data_dir: &Path) -> Result<Manifest, Error> {
        let p = data_dir.join(Self::FILE);
        let text = std::fs::read_to_string(&p).map_err(io(&p))?;
        toml::from_str(&text).map_err(|e| Error::Manifest(e.to_string()))
    }

    /// The engine's tuning directory.
    pub fn dts_dir(data_dir: &Path) -> PathBuf {
        data_dir.join("dts")
    }

    pub fn engine_path(data_dir: &Path) -> PathBuf {
        Self::dts_dir(data_dir).join("dtstech64.dll")
    }

    pub fn oem_config_path(&self, data_dir: &Path) -> PathBuf {
        Self::dts_dir(data_dir).join(oem_config_name(&self.ssid))
    }
}

fn oem_config_name(ssid: &str) -> String {
    format!("dts_apo4_oem_config_{}.xml", ssid.to_ascii_uppercase())
}

fn sha256(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

fn read(e: &Entry) -> Result<Vec<u8>, Error> {
    e.read().map_err(io(Path::new(&e.describe())))
}

/// Import from Acer's downloads (the DTS driver package, and optionally the DTS app package for headphone
/// models), as zip files or extracted folders, into `data_dir`, replacing a previous import.
pub fn import(srcs: &[PathBuf], ssid: &str, data_dir: &Path) -> Result<Manifest, Error> {
    let ssid = ssid.to_ascii_uppercase();
    let src = srcs.first().cloned().unwrap_or_default();
    let files = sources::collect(srcs);
    let find = |name: &str| files.iter().find(|f| f.name().eq_ignore_ascii_case(name));
    let engine = find("dtstech64.dll").ok_or_else(|| Error::NoEngine(src.clone()))?;
    let config_name = oem_config_name(&ssid);
    let config = find(&config_name).ok_or_else(|| Error::NoTuning { ssid: ssid.clone(), dir: src.clone() })?;

    // Check the tuning before touching the existing installation.
    let xml = String::from_utf8_lossy(&read(config)?).into_owned();
    let oem = dts_eagle::OemConfig::parse(&xml)?;
    for out in [dts_eagle::Output::Speakers, dts_eagle::Output::Headphones] {
        for mode in dts_eagle::Mode::ALL {
            if oem.tuning(mode, out).is_none() {
                return Err(Error::Incomplete { ssid, what: format!("no {mode} mode for {out:?}") });
            }
        }
    }
    // Files for the laptop's speakers and headphones are required. Some Acer packages name external-speaker
    // files they do not ship; those are left out, as Windows has to do too.
    let mut wanted: Vec<(String, bool)> = oem
        .tunings
        .iter()
        .flat_map(|t| {
            t.files().into_iter().map(move |f| (f.to_string(), t.output != dts_eagle::Output::ExternalSpeakers))
        })
        .collect();
    wanted.sort();
    wanted.dedup_by(|b, a| {
        let same = a.0 == b.0;
        if same {
            a.1 |= b.1;
        }
        same
    });
    let mut sources = vec![(config_name.clone(), config)];
    let mut missing_optional = Vec::new();
    for (name, required) in &wanted {
        // Tuning files sit next to the OEM configuration.
        match files.iter().find(|f| f.parent() == config.parent() && f.name().eq_ignore_ascii_case(name)) {
            Some(f) => sources.push((name.clone(), f)),
            None if *required => return Err(Error::Incomplete { ssid, what: format!("missing {name}") }),
            None => missing_optional.push(name.clone()),
        }
    }

    // Stage into a fresh directory, then swap it in.
    let dts = Manifest::dts_dir(data_dir);
    let staging = data_dir.join("dts.new");
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(io(&staging))?;
    }
    std::fs::create_dir_all(&staging).map_err(io(&staging))?;
    let engine_bytes = read(engine)?;
    let p = staging.join("dtstech64.dll");
    std::fs::write(&p, &engine_bytes).map_err(io(&p))?;
    let mut hashes = Vec::new();
    for (name, e) in &sources {
        let bytes = read(e)?;
        let p = staging.join(name);
        std::fs::write(&p, &bytes).map_err(io(&p))?;
        hashes.push((name.clone(), sha256(&bytes)));
    }
    let headphones = match headphones::import(&files, &staging) {
        Ok(n) => n,
        Err(e) => {
            log::warn!("headphone models not imported: {e}");
            0
        }
    };
    let spatial = match spatial::import(&files, &staging) {
        Ok(n) => n,
        Err(e) => {
            log::warn!("spatial sound not imported: {e}");
            0
        }
    };
    let engine_sha256 = sha256(&engine_bytes);
    let manifest = Manifest {
        support: support(&ssid),
        ssid,
        source: src.clone(),
        engine_verified: VERIFIED_ENGINES.contains(&engine_sha256.as_str()),
        engine_sha256,
        files: hashes,
        missing_optional,
        headphones,
        spatial_presets: spatial,
    };
    if dts.exists() {
        std::fs::remove_dir_all(&dts).map_err(io(&dts))?;
    }
    std::fs::rename(&staging, &dts).map_err(io(&dts))?;
    let mpath = data_dir.join(Manifest::FILE);
    let text = toml::to_string_pretty(&manifest).map_err(|e| Error::Manifest(e.to_string()))?;
    std::fs::write(&mpath, text).map_err(io(&mpath))?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("acerultrax-importer-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fake_package(root: &Path, ssid: &str) {
        let tuning = root.join(format!("Ext/Acer{ssid}"));
        std::fs::create_dir_all(&tuning).unwrap();
        std::fs::create_dir_all(root.join("Core")).unwrap();
        std::fs::write(root.join("Core/dtstech64.dll"), b"MZ fake").unwrap();
        let mut xml = String::from("<srs_configuration>");
        for out in ["APO4-Internal Stereo", "APO4-Headphone"] {
            for m in dts_eagle::Mode::ALL {
                xml += &format!(
                    "<operating_mode><control name=\"APO4-Operating Mode\">{}</control>\
                     <control name=\"APO4-Speaker Type\">{out}</control>\
                     <control name=\"MFX:Eagle-CTC Blob\">ctc_{ssid}.bin</control></operating_mode>",
                    m.apo_id()
                );
            }
        }
        xml += "</srs_configuration>";
        std::fs::write(tuning.join(format!("dts_apo4_oem_config_{ssid}.xml")), xml).unwrap();
        std::fs::write(tuning.join(format!("CTC_{ssid}.bin")), b"ctc").unwrap();
        std::fs::write(tuning.join(format!("unrelated_{ssid}.lic")), b"lic").unwrap();
    }

    #[test]
    fn imports_and_replaces() {
        let (src, data) = (tmp("src"), tmp("data"));
        fake_package(&src, "1025ABCD");
        let m = import(std::slice::from_ref(&src), "1025abcd", &data).unwrap();
        assert_eq!(m.support, Support::Experimental);
        assert!(!m.engine_verified);
        let dts = Manifest::dts_dir(&data);
        assert!(dts.join("dtstech64.dll").exists());
        assert!(dts.join("ctc_1025ABCD.bin").exists(), "tuning files keep the name the XML uses");
        assert!(!dts.join("unrelated_1025ABCD.lic").exists());
        assert_eq!(Manifest::load(&data).unwrap().ssid, "1025ABCD");
        // A second import replaces the first cleanly.
        import(std::slice::from_ref(&src), "1025ABCD", &data).unwrap();
        assert!(!data.join("dts.new").exists());
        std::fs::remove_dir_all(&src).unwrap();
        std::fs::remove_dir_all(&data).unwrap();
    }

    #[test]
    fn backslash_names_from_windows_zips() {
        let (src, data) = (tmp("src3"), tmp("data3"));
        fake_package(&src, "1025ABCD");
        // Flatten into names like `Ext\Acer1025ABCD\file`, as `unzip` produces for Windows-made archives.
        let flat = tmp("flat3");
        for f in sources::collect(std::slice::from_ref(&src)) {
            std::fs::write(flat.join(f.parts[1..].join("\\")), f.read().unwrap()).unwrap();
        }
        let m = import(std::slice::from_ref(&flat), "1025ABCD", &data).unwrap();
        assert_eq!(m.files.len(), 2);
        for d in [src, data, flat] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }

    #[test]
    fn reports_missing_tuning() {
        let (src, data) = (tmp("src2"), tmp("data2"));
        fake_package(&src, "1025ABCD");
        assert!(matches!(import(std::slice::from_ref(&src), "10251234", &data), Err(Error::NoTuning { .. })));
        std::fs::remove_file(src.join("Ext/Acer1025ABCD/CTC_1025ABCD.bin")).unwrap();
        assert!(matches!(import(std::slice::from_ref(&src), "1025ABCD", &data), Err(Error::Incomplete { .. })));
        assert!(!Manifest::dts_dir(&data).exists(), "failed imports leave nothing behind");
        std::fs::remove_dir_all(&src).unwrap();
        std::fs::remove_dir_all(&data).unwrap();
    }

    #[test]
    fn support_levels() {
        assert_eq!(support("1025166c"), Support::Verified);
        assert_eq!(support("10251613"), Support::Experimental);
    }
}
