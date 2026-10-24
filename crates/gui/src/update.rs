//! App updates: a check of GitHub's latest release once a day while the app runs, and a one-click update where
//! the install method allows it. Settings, profiles and the files from Acer live in the user's config and data
//! folders, which an update never touches.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use importer::download;

const LATEST: &str = "https://api.github.com/repos/fawaz7/AcerUltraX/releases/latest";

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    /// The release notes (Markdown, as on GitHub).
    pub notes: String,
    /// Asset name, download URL, size.
    assets: Vec<(String, String, u64)>,
}

#[derive(Clone, Debug, Default)]
pub enum State {
    #[default]
    Unknown,
    Checking,
    UpToDate,
    Available(Release),
    Failed(String),
    /// Installing; what is happening now.
    Installing(String),
    /// Installed this version; the app restarts into it.
    Installed(String),
}

static STATE: Mutex<State> = Mutex::new(State::Unknown);

pub fn state() -> State {
    STATE.lock().unwrap().clone()
}

fn set(s: State) {
    let available = match &s {
        State::Available(r) => Some(r.version.clone()),
        _ => None,
    };
    *STATE.lock().unwrap() = s;
    // The top-bar icon lives on the main thread.
    glib::MainContext::default().invoke(move || crate::tray::update(|t| t.update = available));
}

/// The user turned automatic checks off (Settings switch on the About page).
fn off_file() -> PathBuf {
    crate::config_dir().join("acerultrax/no-update-check")
}

pub fn automatic() -> bool {
    !off_file().exists()
}

pub fn set_automatic(on: bool) {
    let f = off_file();
    if on {
        let _ = std::fs::remove_file(f);
    } else {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, "");
    }
}

/// Check now, then once a day while the app runs (unless turned off).
pub fn start_daily() {
    if automatic() {
        check_now();
    }
    glib::timeout_add_seconds_local(24 * 3600, || {
        if automatic() {
            check_now();
        }
        glib::ControlFlow::Continue
    });
}

pub fn check_now() {
    if matches!(state(), State::Checking | State::Installing(_)) {
        return;
    }
    set(State::Checking);
    std::thread::spawn(|| {
        set(match latest() {
            Ok(r) if newer(&r.version, env!("CARGO_PKG_VERSION")) => State::Available(r),
            Ok(_) => State::UpToDate,
            Err(e) => State::Failed(e),
        })
    });
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .user_agent(concat!("AcerUltraX/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn get_text(url: &str) -> Result<String, String> {
    let mut resp = agent()
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("could not reach GitHub ({e})"))?;
    resp.body_mut().read_to_string().map_err(|e| e.to_string())
}

fn latest() -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_str(&get_text(LATEST)?).map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().ok_or("GitHub sent no version")?;
    let assets = v["assets"].as_array().map(Vec::as_slice).unwrap_or_default();
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        notes: v["body"].as_str().unwrap_or_default().to_string(),
        assets: assets
            .iter()
            .filter_map(|a| {
                Some((a["name"].as_str()?.into(), a["browser_download_url"].as_str()?.into(), a["size"].as_u64()?))
            })
            .collect(),
    })
}

/// Whether version `a` is newer than `b` ("0.1.10" > "0.1.9").
fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| v.split('.').map(|p| p.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
    parse(a) > parse(b)
}

/// How this copy was installed decides how it updates.
#[derive(Clone, Debug, PartialEq)]
pub enum Method {
    /// The download's installer, for this user (`~/.local`): the app updates itself.
    User(PathBuf),
    /// The apt repository: apt updates it, after the system's password prompt.
    Apt,
    /// A package manager the app should not work around: what to do instead.
    Manual(&'static str),
}

pub fn method() -> Method {
    let exe = std::env::current_exe().and_then(|e| e.canonicalize()).unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    method_for(&exe, &home, Path::new("/"))
}

fn method_for(exe: &Path, home: &Path, root: &Path) -> Method {
    if exe.starts_with(home.join(".local/bin")) {
        Method::User(home.join(".local"))
    } else if exe.starts_with("/nix/store") {
        Method::Manual("Update AcerUltraX in your NixOS configuration, then rebuild.")
    } else if exe.starts_with("/usr/local") {
        Method::Manual("Download the new version from GitHub and run its installer: sudo ./install.sh --system")
    } else if root.join("var/lib/dpkg/info/acerultrax.list").exists() {
        Method::Apt
    } else if std::fs::read_dir(root.join("var/lib/pacman/local"))
        .is_ok_and(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("acerultrax")))
    {
        Method::Manual(
            "Rebuild the acerultrax-git package (makepkg -si in packaging/aur), or let your AUR helper update it.",
        )
    } else {
        Method::Manual("Update AcerUltraX the way you installed it.")
    }
}

