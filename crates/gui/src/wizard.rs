//! First run: find the laptop, download Acer's DTS files from Acer, import them, start the service.
//!
//! For laptops whose tuning is in the known Acer package ([`importer::download::COVERED`]) this is one click.
//! Otherwise the user downloads Acer's files from its support site; the wizard says which, opens the page, finds
//! the files in Downloads (or lets the user pick them) and does the rest.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{Align, gio, glib};
use importer::download::{self, APP, DRIVER, Stage};

use crate::{hbox, label, vbox};

fn product_name() -> String {
    std::fs::read_to_string("/sys/class/dmi/id/product_name").map(|s| s.trim().to_string()).unwrap_or_default()
}

fn downloads_dir() -> PathBuf {
    // XDG user dirs: ~/.config/user-dirs.dirs has XDG_DOWNLOAD_DIR="$HOME/Downloads".
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    std::fs::read_to_string(home.join(".config/user-dirs.dirs"))
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("XDG_DOWNLOAD_DIR="))
                .map(|v| v.trim_matches('"').replace("$HOME", &home.to_string_lossy()))
        })
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("Downloads"))
}

/// Acer's DTS downloads already in the Downloads folder.
fn found_downloads() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(downloads_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            n.ends_with(".zip") && n.contains("dts")
        })
        .collect();
    v.sort();
    v
}

/// Megabytes as file managers show them.
fn mb(bytes: u64) -> u64 {
    bytes.div_ceil(1_000_000)
}

fn time_left(secs: f64) -> String {
    match secs {
        s if s < 60.0 => "less than a minute left".into(),
        s if s < 90.0 => "about a minute left".into(),
        s => format!("about {} minutes left", (s / 60.0).round()),
    }
}

/// Whether the download panel offers the extras or always gets them.
#[derive(Clone, Copy)]
pub enum Extras {
    Choice(bool),
    Always,
}

