//! The profile maker: the user's own profiles, each a DTS mode plus equaliser and DTS adjustments, with the
//! measured response curve. Changes are saved and take effect a moment after the last one.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{Align, Orientation, glib};
use serde_json::{Value, json};

use crate::{client, hbox, label, vbox};

const BUILTIN: [&str; 9] = ["Music", "Movie", "Game1", "Game2", "Game3", "Voice", "Custom Audio", "Off", "Plain"];
/// Modes a profile can start from (profile name and label).
const BASES: [(&str, &str); 8] = [
    ("music", "Music"),
    ("movie", "Movie"),
    ("game1", "Game 1"),
    ("game2", "Game 2"),
    ("game3", "Game 3"),
    ("voice", "Voice"),
    ("custom_audio", "Custom Audio"),
    ("off", "Off"),
];
const BAND_LABELS: [&str; 10] = ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
const ROOMS: [(&str, &str); 4] = [
    ("entertainment", "Entertainment"),
    ("game", "Game"),
    ("sports", "Sports"),
    ("entertainment-2022", "Entertainment 2022"),
];

struct Editor {
    /// The profile being edited (name, mode, tweaks); its name as last saved.
    profile: RefCell<Value>,
    saved_name: RefCell<String>,
    curve: RefCell<Vec<(f64, f64)>>,
    /// Bumped on every change; a save runs only for the latest one.
    generation: Cell<u64>,
    /// Set while widgets are filled in from a profile.
    loading: Cell<bool>,
    list: gtk::ListBox,
    body: gtk::Box,
    name: gtk::Entry,
    base: gtk::DropDown,
    bands: Vec<gtk::Scale>,
    bass: gtk::Scale,
    dialog: gtk::Scale,
    surround: gtk::Switch,
    room: gtk::DropDown,
    loudness: gtk::Switch,
    area: gtk::DrawingArea,
    note: gtk::Label,
}

fn slider(min: f64, max: f64, step: f64) -> gtk::Scale {
    let s = gtk::Scale::with_range(Orientation::Horizontal, min, max, step);
    s.set_hexpand(true);
    s.set_draw_value(true);
    s.set_value_pos(gtk::PositionType::Right);
    s.set_digits(0);
    s
}

fn control_row(title: &str, w: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = hbox(12);
    row.add_css_class("card");
    let l = label(title, &[]);
    l.set_size_request(150, -1);
    row.append(&l);
    w.set_valign(Align::Center);
    row.append(w);
    row
}

