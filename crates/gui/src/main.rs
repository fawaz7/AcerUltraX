//! `acerultrax`: the app window (sound mode, devices, settings) and the top-bar icon. Audio runs in the
//! background service; this only shows and changes its state, so closing it never affects the sound.

mod client;
mod profiles;
mod tray;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{Align, Orientation, glib};
use ksni::blocking::TrayMethods;
use serde_json::Value;

const APP_ID: &str = "io.github.fawaz7.AcerUltraX";

/// (profile, label, hint) for the mode buttons.
const MODES: [(&str, &str, &str); 8] = [
    ("Music", "Music", "For music"),
    ("Movie", "Movie", "For films and series"),
    ("Game1", "Game 1", "Game preset 1"),
    ("Game2", "Game 2", "Game preset 2"),
    ("Game3", "Game 3", "Game preset 3"),
    ("Voice", "Voice", "For calls and speech"),
    ("Custom Audio", "Custom", "Acer's custom preset"),
    ("Off", "Off", "DTS neutral, speakers protected"),
];

fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"))
}

/// The top-bar icon is on when the app starts (hidden) at login.
fn tray_autostart_file() -> PathBuf {
    config_dir().join("autostart/acerultrax-tray.desktop")
}

fn set_tray_autostart(on: bool) {
    let p = tray_autostart_file();
    if on {
        let exe = std::env::current_exe().map(|e| e.display().to_string()).unwrap_or_else(|_| "acerultrax".into());
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(
            &p,
            format!(
                "[Desktop Entry]\nType=Application\nName=AcerUltraX\nExec={exe} --hidden\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n"
            ),
        );
    } else {
        let _ = std::fs::remove_file(&p);
    }
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_wrap(true);
    for c in classes {
        l.add_css_class(c);
    }
    l
}

fn vbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(Orientation::Vertical, spacing)
}

fn hbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(Orientation::Horizontal, spacing)
}

/// Everything the window shows that changes with the service's state.
struct Ui {
    device: gtk::Label,
    chip: gtk::Label,
    power: gtk::Switch,
    grid: gtk::Grid,
    modes: Vec<(String, gtk::ToggleButton)>,
    /// Buttons for the user's own profiles on the Sound page, and the names they were made for.
    mine_title: gtk::Label,
    mine: gtk::FlowBox,
    mine_buttons: RefCell<Vec<(String, gtk::ToggleButton)>>,
    compare: gtk::ToggleButton,
    devices_box: gtk::Box,
    devices_seen: RefCell<Value>,
    /// Set while the UI is being updated from the service, so the handlers do not send it back.
    updating: Cell<bool>,
    tray: Option<ksni::blocking::Handle<tray::AuxTray>>,
}

struct SoundPage {
    page: gtk::Box,
    device: gtk::Label,
    chip: gtk::Label,
    power: gtk::Switch,
    grid: gtk::Grid,
    mine_title: gtk::Label,
    mine: gtk::FlowBox,
    compare: gtk::ToggleButton,
}