/// Install `r`, then restart the service and the app into it.
pub fn install(r: Release) {
    let m = method();
    if matches!(m, Method::Manual(_)) || matches!(state(), State::Installing(_)) {
        return;
    }
    set(State::Installing("Starting…".into()));
    std::thread::spawn(move || {
        let done = match m {
            Method::User(_) => install_user(&r),
            Method::Apt => install_apt(),
            Method::Manual(_) => unreachable!(),
        };
        match done {
            Ok(()) => {
                let cli = std::env::current_exe().map(|e| e.with_file_name("acerultrax-cli")).unwrap_or_default();
                let _ = Command::new(cli).arg("restart").status();
                set(State::Installed(r.version.clone()));
                glib::MainContext::default().invoke(relaunch);
            }
            Err(e) => set(State::Failed(e)),
        }
    });
}

fn install_user(r: &Release) -> Result<(), String> {
    let name = format!("acerultrax-{}-x86_64-linux.tar.gz", r.version);
    let asset = |n: &str| r.assets.iter().find(|a| a.0 == n).ok_or(format!("the release has no {n}"));
    let (_, url, size) = asset(&name)?;
    let sums = get_text(&asset("SHA256SUMS")?.1)?;
    let sha = sums
        .lines()
        .find_map(|l| {
            let (h, n) = l.split_once(char::is_whitespace)?;
            (n.trim().trim_start_matches('*') == name).then(|| h.to_string())
        })
        .ok_or("the release's checksums do not list the download")?;
    // download::fetch takes Acer's package records; leaking three short strings once per update is fine.
    let pkg = download::Package {
        name: "AcerUltraX",
        adds: "",
        url: Box::leak(url.clone().into_boxed_str()),
        file: Box::leak(name.clone().into_boxed_str()),
        size: *size,
        sha256: Box::leak(sha.into_boxed_str()),
    };
    let dir = download::cache_dir().join("update");
    let tar = download::fetch(&pkg, &dir, &AtomicBool::new(false), &mut |n| {
        *STATE.lock().unwrap() = State::Installing(format!("Downloading… {}%", n * 100 / (*size).max(1)));
    })
    .map_err(|e| e.to_string())?;
    *STATE.lock().unwrap() = State::Installing("Installing…".into());
    let ok = |c: &mut Command| c.status().is_ok_and(|s| s.success());
    let script = dir.join(format!("acerultrax-{}-x86_64-linux/install.sh", r.version));
    let res = if !ok(Command::new("tar").arg("-xzf").arg(&tar).arg("-C").arg(&dir)) {
        Err("could not unpack the download".to_string())
    } else if !ok(Command::new("sh").arg(script)) {
        Err("the installer failed".to_string())
    } else {
        Ok(())
    };
    let _ = std::fs::remove_dir_all(&dir);
    res
}

fn install_apt() -> Result<(), String> {
    *STATE.lock().unwrap() = State::Installing("Waiting for your password…".into());
    let ok = Command::new("pkexec")
        .args(["sh", "-c", "apt-get update && apt-get install --only-upgrade -y acerultrax"])
        .status()
        .is_ok_and(|s| s.success());
    ok.then_some(()).ok_or_else(|| "apt could not update it (or the password prompt was cancelled)".into())
}

/// Start the new version and quit this one.
fn relaunch() {
    if let Ok(exe) = std::env::current_exe() {
        // The new process must start after this one has left the session bus, or it would just activate us.
        let _ = Command::new("sh").args(["-c", "sleep 1; exec \"$0\""]).arg(exe).spawn();
    }
    if let Some(app) = gtk::gio::Application::default() {
        app.quit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(newer("0.1.10", "0.1.9"));
        assert!(newer("0.2.0", "0.1.2"));
        assert!(!newer("0.1.2", "0.1.2"));
        assert!(!newer("0.1.1", "0.1.2"));
    }

    #[test]
    fn install_methods() {
        let home = Path::new("/home/u");
        let root = std::env::temp_dir().join(format!("acerultrax-update-test-{}", std::process::id()));
        let m = |exe: &str| method_for(Path::new(exe), home, &root);
        assert_eq!(m("/home/u/.local/bin/acerultrax"), Method::User(home.join(".local")));
        assert!(matches!(m("/nix/store/abc-acerultrax/bin/acerultrax"), Method::Manual(_)));
        assert!(matches!(m("/usr/bin/acerultrax"), Method::Manual(_)));
        std::fs::create_dir_all(root.join("var/lib/dpkg/info")).unwrap();
        std::fs::write(root.join("var/lib/dpkg/info/acerultrax.list"), "").unwrap();
        assert_eq!(m("/usr/bin/acerultrax"), Method::Apt);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Installs the latest release from GitHub into a temporary home folder (network; run with `--ignored`, alone:
    /// it changes HOME for this process).
    #[test]
    #[ignore]
    fn installs_latest_release() {
        let home = std::env::temp_dir().join(format!("acerultrax-update-home-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: this test runs alone (`--ignored`), nothing reads the environment concurrently.
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::remove_var("XDG_CACHE_HOME");
        }
        let r = latest().unwrap();
        install_user(&r).unwrap();
        assert!(home.join(".local/bin/acerultrax").exists() && home.join(".local/bin/acerultrax-daemon").exists());
        assert!(!home.join(".cache/acerultrax/update").exists(), "the download is cleaned up");
        std::fs::remove_dir_all(&home).unwrap();
    }
}