pub fn page() -> gtk::Box {
    let page = hbox(0);
    // Left: the list.
    let side = vbox(8);
    side.add_css_class("page");
    side.set_size_request(240, -1);
    side.append(&label("Profiles", &["title"]));
    let list = gtk::ListBox::new();
    list.add_css_class("profile-list");
    let scroll =
        gtk::ScrolledWindow::builder().child(&list).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
    side.append(&scroll);
    let new_btn = gtk::Button::with_label("+ New profile");
    new_btn.add_css_class("outline");
    side.append(&new_btn);
    page.append(&side);

    // Right: the editor.
    let body = vbox(14);
    body.add_css_class("page");
    body.set_hexpand(true);
    let head = hbox(8);
    let name = gtk::Entry::new();
    name.set_hexpand(true);
    name.add_css_class("title-entry");
    name.update_property(&[gtk::accessible::Property::Label("Profile name")]);
    head.append(&name);
    let use_btn = gtk::Button::with_label("Use on this device");
    use_btn.add_css_class("outline");
    let del_btn = gtk::Button::with_label("Delete");
    del_btn.add_css_class("outline");
    head.append(&use_btn);
    head.append(&del_btn);
    body.append(&head);

    let base_names: Vec<&str> = BASES.iter().map(|b| b.1).collect();
    let base = gtk::DropDown::from_strings(&base_names);
    body.append(&control_row("Based on", &base));

    let area = gtk::DrawingArea::new();
    area.set_content_height(170);
    area.set_hexpand(true);
    let curve_card = vbox(6);
    curve_card.add_css_class("card");
    curve_card.append(&label("Measured response on this output (with your changes)", &["muted", "small"]));
    curve_card.append(&area);
    body.append(&curve_card);

    let eq_card = vbox(8);
    eq_card.add_css_class("card");
    eq_card.append(&label("Equaliser (dB)", &[]));
    let eq_row = gtk::Box::new(Orientation::Horizontal, 6);
    eq_row.set_homogeneous(true);
    let mut bands = Vec::new();
    for l in BAND_LABELS {
        let col = vbox(4);
        let s = gtk::Scale::with_range(Orientation::Vertical, -15.0, 15.0, 0.5);
        s.set_inverted(true);
        s.set_draw_value(true);
        s.set_value_pos(gtk::PositionType::Top);
        s.set_digits(1);
        s.set_size_request(-1, 150);
        s.add_mark(0.0, gtk::PositionType::Left, None);
        s.update_property(&[gtk::accessible::Property::Label(&format!("{l} Hz"))]);
        col.append(&s);
        let bl = label(l, &["muted", "small"]);
        bl.set_xalign(0.5);
        col.append(&bl);
        eq_row.append(&col);
        bands.push(s);
    }
    eq_card.append(&eq_row);
    body.append(&eq_card);

    let bass = slider(0.0, 100.0, 1.0);
    let dialog = slider(0.0, 100.0, 1.0);
    let surround = gtk::Switch::new();
    let room = gtk::DropDown::from_strings(&ROOMS.iter().map(|r| r.1).collect::<Vec<_>>());
    let loudness = gtk::Switch::new();
    body.append(&control_row("Bass", &bass));
    body.append(&control_row("Dialogue clarity", &dialog));
    let sr = control_row("Surround", &surround);
    let room_label = label("Room", &["muted"]);
    room_label.set_hexpand(true);
    room_label.set_xalign(1.0);
    sr.append(&room_label);
    sr.append(&room);
    body.append(&sr);
    body.append(&control_row("Loudness control", &loudness));
    let note = label("", &["muted", "small"]);
    body.append(&note);

    let scroll =
        gtk::ScrolledWindow::builder().child(&body).hexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
    page.append(&scroll);

    let ed = Rc::new(Editor {
        profile: RefCell::new(Value::Null),
        saved_name: RefCell::new(String::new()),
        curve: RefCell::new(Vec::new()),
        generation: Cell::new(0),
        loading: Cell::new(false),
        list,
        body,
        name,
        base,
        bands,
        bass,
        dialog,
        surround,
        room,
        loudness,
        area,
        note,
    });

    let e = ed.clone();
    ed.area.set_draw_func(move |_, cr, w, h| draw_curve(cr, w as f64, h as f64, &e.curve.borrow()));

    // Any change: update the profile and schedule a save.
    let changed = {
        let e = ed.clone();
        Rc::new(move || {
            if !e.loading.get() {
                collect(&e);
                schedule_save(&e);
            }
        })
    };
    for s in ed.bands.iter().chain([&ed.bass, &ed.dialog]) {
        let c = changed.clone();
        s.connect_value_changed(move |_| c());
    }
    for sw in [&ed.surround, &ed.loudness] {
        let c = changed.clone();
        sw.connect_active_notify(move |_| c());
    }
    for d in [&ed.base, &ed.room] {
        let c = changed.clone();
        d.connect_selected_notify(move |_| c());
    }
    let c = changed.clone();
    ed.name.connect_activate(move |_| c());
    let c = changed;
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(move |_| c());
    ed.name.add_controller(focus);

    let e = ed.clone();
    ed.list.connect_row_selected(move |_, row| {
        if let Some(name) =
            row.and_then(|r| r.child()).and_then(|c| c.downcast::<gtk::Label>().ok()).map(|l| l.widget_name())
        {
            select(&e, &name);
        }
    });
    let e = ed.clone();
    new_btn.connect_clicked(move |_| new_profile(&e));
    let e = ed.clone();
    use_btn.connect_clicked(move |_| {
        if let Some(n) = e.profile.borrow()["name"].as_str() {
            client::set_profile(n, None);
            e.note.set_text(&format!("{n} is now used on this device."));
        }
    });
    let e = ed.clone();
    del_btn.connect_clicked(move |_| {
        let name = e.saved_name.borrow().clone();
        if !BUILTIN.contains(&name.as_str()) {
            client::delete_profile(&name);
            fill_list(&e, None);
        }
    });

    fill_list(&ed, None);
    page
}

pub fn is_builtin(name: &str) -> bool {
    BUILTIN.iter().any(|b| b.eq_ignore_ascii_case(name))
}