/// Download Acer's files and set up, with progress and cancel; `done` runs on the main thread after success.
pub fn download_panel(ssid: String, extras: Extras, done: impl Fn(&importer::Manifest) + 'static) -> gtk::Box {
    let panel = vbox(14);
    let switch = match extras {
        Extras::Choice(on) => {
            let row = hbox(12);
            let text = vbox(2);
            text.set_hexpand(true);
            text.append(&label("Headphone models and spatial sound", &[]));
            text.append(&label(
                &format!(
                    "Over 500 headphone models to pick from, and Windows' spatial sound for games and films. \
                     Adds {} MB to the download.",
                    mb(APP.size)
                ),
                &["muted", "small"],
            ));
            row.append(&text);
            let sw = gtk::Switch::new();
            sw.set_active(on);
            sw.set_valign(Align::Center);
            sw.update_property(&[gtk::accessible::Property::Label("Headphone models and spatial sound")]);
            row.append(&sw);
            panel.append(&row);
            Some(sw)
        }
        Extras::Always => None,
    };
    let go = gtk::Button::new();
    go.add_css_class("primary");
    go.set_halign(Align::Start);
    let bar = gtk::ProgressBar::new();
    bar.set_visible(false);
    let status = label("", &["muted", "small"]);
    status.set_visible(false);
    panel.append(&go);
    panel.append(&bar);
    panel.append(&status);

    let with_extras = {
        let sw = switch.clone();
        move || sw.as_ref().is_none_or(|s| s.is_active())
    };
    let idle_label = {
        let (go, with_extras) = (go.clone(), with_extras.clone());
        move || {
            let total = DRIVER.size + if with_extras() { APP.size } else { 0 };
            go.set_label(&format!("Download and set up · {} MB", mb(total)));
        }
    };
    idle_label();
    if let Some(sw) = &switch {
        let idle_label = idle_label.clone();
        sw.connect_active_notify(move |_| idle_label());
    }

    if std::env::var_os("ACERULTRAX_SNAPSHOT_START").is_some() {
        let go = go.clone();
        glib::idle_add_local_once(move || go.emit_clicked());
    }
    let running: Rc<RefCell<Option<Arc<AtomicBool>>>> = Rc::default();
    let done = Rc::new(done);
    go.connect_clicked(move |go| {
        if let Some(cancel) = running.borrow().as_ref() {
            cancel.store(true, Ordering::Relaxed);
            go.set_sensitive(false);
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *running.borrow_mut() = Some(cancel.clone());
        go.set_label("Cancel");
        go.remove_css_class("primary");
        go.add_css_class("outline");
        if let Some(sw) = &switch {
            sw.set_sensitive(false);
        }
        bar.set_visible(true);
        bar.set_fraction(0.0);
        status.set_visible(true);
        status.set_text("Connecting to Acer…");

        let progress: Arc<Mutex<Option<Stage>>> = Arc::default();
        let (bar2, status2, p2) = (bar.clone(), status.clone(), progress.clone());
        let started = Instant::now();
        let mut first: Option<u64> = None;
        let tick = glib::timeout_add_local(Duration::from_millis(200), move || {
            match *p2.lock().unwrap() {
                Some(Stage::Downloading { done, total }) => {
                    bar2.set_fraction(done as f64 / total as f64);
                    // Speed from this session's bytes (a resumed download starts part-way).
                    let base = *first.get_or_insert(done);
                    let secs = started.elapsed().as_secs_f64();
                    let eta = if secs > 2.0 && done > base {
                        format!(" · {}", time_left((total - done) as f64 / ((done - base) as f64 / secs)))
                    } else {
                        String::new()
                    };
                    status2.set_text(&format!("Downloading from Acer… {} of {} MB{eta}", mb(done), mb(total)));
                }
                Some(Stage::Importing) => {
                    bar2.pulse();
                    status2.set_text("Setting up…");
                }
                None => {}
            }
            glib::ControlFlow::Continue
        });

        let (ssid, extras) = (ssid.clone(), with_extras());
        let (go, bar, status, running, done, switch, idle_label) = (
            go.clone(),
            bar.clone(),
            status.clone(),
            running.clone(),
            done.clone(),
            switch.clone(),
            idle_label.clone(),
        );
        glib::spawn_future_local(async move {
            let data = importer::data_dir();
            let result = gio::spawn_blocking(move || {
                download::setup(&ssid, &data, extras, &cancel, &mut |s| *progress.lock().unwrap() = Some(s))
            })
            .await;
            tick.remove();
            *running.borrow_mut() = None;
            go.set_sensitive(true);
            go.remove_css_class("outline");
            go.add_css_class("primary");
            if let Some(sw) = &switch {
                sw.set_sensitive(true);
            }
            idle_label();
            match result {
                Ok(Ok(m)) => {
                    bar.set_fraction(1.0);
                    go.set_visible(false);
                    status.set_text("All set.");
                    done(&m);
                }
                Ok(Err(download::Error::Cancelled)) => {
                    bar.set_visible(false);
                    status.set_text("Stopped. Next time the download carries on where it stopped.");
                }
                Ok(Err(e)) => {
                    bar.set_visible(false);
                    go.set_label("Try again");
                    status.set_text(&format!("Could not set up: {e}"));
                }
                Err(_) => {
                    bar.set_visible(false);
                    go.set_label("Try again");
                    status.set_text("Setup stopped unexpectedly.");
                }
            }
        });
    });
    panel
}

/// Start the service now and at every login.
fn start_service() {
    let cli =
        std::env::current_exe().map(|e| e.with_file_name("acerultrax-cli")).unwrap_or_else(|_| "acerultrax-cli".into());
    let _ = std::process::Command::new(cli).args(["autostart", "on"]).status();
}

/// Set up from files the user downloaded: the steps on Acer's site, the files found, and the button.
fn manual_card(model: &str, done: Rc<dyn Fn()>, parent: &gtk::ApplicationWindow) -> gtk::Box {
    let card = vbox(10);
    card.add_css_class("card");
    card.append(&label("Set up from Acer's website", &[]));
    card.append(&label(
        &format!(
            "On Acer's support page, search for {}, then download from \"Driver\" → Audio: \"XPERI DTS\", and from \
             \"Application\": \"XPERI DTS Utility\" (it adds headphone models and spatial sound). Leave them zipped.",
            if model.is_empty() { "your model" } else { model }
        ),
        &["muted", "small"],
    ));
    let open = gtk::Button::with_label("Open Acer support");
    open.add_css_class("outline");
    open.set_halign(Align::Start);
    open.connect_clicked(|b| {
        let launcher = gtk::UriLauncher::new("https://www.acer.com/support");
        launcher.launch(b.root().and_downcast_ref::<gtk::Window>(), gio::Cancellable::NONE, |_| {});
    });
    card.append(&open);
    card.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let files_label = label("", &["muted", "small"]);
    card.append(&files_label);
    let row = hbox(8);
    let go = gtk::Button::with_label("Set up");
    go.add_css_class("primary");
    let choose = gtk::Button::with_label("Choose files…");
    choose.add_css_class("outline");
    let rescan = gtk::Button::with_label("Look again");
    rescan.add_css_class("outline");
    row.append(&go);
    row.append(&choose);
    row.append(&rescan);
    card.append(&row);
    let status = label("", &["small"]);
    card.append(&status);

    let files: Rc<RefCell<Vec<PathBuf>>> = Rc::default();
    let show = {
        let (files, files_label, go) = (files.clone(), files_label.clone(), go.clone());
        Rc::new(move || {
            let f = files.borrow();
            files_label.set_text(&if f.is_empty() {
                format!("No DTS downloads in {} yet.", downloads_dir().display())
            } else {
                let names: Vec<String> =
                    f.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
                format!("Found: {}", names.join(", "))
            });
            go.set_sensitive(!f.is_empty());
        })
    };
    *files.borrow_mut() = found_downloads();
    show();

    let (f2, s2) = (files.clone(), show.clone());
    rescan.connect_clicked(move |_| {
        *f2.borrow_mut() = found_downloads();
        s2();
    });
    let (f2, s2, w2) = (files.clone(), show.clone(), parent.clone());
    choose.connect_clicked(move |_| {
        let filter = gtk::FileFilter::new();
        filter.add_pattern("*.zip");
        filter.set_name(Some("Zip files"));
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder().title("Acer's DTS downloads").filters(&filters).build();
        let (f3, s3) = (f2.clone(), s2.clone());
        dialog.open_multiple(Some(&w2), gio::Cancellable::NONE, move |res| {
            if let Ok(list) = res {
                let picked: Vec<PathBuf> =
                    (0..list.n_items()).filter_map(|i| list.item(i)?.downcast::<gio::File>().ok()?.path()).collect();
                if !picked.is_empty() {
                    *f3.borrow_mut() = picked;
                    s3();
                }
            }
        });
    });

    go.connect_clicked(move |b| {
        let Some(c) = importer::laptop_codec() else {
            status.set_text("No Acer sound chip found.");
            return;
        };
        b.set_sensitive(false);
        status.set_text("Setting up… (a few seconds)");
        let srcs = files.borrow().clone();
        let (status, b, done) = (status.clone(), b.clone(), done.clone());
        glib::spawn_future_local(async move {
            let data = importer::data_dir();
            let result = gio::spawn_blocking(move || {
                std::fs::create_dir_all(&data).map_err(|e| e.to_string())?;
                importer::import(&srcs, &c.ssid, &data).map_err(|e| e.to_string())
            })
            .await
            .unwrap_or_else(|_| Err("the import stopped unexpectedly".into()));
            match result {
                Ok(_) => {
                    status.set_text("All set.");
                    done();
                }
                Err(e) => {
                    status.set_text(&format!("Could not set up: {e}"));
                    b.set_sensitive(true);
                }
            }
        });
    });
    card
}

/// The wizard window; `done` runs after a successful setup.
pub fn window(app: &gtk::Application, done: impl Fn(&gtk::Application) + 'static) -> gtk::ApplicationWindow {
    let page = vbox(16);
    page.add_css_class("page");
    page.set_size_request(640, -1);
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Set up AcerUltraX")
        .default_width(720)
        .child(&page)
        .build();
    window.add_css_class("aux");

    let title = crate::brand(36, "Welcome to ");
    title.add_css_class("title");
    page.append(&title);
    page.append(&label("DTS:X Ultra sound for your Acer laptop, as on Windows.", &["muted"]));

    // The laptop.
    let codec = importer::laptop_codec();
    let model = product_name();
    let card = hbox(12);
    card.add_css_class("card");
    let text = vbox(4);
    text.set_hexpand(true);
    text.append(&label(if model.is_empty() { "Your laptop" } else { &model }, &[]));
    let (chip, chip_class, hint) = match &codec {
        Some(c) => match importer::support(&c.ssid) {
            importer::Support::Verified => ("Verified", "", format!("Sound chip {} · tested against Windows", c.ssid)),
            importer::Support::Experimental => (
                "Experimental",
                "warn",
                format!("Sound chip {} · should sound as on Windows; please tell us how it goes", c.ssid),
            ),
        },
        None => ("Not supported", "off", "No Acer sound chip found. AcerUltraX needs an Acer laptop with DTS.".into()),
    };
    text.append(&label(&hint, &["muted", "small"]));
    card.append(&text);
    let chip = label(chip, &["chip"]);
    if !chip_class.is_empty() {
        chip.add_css_class(chip_class);
    }
    chip.set_valign(Align::Center);
    card.append(&chip);
    page.append(&card);

    let done = Rc::new(done);
    let finish: Rc<dyn Fn()> = {
        let (w, app, done) = (window.clone(), app.clone(), done.clone());
        Rc::new(move || {
            start_service();
            // The top-bar icon is on from the start, as on Windows.
            crate::set_tray_autostart(true);
            w.close();
            done(&app);
        })
    };
    let manual = manual_card(&model, finish.clone(), &window);
    match codec.as_ref().filter(|c| download::covers(&c.ssid)) {
        Some(c) => {
            // One click: download from Acer.
            let card = vbox(14);
            card.add_css_class("card");
            card.append(&label("Get DTS from Acer", &[]));
            card.append(&label(
                "AcerUltraX downloads your laptop's DTS files directly from Acer and keeps only what it uses \
                 (about 20 MB).",
                &["muted", "small"],
            ));
            card.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            let finish = finish.clone();
            card.append(&download_panel(c.ssid.clone(), Extras::Choice(true), move |_| finish()));
            page.append(&card);

            let revealer = gtk::Revealer::builder().child(&manual).reveal_child(false).build();
            let other = gtk::Button::with_label("I already have Acer's files");
            other.add_css_class("link");
            other.set_halign(Align::Start);
            let r2 = revealer.clone();
            other.connect_clicked(move |_| r2.set_reveal_child(!r2.reveals_child()));
            page.append(&other);
            page.append(&revealer);
        }
        None => {
            if codec.is_some() {
                page.append(&label(
                    "AcerUltraX does not know Acer's download for this laptop yet, so it cannot fetch it by itself. \
                     It takes two downloads from Acer's website:",
                    &["muted", "small"],
                ));
            }
            page.append(&manual);
        }
    }

    // Wrap the explanations at a comfortable width instead of stretching the window.
    fn wrap(w: &gtk::Widget) {
        let mut c = w.first_child();
        while let Some(i) = c {
            if let Some(l) = i.downcast_ref::<gtk::Label>() {
                l.set_max_width_chars(70);
            }
            wrap(&i);
            c = i.next_sibling();
        }
    }
    wrap(page.upcast_ref());
    if std::env::var_os("ACERULTRAX_SNAPSHOT_MANUAL").is_some() {
        let mut c = page.first_child();
        while let Some(w) = c {
            if let Some(r) = w.downcast_ref::<gtk::Revealer>() {
                r.set_reveal_child(true);
            }
            c = w.next_sibling();
        }
    }
    window
}

/// Whether the DTS files are set up.
pub fn needed() -> bool {
    importer::Manifest::load(&importer::data_dir()).is_err()
}
