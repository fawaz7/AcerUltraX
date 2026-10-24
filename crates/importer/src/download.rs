//! One-click setup: Acer's DTS downloads, fetched straight from Acer's download server on the user's machine.
//!
//! Acer's support pages build their download links with scripts, but the links themselves are plain files
//! (`acerid` is a fixed file number), so the app knows them up front. Only links and checksums are kept here;
//! the files come from Acer. One driver package carries the tuning for every laptop in [`COVERED`]; a few older
//! laptops need an older package too ([`FALLBACKS`]).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not reach Acer's download server ({0}). Check the internet connection and try again.")]
    Network(String),
    #[error("the download from Acer was damaged (checksum mismatch). Please try again.")]
    Checksum,
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("cancelled")]
    Cancelled,
    #[error(transparent)]
    Import(#[from] crate::Error),
}

/// One of Acer's downloads.
#[derive(Debug, Clone, Copy)]
pub struct Package {
    /// As Acer's support page lists it.
    pub name: &'static str,
    /// What it adds, in plain words.
    pub adds: &'static str,
    pub url: &'static str,
    pub file: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// Driver → Audio: "XPERI DTS" 1.16.11.0 (2026): the engine, and the tuning for every laptop in [`COVERED`].
pub const DRIVER: Package = Package {
    name: "XPERI DTS 1.16.11.0",
    adds: "DTS sound",
    url: "https://global-download.acer.com/GDFiles/Driver/XPERI%20DTS/XPERI%20DTS_XPERI_1.16.11.0_W11x64_A.zip?acerid=639186452537632000",
    file: "XPERI DTS_XPERI_1.16.11.0_W11x64_A.zip",
    size: 13_547_744,
    sha256: "47276e9ee623e2b74a7a77406fc3bf3716827697c0a7171589bc30ee026b1adb",
};

/// Older driver packages with the tuning of the few laptops [`DRIVER`] no longer has. Setup takes only that
/// tuning from them: the engine still comes from [`DRIVER`] (engine builds differ in compiler, not in sound).
pub const FALLBACKS: &[(Package, &[&str])] = &[
    (
        Package {
            // Acer's label; the package holds the 1.15.17 tuning.
            name: "XPERI DTS 1.5.11.0",
            adds: "DTS sound",
            url: "https://global-download.acer.com/GDFiles/Driver/XPERI%20DTS/XPERI%20DTS_XPERI_1.5.11.0_W11x64_A.zip?acerid=638914532078248962",
            file: "XPERI DTS_XPERI_1.5.11.0_W11x64_A.zip",
            size: 11_991_797,
            sha256: "8cbcb493959a306e5d337e9882385cb41feafba04d55af16adb56d827636b353",
        },
        &["10251906", "10251939"],
    ),
    (
        Package {
            name: "XPERI DTS 1.15.8.0",
            adds: "DTS sound",
            url: "https://global-download.acer.com/GDFiles/Driver/XPERI%20DTS/XPERI%20DTS_XPERI_1.15.8.0_W11x64_A.zip?acerid=638784740894300329",
            file: "XPERI DTS_XPERI_1.15.8.0_W11x64_A.zip",
            size: 11_418_951,
            sha256: "8f0869ecdd7aeb11f4098fca7fc6aa6c0ce821982256f23adab580b9a45517a8",
        },
        &["10251852"],
    ),
    (
        Package {
            name: "XPERI DTS 1.6.16.0",
            adds: "DTS sound",
            url: "https://global-download.acer.com/GDFiles/Driver/XPERI%20DTS/XPERI%20DTS_XPERI_1.6.16.0_W11x64_A.zip?acerid=637812690782020398",
            file: "XPERI DTS_XPERI_1.6.16.0_W11x64_A.zip",
            size: 31_756_807,
            sha256: "1f6892a2efb26c38db60a93ec010b260d4a4c397d468eecd532c500909570964",
        },
        &["1025157A"],
    ),
];

/// Application: "XPERI DTS Utility" 2026.1.2.0 (headphone models and spatial sound).
pub const APP: Package = Package {
    name: "XPERI DTS Utility 2026.1.2.0",
    adds: "headphone models and spatial sound",
    url: "https://global-download.acer.com/GDFiles/Driver/XPERI%20DTS%20Utility/XPERI%20DTS%20Utility_XPERI_2026.1.2.0_W11x64_A.zip?acerid=639114103200274707",
    file: "XPERI DTS Utility_XPERI_2026.1.2.0_W11x64_A.zip",
    size: 584_343_098,
    sha256: "7263dfa91517112407ec17fc0f5c086edf9371f87c238310e6f6040098fff161",
};

/// Sound-chip subsystem IDs whose tuning is in [`DRIVER`].
pub const COVERED: &[&str] = &[
    "0BDA4E75", "1025133B", "10251401", "1025140B", "10251419", "1025142B", "10251432", "10251433", "10251435",
    "1025143D", "1025143F", "10251441", "10251442", "10251443", "10251445", "10251448", "1025144F", "10251455",
    "1025146C", "10251470", "10251471", "10251475", "10251476", "10251479", "1025147B", "1025147D", "10251480",
    "1025148C", "10251492", "10251506", "10251507", "10251509", "1025150A", "1025151E", "1025151F", "10251520",
    "10251521", "10251523", "10251525", "1025152D", "1025152F", "10251532", "10251534", "10251535", "10251536",
    "10251537", "10251539", "1025153B", "1025153C", "1025153E", "10251540", "10251545", "10251546", "1025154D",
    "10251551", "10251555", "10251556", "1025155A", "10251560", "1025156B", "10251573", "10251574", "10251577",
    "1025157B", "1025157E", "1025158B", "10251597", "10251598", "1025159A", "1025159C", "1025159D", "1025159E",
    "10251604", "1025160C", "1025160E", "10251610", "10251611", "10251612", "10251613", "10251614", "10251618",
    "10251619", "1025161A", "1025161B", "1025161C", "10251620", "10251628", "1025162E", "10251630", "10251631",
    "10251632", "10251635", "10251639", "1025163A", "1025163B", "10251641", "10251642", "1025164D", "1025164E",
    "1025164F", "10251651", "1025166C", "1025166D", "1025166E", "1025166F", "10251670", "10251673", "10251674",
    "10251676", "10251677", "10251679", "1025167A", "1025167B", "1025168C", "10251691", "10251697", "10251698",
    "10251699", "1025169A", "1025169B", "1025169C", "1025169D", "10251700", "10251704", "10251709", "1025170F",
    "10251712", "10251713", "10251715", "10251718", "1025171A", "1025171B", "1025171E", "10251720", "10251726",
    "10251729", "1025172C", "1025172D", "10251730", "10251731", "10251732", "10251733", "10251734", "10251735",
    "10251736", "10251738", "1025173F", "10251742", "10251743", "10251748", "1025174F", "10251752", "10251754",
    "10251758", "10251759", "1025175B", "1025175C", "1025175E", "10251760", "10251761", "1025176B", "1025176D",
    "10251771", "1025177C", "10251786", "10251787", "10251788", "10251789", "1025178A", "1025178D", "10251792",
    "10251799", "10251807", "10251812", "1025181A", "1025181B", "1025181F", "10251826", "10251828", "1025182A",
    "1025182C", "1025182D", "10251830", "10251832", "10251834", "10251835", "10251836", "10251837", "10251838",
    "10251839", "1025183B", "1025183E", "1025183F", "10251843", "10251844", "10251850", "10251854", "10251855",
    "10251857", "10251858", "10251862", "10251863", "10251864", "10251871", "10251873", "10251874", "1025187C",
    "10251883", "10251888", "10251897", "10251907", "10251927", "10251928", "1025192F", "10251930", "1025193A",
    "1025193D", "1025193E", "1025193F", "10251967", "10251969", "10251972", "10251981", "10251982", "1025198B",
    "10251993", "10251994", "10252009", "1025200A", "10252025", "1025202A", "1025202B", "1025202C", "1025202D",
    "10252030", "10252031", "10252041", "1025204D", "1025204E", "10252050", "10252051", "1025205A", "10252063",
    "10252064", "1025208B", "1025209E", "171F1025", "175D1025", "17721025", "18981025", "19391025", "20321025",
    "20331025", "20381025", "203A1025", "203B1025", "203D1025", "203E1025", "20421025", "20431025", "20451025",
    "20461025", "20521025", "20561025", "20571025", "20581025", "20591025", "205C1025", "205D1025", "20611025",
    "20721025", "207F1025", "21091025", "210E1025", "211F1025", "21291025", "212A1025", "21391025",
];

/// Laptops whose tuning Acer changed between "XPERI DTS" 1.12.4.0 (what setup used up to AcerUltraX 0.1.2) and
/// [`DRIVER`].
const RETUNED_SINCE_1_12_4: &[&str] = &[
    "1025168C", "10251697", "1025169B", "1025169C", "1025169D", "10251704", "10251713", "10251718", "1025171A",
    "10251729", "1025172C", "1025172D", "10251730", "10251731", "10251732", "10251733", "10251734", "10251735",
    "10251736", "10251738", "1025173F", "10251742", "10251743", "10251748", "1025174F", "10251752", "10251754",
    "10251758", "10251759", "1025175B", "1025175C", "1025175E", "10251760", "10251761", "1025176B", "1025176D",
    "171F1025",
];

/// Whether setting up again from Acer's current downloads changes how this laptop sounds.
pub fn newer_tuning(m: &crate::Manifest) -> bool {
    let old = m.source.file_name().is_some_and(|n| n.to_string_lossy().contains("XPERI DTS_XPERI_1.12.4.0"));
    old && RETUNED_SINCE_1_12_4.iter().any(|s| s.eq_ignore_ascii_case(&m.ssid))
}

/// Whether Acer's downloads have the tuning for this laptop, so setup can be automatic.
pub fn covers(ssid: &str) -> bool {
    COVERED.iter().any(|s| s.eq_ignore_ascii_case(ssid)) || fallback(ssid).is_some()
}

/// The older package to take this laptop's tuning from, when [`DRIVER`] does not have it.
pub fn fallback(ssid: &str) -> Option<&'static Package> {
    if COVERED.iter().any(|s| s.eq_ignore_ascii_case(ssid)) {
        return None;
    }
    FALLBACKS.iter().find(|(_, ids)| ids.iter().any(|s| s.eq_ignore_ascii_case(ssid))).map(|(p, _)| p)
}

