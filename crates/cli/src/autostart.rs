//! Start the background service with the session: a systemd user service where systemd runs the session,
//! otherwise an XDG autostart entry (works on any desktop).

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

const UNIT: &str = "acerultrax.service";

fn home(sub: &str) -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(sub)
}

fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home(".config"))
}

fn systemd_user() -> bool {
    Command::new("systemctl").args(["--user", "is-system-running"]).output().is_ok_and(|o| !o.stdout.is_empty())
}

fn systemctl(args: &[&str]) -> Result<()> {
    let st = Command::new("systemctl").arg("--user").args(args).status().context("running systemctl")?;
    if !st.success() {
        bail!("systemctl --user {} failed", args.join(" "));
    }
    Ok(())
}

/// The daemon next to this program (a build tree or a package's /usr/bin).
fn daemon_path() -> Result<PathBuf> {
    let p = std::env::current_exe()?.with_file_name("acerultrax-daemon");
    if !p.exists() {
        bail!("{} not found", p.display());
    }
    Ok(p)
}

fn desktop_file() -> PathBuf {
    config_dir().join("autostart/acerultrax-daemon.desktop")
}

pub fn enable() -> Result<()> {
    if systemd_user() {
        // A package installs the unit system-wide; otherwise write one for this user.
        if !PathBuf::from("/usr/lib/systemd/user").join(UNIT).exists() {
            let unit = include_str!("../../../packaging/acerultrax.service")
                .replace("/usr/bin/acerultrax-daemon", &daemon_path()?.display().to_string());
            let p = config_dir().join("systemd/user").join(UNIT);
            std::fs::create_dir_all(p.parent().unwrap())?;
            std::fs::write(&p, unit)?;
            systemctl(&["daemon-reload"])?;
        }
        systemctl(&["enable", "--now", UNIT])?;
        println!("AcerUltraX now starts with your session (systemd user service), and is running.");
    } else {
        let d = daemon_path()?;
        let p = desktop_file();
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(
            &p,
            format!(
                "[Desktop Entry]\nType=Application\nName=AcerUltraX service\nExec={}\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
                d.display()
            ),
        )?;
        Command::new(&d).spawn().context("starting the service")?;
        println!("AcerUltraX now starts when you log in, and is running.");
    }
    Ok(())
}

pub fn disable() -> Result<()> {
    if systemd_user() {
        systemctl(&["disable", "--now", UNIT])?;
    }
    let _ = std::fs::remove_file(desktop_file());
    println!("AcerUltraX no longer starts with your session.");
    Ok(())
}

pub fn enabled() -> bool {
    if systemd_user() {
        return Command::new("systemctl")
            .args(["--user", "is-enabled", "--quiet", UNIT])
            .status()
            .is_ok_and(|s| s.success());
    }
    desktop_file().exists()
}
