//! `acerultrax`: the app window (sound mode, devices, settings) and the top-bar icon. Audio runs in the
//! background service; this only shows and changes its state, so closing it never affects the sound.

mod client;
mod profiles;
mod tray;
mod update;
mod wizard;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{Align, Orientation, glib};
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
    ("Off", "Off", "Neutral, speakers protected"),
];

/// The mode buttons for the modes this laptop's tuning has (all of them while the service is not reachable).
fn available_modes() -> impl Iterator<Item = &'static (&'static str, &'static str, &'static str)> {
    let known = client::profiles();
    MODES.iter().filter(move |m| known.is_empty() || known.iter().any(|k| k == m.0))
}

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

/// The four profiles in the top-bar menu (one name per line in the app's config folder).
fn tray_profiles_file() -> PathBuf {
    config_dir().join("acerultrax/top-bar-profiles")
}

fn tray_profiles() -> Vec<String> {
    let saved: Vec<String> = std::fs::read_to_string(tray_profiles_file())
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .take(4)
        .collect();
    if saved.len() == 4 {
        return saved;
    }
    // Laptops without some modes (Voice) get the next available ones instead.
    let modes: Vec<&str> = available_modes().map(|m| m.0).collect();
    let mut v: Vec<String> = tray::DEFAULT_SHOWN.iter().filter(|d| modes.contains(d)).map(|s| s.to_string()).collect();
    v.extend(modes.iter().filter(|m| !tray::DEFAULT_SHOWN.contains(m)).take(4 - v.len()).map(|s| s.to_string()));
    v
}

fn set_tray_profiles(names: &[String]) {
    let p = tray_profiles_file();
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    let _ = std::fs::write(&p, names.join("\n") + "\n");
    let shown = names.to_vec();
    tray::update(move |t| t.shown = shown);
}

fn set_tray_autostart(on: bool) {
    let p = tray_autostart_file();
    if on {
        let exe =
            std::env::current_exe().map(|e| importer::desktop_exec_quote(&e)).unwrap_or_else(|_| "acerultrax".into());
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

/// The X mark and "AcerUltraX", as in the sidebar and the setup window.
fn brand(mark: i32, prefix: &str) -> gtk::Box {
    let b = hbox(10);
    let tex = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_static(include_bytes!("mark.png"))).ok();
    let img = gtk::Image::from_paintable(tex.as_ref());
    img.set_pixel_size(mark);
    b.append(&img);
    let l = label("", &[]);
    l.set_markup(&format!("{prefix}Acer<span foreground=\"#22d3ee\">UltraX</span>"));
    b.append(&l);
    b
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
    spatial_card: gtk::Box,
    spatial: gtk::Switch,
    compare: gtk::ToggleButton,
    devices_box: gtk::Box,
    devices_seen: RefCell<Value>,
    /// Set while the UI is being updated from the service, so the handlers do not send it back.
    updating: Cell<bool>,
}

struct SoundPage {
    page: gtk::Box,
    device: gtk::Label,
    chip: gtk::Label,
    power: gtk::Switch,
    grid: gtk::Grid,
    mine_title: gtk::Label,
    mine: gtk::FlowBox,
    spatial_card: gtk::Box,
    spatial: gtk::Switch,
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
    power.set_tooltip_text(Some("Acer:Ultra X on or off"));
    power.update_property(&[gtk::accessible::Property::Label("Acer:Ultra X on or off")]);
    header.append(&power);
    page.append(&header);

    page.append(&label("Sound mode", &["h2"]));
    let grid = gtk::Grid::new();
    grid.set_row_spacing(12);
    grid.set_column_spacing(12);
    grid.set_column_homogeneous(true);
    let mut first: Option<gtk::ToggleButton> = None;
    for (i, (profile, name, hint)) in available_modes().enumerate() {
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

    let spatial_card = hbox(12);
    spatial_card.add_css_class("card");
    let text = vbox(4);
    text.set_hexpand(true);
    text.append(&label("Spatial sound", &[]));
    text.append(&label(
        "Like Windows' spatial sound: 5.1 and 7.1 games and films all around you, on speakers or headphones",
        &["muted", "small"],
    ));
    spatial_card.append(&text);
    let spatial = gtk::Switch::new();
    spatial.set_valign(Align::Center);
    spatial.update_property(&[gtk::accessible::Property::Label("Spatial sound")]);
    spatial_card.append(&spatial);
    spatial_card.set_visible(false);
    page.append(&spatial_card);

    let card = hbox(12);
    card.add_css_class("card");
    let text = vbox(4);
    text.set_hexpand(true);
    text.append(&label("Compare with plain sound", &[]));
    text.append(&label("Same loudness, so you hear only what Acer:Ultra X changes", &["muted", "small"]));
    card.append(&text);
    let compare = gtk::ToggleButton::with_label("Compare");
    compare.add_css_class("outline");
    compare.set_valign(Align::Center);
    card.append(&compare);
    page.append(&card);

    page.append(&label(
        "Volume: use your system's volume control as usual. It applies after Acer:Ultra X, as on Windows.",
        &["muted", "small"],
    ));
    SoundPage { page, device, chip, power, grid, mine_title, mine, spatial_card, spatial, compare }
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
        "speakers" => "Acer's speaker tuning",
        "headphones" => "Acer's headphone tuning",
        "bluetooth" => "Headphones · Acer's headphone tuning",
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
        "Acer:Ultra X is on whenever you log in",
        autostart,
        Box::new(|on| {
            let cli = std::env::current_exe()
                .map(|e| e.with_file_name("acerultrax-cli"))
                .unwrap_or_else(|_| "acerultrax-cli".into());
            let _ = std::process::Command::new(cli).args(["autostart", if on { "on" } else { "off" }]).status();
        }),
    ));
    if let Some(s) = client::status().filter(|s| s["spatial_available"].as_bool() == Some(true)) {
        page.append(&row(
            "Spatial sound: save power",
            "Lighter spatial presets (Windows' \"Best power\"); off = \"Best sound\"",
            s["spatial_power_saving"].as_bool() == Some(true),
            Box::new(client::set_spatial_power_saving),
        ));
    }
    if let (Ok(m), Some(c)) = (importer::Manifest::load(&importer::data_dir()), importer::laptop_codec()) {
        if m.headphones == 0 && importer::download::covers(&c.ssid) {
            page.append(&extras_card(c.ssid));
        }
    }
    page.append(&top_bar_card(&row));
    page
}