fn sound_page(ui_modes: &mut Vec<(String, gtk::ToggleButton)>) -> SoundPage {
    let page = vbox(24);
    page.add_css_class("page");

    let header = hbox(16);
    let who = vbox(2);
    who.append(&label("PLAYING ON", &["caps"]));
    let device = label("…", &["title"]);
    who.append(&device);
    who.set_hexpand(true);
    header.append(&who);
    let chip = label("", &["chip"]);
    chip.set_valign(Align::Center);
    header.append(&chip);
    let power = gtk::Switch::new();
    power.set_valign(Align::Center);
    power.set_tooltip_text(Some("DTS:X Ultra on or off"));
    power.update_property(&[gtk::accessible::Property::Label("DTS:X Ultra on or off")]);
    header.append(&power);
    page.append(&header);

    page.append(&label("Sound mode", &["h2"]));
    let grid = gtk::Grid::new();
    grid.set_row_spacing(12);
    grid.set_column_spacing(12);
    grid.set_column_homogeneous(true);
    let mut first: Option<gtk::ToggleButton> = None;
    for (i, (profile, name, hint)) in MODES.iter().enumerate() {
        let inner = vbox(6);
        inner.append(&label(name, &["name"]));
        inner.append(&label(hint, &["muted", "small"]));
        let b = gtk::ToggleButton::new();
        b.set_child(Some(&inner));
        b.add_css_class("mode");
        if let Some(f) = &first {
            b.set_group(Some(f));
        } else {
            first = Some(b.clone());
        }
        grid.attach(&b, (i % 4) as i32, (i / 4) as i32, 1, 1);
        ui_modes.push((profile.to_string(), b));
    }
    page.append(&grid);
    let mine_title = label("Your profiles", &["h2"]);
    let mine = gtk::FlowBox::new();
    mine.set_selection_mode(gtk::SelectionMode::None);
    mine.set_column_spacing(12);
    mine.set_row_spacing(12);
    mine.set_max_children_per_line(4);
    mine.set_homogeneous(true);
    page.append(&mine_title);
    page.append(&mine);

    let card = hbox(12);
    card.add_css_class("card");
    let text = vbox(4);
    text.set_hexpand(true);
    text.append(&label("Compare with plain sound", &[]));
    text.append(&label("Same loudness, so you hear only what DTS changes", &["muted", "small"]));
    card.append(&text);
    let compare = gtk::ToggleButton::with_label("Compare");
    compare.add_css_class("outline");
    compare.set_valign(Align::Center);
    card.append(&compare);
    page.append(&card);

    page.append(&label(
        "Volume: use your system's volume control as usual. It applies after DTS, as on Windows.",
        &["muted", "small"],
    ));
    SoundPage { page, device, chip, power, grid, mine_title, mine, compare }
}

fn devices_page() -> (gtk::Box, gtk::Box) {
    let page = vbox(20);
    page.add_css_class("page");
    page.append(&label("Devices", &["title"]));
    page.append(&label(
        "Each output keeps its own profile. AcerUltraX switches when you plug in or connect one.",
        &["muted", "small"],
    ));
    let list = vbox(8);
    let scroll =
        gtk::ScrolledWindow::builder().child(&list).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
    page.append(&scroll);
    (page, list)
}

fn kind_text(kind: &str) -> &'static str {
    match kind {
        "speakers" => "DTS speaker tuning",
        "headphones" => "DTS headphone tuning",
        "bluetooth" => "Headphones · DTS headphone tuning",
        "bluetooth-speaker" => "Speaker · plain sound unless you pick a mode",
        _ => "Plain sound unless you pick a mode",
    }
}

/// What to call the output a status or device entry describes.
pub fn friendly_name(kind: &str, name: &str) -> String {
    match kind {
        "speakers" => "Laptop speakers".into(),
        "headphones" => "Headphones (3.5 mm jack)".into(),
        _ => name.into(),
    }
}