/// Where downloads wait until they are imported (`$XDG_CACHE_HOME/acerultrax`).
pub fn cache_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("XDG_CACHE_HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(d).join("acerultrax");
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join(".cache/acerultrax")
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io { path: path.to_path_buf(), source }
}

fn file_sha256(path: &Path) -> Result<String, Error> {
    let mut f = std::fs::File::open(path).map_err(io(path))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(io(path))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Download `pkg` into `dir` (resuming an interrupted download) and check it. `progress` gets the bytes so far.
/// Returns the file's path.
pub fn fetch(pkg: &Package, dir: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<PathBuf, Error> {
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    let done = dir.join(pkg.file);
    if std::fs::metadata(&done).is_ok_and(|m| m.len() == pkg.size) && file_sha256(&done)? == pkg.sha256 {
        progress(pkg.size);
        return Ok(done);
    }
    let part = dir.join(format!("{}.part", pkg.file));
    let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > pkg.size {
        have = 0;
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent(concat!("AcerUltraX/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    if have < pkg.size {
        let mut req = agent.get(pkg.url);
        if have > 0 {
            req = req.header("Range", format!("bytes={have}-"));
        }
        let resp = req.call().map_err(|e| Error::Network(e.to_string()))?;
        // A server that ignores the range sends the whole file again.
        if resp.status() != 206 {
            have = 0;
        }
        let mut out = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(have > 0)
            .truncate(have == 0)
            .open(&part)
            .map_err(io(&part))?;
        let mut body = resp.into_body().into_with_config().limit(pkg.size - have + 1).reader();
        let mut buf = vec![0u8; 1 << 16];
        progress(have);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let n = body.read(&mut buf).map_err(|e| Error::Network(e.to_string()))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(io(&part))?;
            have += n as u64;
            progress(have);
        }
        out.flush().map_err(io(&part))?;
    }
    if have != pkg.size {
        return Err(Error::Network("the download ended early".into()));
    }
    if file_sha256(&part)? != pkg.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(Error::Checksum);
    }
    std::fs::rename(&part, &done).map_err(io(&done))?;
    Ok(done)
}

/// Where [`setup`] is.
#[derive(Debug, Clone, Copy)]
pub enum Stage {
    /// Bytes downloaded so far of all packages together.
    Downloading {
        done: u64,
        total: u64,
    },
    Importing,
}

/// The whole one-click setup: download [`DRIVER`] (plus this laptop's [`FALLBACKS`] package, and [`APP`] if
/// `extras`), import them for `ssid` into
/// `data_dir`, then delete the downloads (only the few files AcerUltraX uses are kept).
pub fn setup(
    ssid: &str,
    data_dir: &Path,
    extras: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Stage),
) -> Result<crate::Manifest, Error> {
    // DRIVER first: the importer takes the engine from the first source that has one.
    let mut pkgs = vec![DRIVER];
    pkgs.extend(fallback(ssid).copied());
    if extras {
        pkgs.push(APP);
    }
    let total = pkgs.iter().map(|p| p.size).sum();
    let cache = cache_dir();
    let mut before = 0;
    let mut files = Vec::new();
    for pkg in &pkgs {
        files.push(fetch(pkg, &cache, cancel, &mut |n| progress(Stage::Downloading { done: before + n, total }))?);
        before += pkg.size;
    }
    progress(Stage::Importing);
    std::fs::create_dir_all(data_dir).map_err(io(data_dir))?;
    let m = crate::import(&files, ssid, data_dir)?;
    for f in &files {
        let _ = std::fs::remove_file(f);
    }
    let _ = std::fs::remove_dir(&cache);
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covered_ids_are_sound_chip_ids() {
        assert!(covers("1025166c"));
        assert!(!covers("10250000"));
        assert!(COVERED.iter().all(|s| s.len() == 8 && s.chars().all(|c| c.is_ascii_hexdigit())));
        assert!(COVERED.windows(2).all(|w| w[0] < w[1]), "sorted, no duplicates");
        assert!(covers("1025157a") && fallback("1025157A").is_some_and(|p| p.name.contains("1.6.16")));
        assert!(fallback("1025166C").is_none());
        assert!(RETUNED_SINCE_1_12_4.iter().all(|s| COVERED.contains(s)));
        let fb = FALLBACKS.iter().flat_map(|(_, ids)| ids.iter());
        assert!(fb.clone().all(|s| !COVERED.contains(s)), "fallbacks only for laptops DRIVER lacks");
    }

    /// Downloads the driver package from Acer (network; run with `--ignored`).
    #[test]
    #[ignore]
    fn fetch_driver() {
        let dir = std::env::temp_dir().join(format!("acerultrax-dl-{}", std::process::id()));
        let mut last = 0;
        let p = fetch(&DRIVER, &dir, &AtomicBool::new(false), &mut |n| last = n).unwrap();
        assert_eq!(last, DRIVER.size);
        assert_eq!(std::fs::metadata(&p).unwrap().len(), DRIVER.size);
        // An interrupted download resumes where it stopped.
        let part = dir.join(format!("{}.part", DRIVER.file));
        std::fs::rename(&p, &part).unwrap();
        std::fs::OpenOptions::new().write(true).open(&part).unwrap().set_len(DRIVER.size / 2).unwrap();
        let mut first = None;
        fetch(&DRIVER, &dir, &AtomicBool::new(false), &mut |n| {
            first.get_or_insert(n);
        })
        .unwrap();
        assert_eq!(first, Some(DRIVER.size / 2));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