/// Builds a settings row: title, hint, initial state, what the switch does.
type RowFn = dyn Fn(&str, &str, bool, Box<dyn Fn(bool)>) -> gtk::Box;

/// The top-bar icon switch, and the four profiles its menu offers.
fn top_bar_card(row: &RowFn) -> gtk::Box {
    let card = vbox(14);
    card.add_css_class("card");
    let picks = vbox(10);
    let on = tray_autostart_file().exists();
    picks.set_visible(on);
    let p2 = picks.clone();
    let switch_row = row(
        "Top bar icon",
        "Switch profiles from the top bar; AcerUltraX stays there after you close this window",
        on,
        Box::new(move |on| {
            set_tray_autostart(on);
            p2.set_visible(on);
            match gtk::gio::Application::default().and_downcast::<gtk::Application>() {
                Some(app) if on => {
                    tray::start(&app);
                }
                _ => tray::stop(),
            }
        }),
    );
    // The row is a card of its own elsewhere; here it is part of this one.
    switch_row.remove_css_class("card");
    card.append(&switch_row);

    picks.append(&gtk::Separator::new(Orientation::Horizontal));
    picks.append(&label("Profiles in the top-bar menu", &[]));
    let names: Vec<String> = available_modes()
        .map(|m| m.0.to_string())
        .chain(client::profiles().into_iter().filter(|n| !profiles::is_builtin(n)))
        .collect();
    let labels: Vec<String> = names.iter().map(|n| client::label(n)).collect();
    let current = tray_profiles();
    let grid = gtk::Grid::new();
    grid.set_column_spacing(12);
    grid.set_column_homogeneous(true);
    let drops: Rc<RefCell<Vec<gtk::DropDown>>> = Rc::default();
    for i in 0..4 {
        let model = gtk::StringList::new(&labels.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        let d = gtk::DropDown::new(Some(model), gtk::Expression::NONE);
        d.update_property(&[gtk::accessible::Property::Label(&format!("Top-bar profile {}", i + 1))]);
        if let Some(k) = current.get(i).and_then(|c| names.iter().position(|n| n == c)) {
            d.set_selected(k as u32);
        }
        let (drops2, names2) = (drops.clone(), names.clone());
        d.connect_selected_notify(move |_| {
            let chosen: Vec<String> =
                drops2.borrow().iter().filter_map(|d| names2.get(d.selected() as usize).cloned()).collect();
            if chosen.len() == 4 {
                set_tray_profiles(&chosen);
            }
        });
        grid.attach(&d, i as i32, 0, 1, 1);
        drops.borrow_mut().push(d);
    }
    picks.append(&grid);
    card.append(&picks);
    card
}

/// Built by, links, and a star.
fn about_page() -> gtk::Box {
    let page = vbox(14);
    page.add_css_class("page");
    page.set_valign(Align::Center);
    page.set_halign(Align::Center);
    let tex = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_static(include_bytes!(
        "../../../data/icons/hicolor/256x256/apps/io.github.fawaz7.AcerUltraX.png"
    )))
    .ok();
    let icon = gtk::Image::from_paintable(tex.as_ref());
    icon.set_pixel_size(128);
    page.append(&icon);
    let center = |l: gtk::Label| {
        l.set_xalign(0.5);
        l.set_justify(gtk::Justification::Center);
        l
    };
    let name = center(label("", &["about-name"]));
    name.set_markup("Acer<span foreground=\"#22d3ee\">UltraX</span>");
    page.append(&name);
    page.append(&center(label(&format!("Version {}", env!("CARGO_PKG_VERSION")), &["muted", "small"])));
    page.append(&center(label("Your Acer laptop's Windows sound, on Linux.", &[])));
    let by = center(label("", &["muted"]));
    by.set_markup("Built with care by <b>Fawaz Al-Ghzawi</b>");
    by.set_margin_top(8);
    page.append(&by);

    let links = hbox(10);
    links.set_halign(Align::Center);
    links.set_margin_top(10);
    let link = |text: &str, uri: &'static str, class: &str| {
        let b = gtk::Button::with_label(text);
        b.add_css_class(class);
        b.connect_clicked(move |b| {
            gtk::UriLauncher::new(uri).launch(
                b.root().and_downcast_ref::<gtk::Window>(),
                gtk::gio::Cancellable::NONE,
                |_| {},
            );
        });
        b
    };
    links.append(&link("★  Star on GitHub", "https://github.com/fawaz7/AcerUltraX", "primary"));
    links.append(&link("Fawaz on GitHub", "https://github.com/fawaz7", "outline"));
    links.append(&link("Report a problem", "https://github.com/fawaz7/AcerUltraX/issues", "outline"));
    page.append(&links);
    page.append(&center(label("Like it? A star helps other Acer owners find it.", &["muted", "small"])));
    page.append(&update_card());

    let legal = center(label(
        "Free and open source under the MIT licence. Not affiliated with or endorsed by Acer.",
        &["muted", "small"],
    ));
    legal.set_margin_top(24);
    legal.set_max_width_chars(60);
    page.append(&legal);
    page
}