/// "Headphones: <model>" with a searchable list, as Windows' DTS app offers (its EQ replaces Acer's default
/// headphone EQ). Wired: Acer's default first; Bluetooth/USB: automatic recognition first.
fn headphone_picker(d: &Value, models: &[Value]) -> gtk::Box {
    let wired = d["kind"].as_str() == Some("headphones");
    let mut ids: Vec<Option<String>> = vec![if wired { Some("default".into()) } else { None }];
    let mut names =
        vec![if wired { "Acer's headphone tuning".to_string() } else { "Recognise automatically".to_string() }];
    if !wired {
        ids.push(Some("default".into()));
        names.push("Acer's headphone tuning".into());
    }
    for m in models {
        ids.push(m["id"].as_str().map(String::from));
        names.push(
            format!("{} {}", m["brand"].as_str().unwrap_or(""), m["name"].as_str().unwrap_or("")).trim().to_string(),
        );
    }
    let drop = gtk::DropDown::from_strings(&names.iter().map(String::as_str).collect::<Vec<_>>());
    drop.set_enable_search(true);
    drop.set_search_match_mode(gtk::StringFilterMatchMode::Substring);
    drop.set_expression(Some(gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        None::<gtk::Expression>,
        "string",
    )));
    drop.update_property(&[gtk::accessible::Property::Label("Headphone model")]);
    // The automatic choice shows as "Recognise automatically"; a chosen model (or "default") as itself.
    let current =
        if d["headphone_auto"].as_bool() == Some(true) { None } else { d["headphone"].as_str().map(String::from) };
    let current = if wired && current.is_none() { Some("default".into()) } else { current };
    if let Some(i) = ids.iter().position(|x| *x == current) {
        drop.set_selected(i as u32);
    }
    let device = d["device"].as_str().unwrap_or("").to_string();
    drop.connect_selected_notify(move |dd| {
        if let Some(id) = ids.get(dd.selected() as usize) {
            client::set_headphone(id.as_deref(), &device);
        }
    });
    let row = hbox(8);
    row.set_margin_top(6);
    row.append(&label("Headphones:", &["muted", "small"]));
    row.append(&drop);
    row
}

fn fill_devices(ui: &Rc<Ui>, devices: &[Value]) {
    while let Some(c) = ui.devices_box.first_child() {
        ui.devices_box.remove(&c);
    }
    let profiles = client::profiles();
    let labels: Vec<String> = profiles.iter().map(|p| client::label(p)).collect();
    let models = client::headphones();
    let group_of = |d: &Value| match d["kind"].as_str().unwrap_or("") {
        "speakers" | "headphones" => "LAPTOP",
        "bluetooth" | "bluetooth-speaker" => "BLUETOOTH",
        _ => "OTHER OUTPUTS",
    };
    let mut last = "";
    for d in devices {
        let group = group_of(d);
        if group != last {
            let h = label(group, &["caps"]);
            h.set_margin_top(if last.is_empty() { 0 } else { 12 });
            ui.devices_box.append(&h);
            last = group;
        }
        let row = hbox(16);
        row.add_css_class("card");
        let text = vbox(2);
        text.set_hexpand(true);
        let kind = d["kind"].as_str().unwrap_or("");
        let name = friendly_name(kind, d["device_name"].as_str().unwrap_or(""));
        text.append(&label(&name, &[]));
        let state = if d["active"].as_bool() == Some(true) {
            "Playing now"
        } else if d["present"].as_bool() == Some(true) {
            "Ready"
        } else if group == "BLUETOOTH" {
            "Not connected"
        } else if kind == "speakers" {
            "Not in use"
        } else {
            "Not plugged in"
        };
        text.append(&label(&format!("{state} · {}", kind_text(kind)), &["muted", "small"]));
        if d["wearable"].as_bool() == Some(true) && !models.is_empty() {
            text.append(&headphone_picker(d, &models));
        }
        row.append(&text);
        let drop = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        drop.set_valign(Align::Center);
        drop.update_property(&[gtk::accessible::Property::Label(&format!("Profile for {name}"))]);
        if let Some(i) = profiles.iter().position(|p| Some(p.as_str()) == d["profile"].as_str()) {
            drop.set_selected(i as u32);
        }
        let device = d["device"].as_str().unwrap_or("").to_string();
        let profiles = profiles.clone();
        drop.connect_selected_notify(move |dd| {
            if let Some(p) = profiles.get(dd.selected() as usize) {
                client::set_profile(p, Some(&device));
            }
        });
        row.append(&drop);
        ui.devices_box.append(&row);
    }
}