fn fill_list(ed: &Rc<Editor>, select_name: Option<&str>) {
    while let Some(c) = ed.list.first_child() {
        ed.list.remove(&c);
    }
    let all = client::profiles_full();
    let mine: Vec<&Value> = all.iter().filter(|p| !is_builtin(p["name"].as_str().unwrap_or(""))).collect();
    let header = |t: &str| {
        let row = gtk::ListBoxRow::new();
        row.set_selectable(false);
        row.set_activatable(false);
        row.set_child(Some(&label(t, &["caps"])));
        row
    };
    ed.list.append(&header("YOURS"));
    if mine.is_empty() {
        let row = header("None yet: start one with + New profile");
        ed.list.append(&row);
    }
    let mut to_select = None;
    // Built-in profiles in a natural order: the DTS modes, then Off and Plain.
    let mut builtin: Vec<&Value> = all.iter().filter(|p| is_builtin(p["name"].as_str().unwrap_or(""))).collect();
    builtin.sort_by_key(|p| BUILTIN.iter().position(|b| Some(*b) == p["name"].as_str()));
    for (group, items) in [("", mine), ("BUILT IN", builtin)] {
        if !group.is_empty() {
            ed.list.append(&header(group));
        }
        for p in items {
            let n = p["name"].as_str().unwrap_or("");
            let l = label(&client::label(n), &[]);
            l.set_widget_name(n);
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&l));
            ed.list.append(&row);
            if select_name == Some(n) || (select_name.is_none() && to_select.is_none()) {
                to_select = Some(row);
            }
        }
    }
    match to_select {
        Some(r) => ed.list.select_row(Some(&r)),
        None => ed.body.set_sensitive(false),
    }
}

/// Show a profile in the editor, starting from the mode's factory values where it changes nothing.
fn select(ed: &Rc<Editor>, name: &str) {
    let all = client::profiles_full();
    let Some(p) = all.iter().find(|p| p["name"].as_str() == Some(name)).cloned() else { return };
    let builtin = is_builtin(name);
    let mode = p["mode"].as_str().unwrap_or("music").to_string();
    let defaults = client::mode_defaults(&mode);
    let tw = &p["tweaks"];
    let pick = |k: &str| if tw[k].is_null() { defaults[k].clone() } else { tw[k].clone() };
    ed.loading.set(true);
    ed.name.set_text(&client::label(name));
    if let Some(i) = BASES.iter().position(|b| b.0 == mode) {
        ed.base.set_selected(i as u32);
    }
    let eq = tw["eq"].as_array().cloned().unwrap_or_default();
    for (i, s) in ed.bands.iter().enumerate() {
        s.set_value(eq.get(i).and_then(Value::as_f64).unwrap_or(0.0) / 10.0);
    }
    ed.bass.set_value(pick("bass").as_f64().unwrap_or(0.0));
    ed.dialog.set_value(pick("dialog").as_f64().unwrap_or(0.0));
    ed.surround.set_active(pick("surround").as_bool().unwrap_or(false));
    ed.loudness.set_active(pick("loudness").as_bool().unwrap_or(false));
    let room = pick("room");
    if let Some(i) = ROOMS.iter().position(|r| Some(r.0) == room.as_str()) {
        ed.room.set_selected(i as u32);
    }
    ed.loading.set(false);
    *ed.profile.borrow_mut() = p.clone();
    *ed.saved_name.borrow_mut() = name.to_string();
    ed.body.set_sensitive(true);
    for w in ed.bands.iter().map(|b| b.clone().upcast::<gtk::Widget>()).chain([
        ed.bass.clone().upcast(),
        ed.dialog.clone().upcast(),
        ed.surround.clone().upcast(),
        ed.room.clone().upcast(),
        ed.loudness.clone().upcast(),
        ed.base.clone().upcast(),
        ed.name.clone().upcast(),
    ]) {
        w.set_sensitive(!builtin);
    }
    ed.note.set_text(if builtin {
        "Built-in profiles are Acer's factory tuning. Use + New profile to make your own from one."
    } else {
        "Changes are saved and heard right away when this profile is in use."
    });
    refresh_curve(ed);
}

/// Read the widgets into the profile.
fn collect(ed: &Rc<Editor>) {
    let mode = BASES[ed.base.selected() as usize % BASES.len()].0;
    let eq: Vec<i32> = ed.bands.iter().map(|s| (s.value() * 10.0).round() as i32).collect();
    let mut p = ed.profile.borrow_mut();
    p["name"] = json!(ed.name.text().trim());
    p["mode"] = json!(mode);
    p["tweaks"] = json!({
        "eq": if eq.iter().all(|g| *g == 0) { Value::Null } else { json!(eq) },
        "bass": ed.bass.value().round() as i32,
        "dialog": ed.dialog.value().round() as i32,
        "surround": ed.surround.is_active(),
        "room": ROOMS[ed.room.selected() as usize % ROOMS.len()].0,
        "loudness": ed.loudness.is_active(),
    });
}