/// Updates: status, release notes, the update button (or what to do instead), and the automatic check.
fn update_card() -> gtk::Box {
    let card = vbox(10);
    card.add_css_class("card");
    card.set_margin_top(18);
    card.set_size_request(520, -1);
    let status = label("", &[]);
    let notes = label("", &["muted", "small"]);
    notes.set_wrap(true);
    notes.set_max_width_chars(70);
    let manual = label("", &["muted", "small"]);
    manual.set_wrap(true);
    manual.set_selectable(true);
    let buttons = hbox(10);
    let install = gtk::Button::with_label("Update now");
    install.add_css_class("primary");
    let check = gtk::Button::with_label("Check now");
    check.add_css_class("outline");
    check.connect_clicked(|_| update::check_now());
    install.connect_clicked(|_| {
        if let update::State::Available(r) = update::state() {
            update::install(r);
        }
    });
    buttons.append(&install);
    buttons.append(&check);
    let auto = hbox(12);
    let auto_text = label("Check for updates automatically", &["muted", "small"]);
    auto_text.set_hexpand(true);
    let sw = gtk::Switch::new();
    sw.set_active(update::automatic());
    sw.update_property(&[gtk::accessible::Property::Label("Check for updates automatically")]);
    sw.connect_active_notify(|s| update::set_automatic(s.is_active()));
    auto.append(&auto_text);
    auto.append(&sw);
    for w in [
        status.upcast_ref::<gtk::Widget>(),
        notes.upcast_ref(),
        manual.upcast_ref(),
        buttons.upcast_ref(),
        auto.upcast_ref(),
    ] {
        card.append(w);
    }

    // Follow the background check while the page exists.
    let weak = card.downgrade();
    let mut last = String::new();
    let refresh = move || {
        let Some(_card) = weak.upgrade() else { return glib::ControlFlow::Break };
        let st = update::state();
        let key = format!("{st:?}");
        if key == last {
            return glib::ControlFlow::Continue;
        }
        last = key;
        let method = update::method();
        let (text, available) = match &st {
            update::State::Unknown => ("Updates are not checked automatically.".to_string(), None),
            update::State::Checking => ("Checking for updates…".into(), None),
            update::State::UpToDate => ("AcerUltraX is up to date.".into(), None),
            update::State::Failed(e) => (format!("Could not update: {e}."), None),
            update::State::Installing(what) => (what.clone(), None),
            update::State::Installed(v) => (format!("Version {v} is installed. Restarting…"), None),
            update::State::Available(r) => (format!("Version {} is available.", r.version), Some(r)),
        };
        status.set_text(&text);
        notes.set_text(available.map(|r| release_notes(&r.notes)).as_deref().unwrap_or(""));
        notes.set_visible(available.is_some());
        let manual_text = match (&method, available) {
            (update::Method::Manual(how), Some(_)) => Some(*how),
            _ => None,
        };
        manual.set_text(manual_text.unwrap_or(""));
        manual.set_visible(manual_text.is_some());
        install.set_visible(available.is_some() && !matches!(method, update::Method::Manual(_)));
        check.set_sensitive(!matches!(st, update::State::Checking | update::State::Installing(_)));
        glib::ControlFlow::Continue
    };
    let mut refresh = refresh;
    refresh();
    glib::timeout_add_local(Duration::from_millis(500), refresh);
    card
}