fn settings_page() -> gtk::Box {
    let page = vbox(16);
    page.add_css_class("page");
    page.append(&label("Settings", &["title"]));

    let row = |title: &str, hint: &str, on: bool, f: Box<dyn Fn(bool)>| {
        let card = hbox(12);
        card.add_css_class("card");
        let text = vbox(4);
        text.set_hexpand(true);
        text.append(&label(title, &[]));
        text.append(&label(hint, &["muted", "small"]));
        card.append(&text);
        let sw = gtk::Switch::new();
        sw.set_active(on);
        sw.set_valign(Align::Center);
        sw.update_property(&[gtk::accessible::Property::Label(title)]);
        sw.connect_active_notify(move |s| f(s.is_active()));
        card.append(&sw);
        card
    };
    let autostart = std::process::Command::new("systemctl")
        .args(["--user", "is-enabled", "--quiet", "acerultrax.service"])
        .status()
        .is_ok_and(|s| s.success())
        || config_dir().join("autostart/acerultrax-daemon.desktop").exists();
    page.append(&row(
        "Start with my session",
        "DTS:X Ultra is on whenever you log in",
        autostart,
        Box::new(|on| {
            let cli = std::env::current_exe()
                .map(|e| e.with_file_name("acerultrax-cli"))
                .unwrap_or_else(|_| "acerultrax-cli".into());
            let _ = std::process::Command::new(cli).args(["autostart", if on { "on" } else { "off" }]).status();
        }),
    ));
    page.append(&row(
        "Icon in the top bar",
        "Quick mode switching; keeps AcerUltraX in the top bar after you close this window",
        tray_autostart_file().exists(),
        Box::new(set_tray_autostart),
    ));
    page
}

/// One button per user profile, in the same group as the mode buttons; rebuilt when the profiles change.
fn fill_mine(ui: &Rc<Ui>) {
    let names: Vec<String> = client::profiles_full()
        .iter()
        .filter_map(|p| p["name"].as_str().map(String::from))
        .filter(|n| !profiles::is_builtin(n))
        .collect();
    if ui.mine_buttons.borrow().iter().map(|(n, _)| n).eq(names.iter()) {
        return;
    }
    while let Some(c) = ui.mine.first_child() {
        ui.mine.remove(&c);
    }
    let mut v = Vec::new();
    for n in names {
        let inner = vbox(6);
        inner.append(&label(&n, &["name"]));
        inner.append(&label("Your profile", &["muted", "small"]));
        let b = gtk::ToggleButton::new();
        b.set_child(Some(&inner));
        b.add_css_class("mode");
        b.set_group(ui.modes.first().map(|m| &m.1));
        let (ui2, name) = (ui.clone(), n.clone());
        b.connect_toggled(move |b| {
            if b.is_active() && !ui2.updating.get() {
                client::set_profile(&name, None);
            }
        });
        ui.mine.insert(&b, -1);
        v.push((n, b));
    }
    ui.mine_title.set_visible(!v.is_empty());
    ui.mine.set_visible(!v.is_empty());
    *ui.mine_buttons.borrow_mut() = v;
}

