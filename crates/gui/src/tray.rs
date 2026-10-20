//! The top-bar icon (StatusNotifierItem): KDE and most desktops show it; stock GNOME needs the AppIndicator
//! extension. The app works the same without it.

use gtk::prelude::*;
use ksni::menu::{MenuItem, RadioGroup, RadioItem, StandardItem};

use crate::client;

/// The profiles in the menu unless the user picks others (Settings).
pub const DEFAULT_SHOWN: [&str; 4] = ["Music", "Movie", "Game1", "Voice"];

#[derive(Default)]
pub struct AuxTray {
    pub device: Option<String>,
    pub profile: Option<String>,
    /// The four profiles the menu offers.
    pub shown: Vec<String>,
}

fn open_window() {
    gtk::glib::MainContext::default().invoke(|| {
        if let Some(app) = gtk::gio::Application::default() {
            app.activate();
        }
    });
}

thread_local! {
    /// The running icon, and the hold that keeps the app alive with only the icon.
    static TRAY: std::cell::RefCell<Option<(ksni::blocking::Handle<AuxTray>, gtk::gio::ApplicationHoldGuard)>> =
        const { std::cell::RefCell::new(None) };
}

/// Show the icon (if the desktop has a tray). Returns whether it is shown.
pub fn start(app: &gtk::Application) -> bool {
    use ksni::blocking::TrayMethods;
    TRAY.with_borrow_mut(|t| {
        if t.is_none() {
            match AuxTray::default().spawn() {
                Ok(h) => *t = Some((h, app.hold())),
                Err(e) => eprintln!("acerultrax: no top-bar icon: {e}"),
            }
        }
        t.is_some()
    })
}

pub fn stop() {
    if let Some((h, _hold)) = TRAY.take() {
        h.shutdown().wait();
    }
}

pub fn running() -> bool {
    TRAY.with_borrow(|t| t.is_some())
}

/// The device and profile the menu shows now.
pub fn current() -> Option<(Option<String>, Option<String>)> {
    TRAY.with_borrow(|t| t.as_ref().and_then(|(h, _)| h.update(|t| (t.device.clone(), t.profile.clone()))))
}

pub fn update(f: impl FnOnce(&mut AuxTray)) {
    TRAY.with_borrow(|t| {
        if let Some((h, _)) = t {
            h.update(f);
        }
    });
}

impl ksni::Tray for AuxTray {
    fn id(&self) -> String {
        "acerultrax".into()
    }

    fn title(&self) -> String {
        "AcerUltraX".into()
    }

    // A symbolic icon, so the desktop draws it in the panel's colour.
    fn icon_name(&self) -> String {
        "io.github.fawaz7.AcerUltraX-symbolic".into()
    }

    // Installed icons are found in the icon theme; a build run from the source tree uses the tree's.
    fn icon_theme_path(&self) -> String {
        let dev = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/icons");
        let installed = "/usr/share/icons/hicolor/symbolic/apps/io.github.fawaz7.AcerUltraX-symbolic.svg";
        if !std::path::Path::new(installed).exists() && std::path::Path::new(dev).exists() {
            dev.into()
        } else {
            String::new()
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let what = match (&self.profile, &self.device) {
            (Some(p), Some(d)) => format!("{} on {d}", client::label(p)),
            _ => "Service not running".into(),
        };
        ksni::ToolTip { title: "AcerUltraX".into(), description: what, ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        open_window();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let selected =
            self.profile.as_deref().and_then(|p| self.shown.iter().position(|m| m == p)).unwrap_or(self.shown.len());
        let options: Vec<RadioItem> =
            self.shown.iter().map(|m| RadioItem { label: client::label(m), ..Default::default() }).collect();
        vec![
            StandardItem {
                label: self.device.clone().unwrap_or_else(|| "Service not running".into()),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            RadioGroup {
                selected,
                select: Box::new(|t: &mut Self, i| {
                    if let Some(n) = t.shown.get(i).cloned() {
                        client::set_profile(&n, None);
                        t.profile = Some(n);
                    }
                }),
                options,
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Open AcerUltraX".into(),
                activate: Box::new(|_| open_window()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|_| {
                    gtk::glib::MainContext::default().invoke(|| {
                        if let Some(app) = gtk::gio::Application::default() {
                            app.quit();
                        }
                    })
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}