/// GitHub release notes as plain text: the list items, without Markdown marks.
fn release_notes(md: &str) -> String {
    md.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("- ") || l.starts_with("* "))
        .map(|l| format!("• {}", l[2..].replace("**", "").replace('`', "")))
        .take(8)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Offer the DTS app download when setup skipped it.
fn extras_card(ssid: String) -> gtk::Box {
    let card = vbox(10);
    card.add_css_class("card");
    card.append(&label("Headphone models and spatial sound", &[]));
    card.append(&label(
        "Not set up yet. Adds over 500 headphone models to pick from, and Windows' spatial sound for games and \
         films, downloaded from Acer.",
        &["muted", "small"],
    ));
    card.append(&wizard::download_panel(ssid, wizard::Extras::Always, |_| {
        let cli = std::env::current_exe()
            .map(|e| e.with_file_name("acerultrax-cli"))
            .unwrap_or_else(|_| "acerultrax-cli".into());
        let _ = std::process::Command::new(cli).arg("restart").status();
    }));
    card
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
                format!("Acer:Ultra X on · {apps} app{}", if apps == 1 { "" } else { "s" })
            } else if enabled {
                "Off for this device".into()
            } else {
                "Acer:Ultra X off".into()
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
            ui.spatial_card.set_visible(s["spatial_available"].as_bool() == Some(true));
            ui.spatial.set_active(s["spatial"].as_bool() == Some(true));
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
    update_tray(st.as_ref());
}

/// The top-bar menu's device and profile, from a status reply (only touched when they change).
fn update_tray(st: Option<&Value>) {
    if !tray::running() {
        return;
    }
    let (device, profile) = match st {
        Some(s) => (
            s["device_name"].as_str().map(|n| friendly_name(s["kind"].as_str().unwrap_or(""), n)),
            s["profile"].as_str().map(String::from),
        ),
        None => (None, None),
    };
    if tray::current().is_some_and(|(d, p)| d == device && p == profile) {
        return;
    }
    tray::update(move |t| {
        t.device = device;
        t.profile = profile;
    });
}

fn refresh_tray() {
    if tray::running() {
        update_tray(client::status().as_ref());
    }
}

/// The laptop and how well it is supported, for the sidebar.
fn model_text() -> String {
    match importer::laptop_codec() {
        Some(c) => match importer::VERIFIED_MODELS.iter().find(|(s, _)| s.eq_ignore_ascii_case(&c.ssid)) {
            Some((_, model)) => format!("{}\nAcer:Ultra X · supported", model.trim_start_matches("Acer ")),
            None => format!("Acer laptop {}\nAcer:Ultra X · experimental", c.ssid),
        },
        None => "No Acer sound chip found".into(),
    }
}