fn refresh(ui: &Rc<Ui>) {
    let st = client::status();
    ui.updating.set(true);
    match &st {
        Some(s) => {
            match s["device_name"].as_str() {
                Some(n) => ui.device.set_text(&friendly_name(s["kind"].as_str().unwrap_or(""), n)),
                None => ui.device.set_text("No output device"),
            }
            let profile = s["profile"].as_str().unwrap_or("");
            let enabled = s["enabled"].as_bool() != Some(false);
            ui.power.set_active(enabled);
            ui.grid.set_sensitive(enabled);
            let on = enabled && s["mode"].as_str().is_some();
            let apps = s["streams"].as_u64().unwrap_or(0);
            ui.chip.set_text(&if on {
                format!("DTS:X Ultra on · {apps} app{}", if apps == 1 { "" } else { "s" })
            } else if enabled {
                "DTS off for this device".into()
            } else {
                "DTS off".into()
            });
            if on {
                ui.chip.remove_css_class("off")
            } else {
                ui.chip.add_css_class("off")
            }
            fill_mine(ui);
            let buttons = ui.mine_buttons.borrow();
            let all = ui.modes.iter().chain(buttons.iter());
            match all.clone().find(|(p, _)| p == profile) {
                Some((_, b)) => b.set_active(true),
                None => all.for_each(|(_, b)| b.set_active(false)),
            }
            ui.compare.set_active(s["compare"].as_bool() == Some(true));
            ui.compare.set_label(if ui.compare.is_active() { "Hearing plain" } else { "Compare" });
        }
        None => {
            ui.device.set_text("AcerUltraX service is not running");
            ui.chip.set_text("Off");
            ui.chip.add_css_class("off");
        }
    }
    let devices = client::devices();
    let devices_v = Value::Array(devices.clone());
    if *ui.devices_seen.borrow() != devices_v {
        fill_devices(ui, &devices);
        *ui.devices_seen.borrow_mut() = devices_v;
    }
    ui.updating.set(false);
    if let Some(t) = &ui.tray {
        let (device, profile, compare, enabled) = match &st {
            Some(s) => (
                s["device_name"].as_str().map(|n| friendly_name(s["kind"].as_str().unwrap_or(""), n)),
                s["profile"].as_str().map(String::from),
                s["compare"].as_bool() == Some(true),
                s["enabled"].as_bool() != Some(false),
            ),
            None => (None, None, false, false),
        };
        let mine: Vec<String> = ui.mine_buttons.borrow().iter().map(|(n, _)| n.clone()).collect();
        t.update(move |t| {
            t.device = device;
            t.profile = profile;
            t.compare = compare;
            t.enabled = enabled;
            t.mine = mine;
        });
    }
}

/// The laptop and how well it is supported, for the sidebar.
fn model_text() -> String {
    match importer::laptop_codec() {
        Some(c) => match importer::VERIFIED_MODELS.iter().find(|(s, _)| s.eq_ignore_ascii_case(&c.ssid)) {
            Some((_, model)) => format!("{}\nDTS:X Ultra · supported", model.trim_start_matches("Acer ")),
            None => format!("Acer laptop {}\nDTS:X Ultra · experimental", c.ssid),
        },
        None => "No Acer sound chip found".into(),
    }
}

fn build_window(app: &gtk::Application, tray: Option<ksni::blocking::Handle<tray::AuxTray>>) -> gtk::ApplicationWindow {
    let mut modes = Vec::new();
    let SoundPage { page: sound, device, chip, power, grid, mine_title, mine, compare } = sound_page(&mut modes);
    let (devices, devices_box) = devices_page();
    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.add_named(&sound, Some("sound"));
    stack.add_named(&profiles::page(), Some("profiles"));
    stack.add_named(&devices, Some("devices"));
    stack.add_named(&settings_page(), Some("settings"));

    let side = vbox(4);
    side.add_css_class("sidebar");
    side.set_size_request(200, -1);
    let brand = label("", &["brand"]);
    brand.set_markup("Acer<span foreground=\"#22d3ee\">UltraX</span>");
    side.append(&brand);
    let mut group: Option<gtk::ToggleButton> = None;
    for (id, text) in [("sound", "Sound"), ("profiles", "Profiles"), ("devices", "Devices"), ("settings", "Settings")] {
        let b = gtk::ToggleButton::with_label(text);
        b.add_css_class("nav");
        if let Some(l) = b.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
            l.set_xalign(0.0);
        }
        match &group {
            Some(g) => b.set_group(Some(g)),
            None => {
                b.set_active(true);
                group = Some(b.clone());
            }
        }
        let st = stack.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                st.set_visible_child_name(id);
            }
        });
        side.append(&b);
    }
    let spacer = vbox(0);
    spacer.set_vexpand(true);
    side.append(&spacer);
    side.append(&label(&model_text(), &["model", "muted", "small"]));

    let root = hbox(0);
    root.append(&side);
    root.append(&stack);

    let ui = Rc::new(Ui {
        device,
        chip,
        power,
        grid,
        modes,
        mine_title,
        mine,
        mine_buttons: RefCell::new(Vec::new()),
        compare,
        devices_box,
        devices_seen: RefCell::new(Value::Null),
        updating: Cell::new(false),
        tray,
    });
    for (p, b) in &ui.modes {
        let (ui2, p) = (ui.clone(), p.clone());
        b.connect_toggled(move |b| {
            if b.is_active() && !ui2.updating.get() {
                client::set_profile(&p, None);
                refresh(&ui2);
            }
        });
    }
    let ui2 = ui.clone();
    ui.power.connect_active_notify(move |sw| {
        if !ui2.updating.get() {
            client::set_enabled(sw.is_active());
            refresh(&ui2);
        }
    });
    let ui2 = ui.clone();
    ui.compare.connect_toggled(move |b| {
        if !ui2.updating.get() {
            client::set_compare(b.is_active());
            refresh(&ui2);
        }
    });
    refresh(&ui);
    let ui2 = ui.clone();
    glib::timeout_add_local(Duration::from_secs(1), move || {
        refresh(&ui2);
        glib::ControlFlow::Continue
    });

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("AcerUltraX")
        .default_width(1040)
        .default_height(700)
        .child(&root)
        .build();
    window.add_css_class("aux");
    // With the top-bar icon, closing the window keeps the app (and the icon) running.
    window.set_hide_on_close(ui.tray.is_some());
    window
}