fn schedule_save(ed: &Rc<Editor>) {
    let g = ed.generation.get() + 1;
    ed.generation.set(g);
    let e = ed.clone();
    glib::timeout_add_local_once(Duration::from_millis(300), move || {
        if e.generation.get() == g {
            save(&e);
        }
    });
}

fn save(ed: &Rc<Editor>) {
    let p = ed.profile.borrow().clone();
    let new_name = p["name"].as_str().unwrap_or("").to_string();
    let old = ed.saved_name.borrow().clone();
    if let Err(e) = client::save_profile(&p) {
        ed.note.set_text(&e);
        return;
    }
    if new_name != old {
        // A rename: devices that used the old name follow it.
        for d in client::devices() {
            if d["profile"].as_str() == Some(old.as_str()) {
                client::set_profile(&new_name, d["device"].as_str());
            }
        }
        client::delete_profile(&old);
        *ed.saved_name.borrow_mut() = new_name.clone();
        fill_list(ed, Some(&new_name));
        return;
    }
    ed.note.set_text("Saved.");
    refresh_curve(ed);
}

/// Measure the curve off the main thread (about 0.1 s), so editing never stalls; a newer edit's result wins.
fn refresh_curve(ed: &Rc<Editor>) {
    let p = ed.profile.borrow().clone();
    let (ed, g) = (ed.clone(), ed.generation.get());
    gtk::glib::spawn_future_local(async move {
        let Ok(curve) = gtk::gio::spawn_blocking(move || client::response(&p)).await else { return };
        if ed.generation.get() == g {
            *ed.curve.borrow_mut() = curve;
            ed.area.queue_draw();
        }
    });
}

fn new_profile(ed: &Rc<Editor>) {
    // Start from the selected profile's mode, or Music.
    let base = ed.profile.borrow()["mode"].as_str().filter(|m| *m != "plain").unwrap_or("music").to_string();
    let names: Vec<String> =
        client::profiles_full().iter().filter_map(|p| p["name"].as_str().map(String::from)).collect();
    let name = (1..).map(|i| format!("My profile {i}")).find(|n| !names.contains(n)).unwrap();
    let p = json!({"name": name, "mode": base, "tweaks": client::mode_defaults(&base)});
    if client::save_profile(&p).is_ok() {
        fill_list(ed, Some(&name));
    }
}

/// The response curve: 20 Hz to 20 kHz on a log axis, the dB range fitted to the curve.
fn draw_curve(cr: &gtk::cairo::Context, w: f64, h: f64, pts: &[(f64, f64)]) {
    let (lx, rx) = (20f64.log10(), 20000f64.log10());
    let x = |f: f64| (f.max(20.0).log10() - lx) / (rx - lx) * w;
    let (lo, hi) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
    let (lo, hi) =
        if pts.is_empty() { (-12.0, 12.0) } else { ((lo - 3.0).min(-6.0).floor(), (hi + 3.0).max(6.0).ceil()) };
    let y = |db: f64| h - (db - lo) / (hi - lo) * h;
    cr.set_line_width(1.0);
    cr.set_source_rgba(0.6, 0.66, 0.75, 0.25);
    for f in [100.0, 1000.0, 10000.0] {
        cr.move_to(x(f), 0.0);
        cr.line_to(x(f), h);
    }
    let _ = cr.stroke();
    cr.set_source_rgba(0.6, 0.66, 0.75, 0.5);
    cr.move_to(0.0, y(0.0));
    cr.line_to(w, y(0.0));
    let _ = cr.stroke();
    cr.set_font_size(11.0);
    for (f, t) in [(100.0, "100 Hz"), (1000.0, "1 kHz"), (10000.0, "10 kHz")] {
        cr.move_to(x(f) + 4.0, h - 4.0);
        let _ = cr.show_text(t);
    }
    cr.move_to(4.0, y(0.0) - 4.0);
    let _ = cr.show_text("0 dB");
    if pts.is_empty() {
        return;
    }
    cr.set_source_rgb(0.133, 0.827, 0.933);
    cr.set_line_width(2.5);
    for (i, (f, db)) in pts.iter().enumerate() {
        if i == 0 {
            cr.move_to(x(*f), y(*db));
        } else {
            cr.line_to(x(*f), y(*db));
        }
    }
    let _ = cr.stroke();
}