fn build_window(app: &gtk::Application) -> gtk::ApplicationWindow {
    let mut modes = Vec::new();
    let SoundPage { page: sound, device, chip, power, grid, mine_title, mine, spatial_card, spatial, compare } =
        sound_page(&mut modes);
    let (devices, devices_box) = devices_page();
    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.add_named(&sound, Some("sound"));
    stack.add_named(&profiles::page(), Some("profiles"));
    stack.add_named(&devices, Some("devices"));
    stack.add_named(&settings_page(), Some("settings"));
    stack.add_named(&about_page(), Some("about"));

    let side = vbox(4);
    side.add_css_class("sidebar");
    side.set_size_request(200, -1);
    let b = brand(28, "");
    b.add_css_class("brand");
    side.append(&b);
    let mut group: Option<gtk::ToggleButton> = None;
    for (id, text) in [
        ("sound", "Sound"),
        ("profiles", "Profiles"),
        ("devices", "Devices"),
        ("settings", "Settings"),
        ("about", "About"),
    ] {
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
        spatial_card,
        spatial,
        compare,
        devices_box,
        devices_seen: RefCell::new(Value::Null),
        updating: Cell::new(false),
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
    ui.spatial.connect_active_notify(move |sw| {
        if !ui2.updating.get() {
            client::set_spatial(sw.is_active());
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
    let window_ref: Rc<RefCell<Option<glib::WeakRef<gtk::ApplicationWindow>>>> = Rc::default();
    let w2 = window_ref.clone();
    let ticks = Cell::new(0u32);
    glib::timeout_add_local(Duration::from_secs(1), move || {
        // Hidden (only the top-bar icon): just keep the icon's menu current, every few seconds.
        let visible = w2.borrow().as_ref().and_then(|w| w.upgrade()).is_some_and(|w| w.is_visible());
        ticks.set(ticks.get().wrapping_add(1));
        if visible {
            refresh(&ui2);
        } else if ticks.get() % 4 == 0 {
            refresh_tray();
        }
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
    *window_ref.borrow_mut() = Some(window.downgrade());
    // With the top-bar icon, closing the window keeps the app (and the icon) running.
    window.connect_close_request(|w| {
        if tray::running() {
            w.set_visible(false);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window
}

/// Switch the main window to a page by its sidebar label ("about", "settings", …).
fn show_page(window: &gtk::ApplicationWindow, page: &str) {
    // root box -> sidebar -> nav buttons: activate the one whose label matches.
    let Some(side) = window.child().and_then(|r| r.first_child()) else { return };
    let mut c = side.first_child();
    while let Some(b) = c {
        if let Ok(t) = b.clone().downcast::<gtk::ToggleButton>() {
            if t.label().is_some_and(|l| l.eq_ignore_ascii_case(page)) {
                t.set_active(true);
            }
        }
        c = b.next_sibling();
    }
}

/// Open the app on a page (from the top-bar menu).
pub(crate) fn show_window_page(page: &str) {
    let Some(app) = gtk::gio::Application::default() else { return };
    app.activate();
    if let Some(w) = app.downcast_ref::<gtk::Application>().and_then(|a| a.windows().into_iter().next()) {
        if let Ok(w) = w.downcast::<gtk::ApplicationWindow>() {
            show_page(&w, page);
        }
    }
}

/// Developer aid: save the window as a PNG once it has been drawn, then quit (`ACERULTRAX_SNAPSHOT=out.png`;
/// `ACERULTRAX_PAGE=devices|settings` picks the page).
fn snapshot_and_quit(app: &gtk::Application, window: &gtk::ApplicationWindow, path: PathBuf) {
    if let Some(page) = std::env::var_os("ACERULTRAX_PAGE") {
        show_page(window, &page.to_string_lossy());
    }
    let (app, window) = (app.clone(), window.clone());
    let delay = std::env::var("ACERULTRAX_SNAPSHOT_DELAY").ok().and_then(|d| d.parse().ok()).unwrap_or(1500);
    glib::timeout_add_local_once(Duration::from_millis(delay), move || {
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
        gtk::Window::set_default_icon_name(APP_ID);
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
        if wizard::needed() {
            // First run: set up, then open the app (hold it so closing the wizard window does not quit midway).
            let hold = app.hold();
            let w = wizard::window(app, move |app| {
                let _ = &hold;
                app.activate();
            });
            w.present();
            if let Some(path) = std::env::var_os("ACERULTRAX_SNAPSHOT") {
                snapshot_and_quit(app, &w, PathBuf::from(path));
            }
            return;
        }
        if tray_autostart_file().exists() || hidden {
            tray::start(app);
        }
        if !started.get() {
            update::start_daily();
        }
        let window = build_window(app);
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
