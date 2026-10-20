//! The top-bar icon (StatusNotifierItem): KDE and most desktops show it; stock GNOME needs the AppIndicator
//! extension. The app works the same without it.

use gtk::prelude::*;
use ksni::menu::{CheckmarkItem, MenuItem, RadioGroup, RadioItem, StandardItem};

use crate::client;

pub const MODES: [&str; 9] = ["Music", "Movie", "Game1", "Game2", "Game3", "Voice", "Custom Audio", "Off", "Plain"];

#[derive(Default)]
pub struct AuxTray {
    pub device: Option<String>,
    pub profile: Option<String>,
    pub compare: bool,
    pub enabled: bool,
    /// The user's own profiles, listed after the modes.
    pub mine: Vec<String>,
}

fn open_window() {
    gtk::glib::MainContext::default().invoke(|| {
        if let Some(app) = gtk::gio::Application::default() {
            app.activate();
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
        ksni::ToolTip { title: "DTS:X Ultra".into(), description: what, ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        open_window();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let names: Vec<String> = MODES.iter().map(|m| m.to_string()).chain(self.mine.iter().cloned()).collect();
        let selected = self.profile.as_deref().and_then(|p| names.iter().position(|m| m == p)).unwrap_or(names.len());
        let options: Vec<RadioItem> =
            names.iter().map(|m| RadioItem { label: client::label(m), ..Default::default() }).collect();
        vec![
            StandardItem {
                label: self.device.clone().unwrap_or_else(|| "Service not running".into()),
                enabled: false,
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "DTS:X Ultra".into(),
                checked: self.enabled,
                activate: Box::new(|t: &mut Self| {
                    t.enabled = !t.enabled;
                    client::set_enabled(t.enabled);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            RadioGroup {
                selected,
                select: Box::new(|t: &mut Self, i| {
                    let names: Vec<String> =
                        MODES.iter().map(|m| m.to_string()).chain(t.mine.iter().cloned()).collect();
                    if let Some(n) = names.get(i) {
                        client::set_profile(n, None);
                        t.profile = Some(n.clone());
                    }
                }),
                options,
            }
            .into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: "Compare with plain sound".into(),
                checked: self.compare,
                activate: Box::new(|t: &mut Self| {
                    t.compare = !t.compare;
                    client::set_compare(t.compare);
                }),
                ..Default::default()
            }
            .into(),
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