/// Developer aid: save the window as a PNG once it has been drawn, then quit (`ACERULTRAX_SNAPSHOT=out.png`;
/// `ACERULTRAX_PAGE=devices|settings` picks the page).
fn snapshot_and_quit(app: &gtk::Application, window: &gtk::ApplicationWindow, path: PathBuf) {
    if let Some(page) = std::env::var_os("ACERULTRAX_PAGE") {
        let mut w = window.child();
        // root box -> sidebar -> nav buttons: activate the one whose label matches.
        if let Some(side) = w.take().and_then(|r| r.first_child()) {
            let mut c = side.first_child();
            while let Some(b) = c {
                if let Ok(t) = b.clone().downcast::<gtk::ToggleButton>() {
                    if t.label().is_some_and(|l| l.eq_ignore_ascii_case(&page.to_string_lossy())) {
                        t.set_active(true);
                    }
                }
                c = b.next_sibling();
            }
        }
    }
    let (app, window) = (app.clone(), window.clone());
    glib::timeout_add_local_once(Duration::from_millis(1500), move || {
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let (w, h) = (window.width() as f64, window.height() as f64);
        let snap = gtk::Snapshot::new();
        paintable.snapshot(&snap, w, h);
        if let (Some(node), Some(native)) = (snap.to_node(), window.native()) {
            let tex = native.renderer().expect("renderer").render_texture(&node, None);
            let _ = tex.save_to_png(&path);
        }
        app.quit();
    });
}

fn main() -> glib::ExitCode {
    let hidden = std::env::args().any(|a| a == "--hidden");
    let app = gtk::Application::builder().application_id(APP_ID).build();
    let started = Rc::new(Cell::new(false));
    app.connect_startup(|_| {
        let css = gtk::CssProvider::new();
        css.load_from_string(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("no display"),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
    app.connect_activate(move |app| {
        if let Some(w) = app.windows().first() {
            w.present();
            return;
        }
        let tray = if tray_autostart_file().exists() || hidden {
            match tray::AuxTray::default().spawn() {
                Ok(h) => Some(h),
                Err(e) => {
                    eprintln!("acerultrax: no top-bar icon: {e}");
                    None
                }
            }
        } else {
            None
        };
        if tray.is_some() {
            // Keep running with only the icon when the window is closed.
            std::mem::forget(app.hold());
        }
        let window = build_window(app, tray);
        if !(hidden && !started.get()) {
            window.present();
        }
        if let Some(path) = std::env::var_os("ACERULTRAX_SNAPSHOT") {
            snapshot_and_quit(app, &window, PathBuf::from(path));
        }
        started.set(true);
    });
    // GTK must not see our own --hidden flag.
    app.run_with_args(&std::env::args().filter(|a| a != "--hidden").collect::<Vec<_>>())
}
