//! First run: find the laptop, get Acer's DTS downloads, import them, start the service.
//!
//! Acer only hands out its downloads through its support site (time-limited links made by the page's scripts),
//! so the user downloads two files there; the wizard says which, opens the page, finds the files in Downloads
//! (or lets the user pick them) and does the rest.

use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Align, gio, glib};

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

/// The wizard window; `done` runs after a successful import.
pub fn window(app: &gtk::Application, done: impl Fn(&gtk::Application) + 'static) -> gtk::ApplicationWindow {
    let page = vbox(18);
    page.add_css_class("page");
    page.set_size_request(640, -1);

    let brand = label("", &["brand"]);
    brand.set_markup("Welcome to Acer<span foreground=\"#22d3ee\">UltraX</span>");
    page.append(&brand);

    // 1. The laptop.
    let codec = importer::laptop_codec();
    let model = product_name();
    let card = vbox(4);
    card.add_css_class("card");
    card.append(&label("1. Your laptop", &[]));
    let support = match &codec {
        Some(c) => match importer::support(&c.ssid) {
            importer::Support::Verified => {
                format!("{model} (sound chip {}): supported, verified against Windows.", c.ssid)
            }
            importer::Support::Experimental => format!(
                "{model} (sound chip {}): experimental. It should work like on Windows; please report how it goes.",
                c.ssid
            ),
        },
        None => "No Acer sound chip found. AcerUltraX needs an Acer laptop with DTS:X Ultra.".into(),
    };
    card.append(&label(&support, &["muted", "small"]));
    page.append(&card);

    // 2. The downloads.
    let card = vbox(8);
    card.add_css_class("card");
    card.append(&label("2. Get the DTS files from Acer", &[]));
    card.append(&label(
        &format!(
            "On Acer's support page, search for {}, then download from \"Driver\" → Audio: \"XPERI DTS\", and from \
             \"Application\": \"XPERI DTS Utility\" (both versions if there are two; they add headphone models). \
             Leave them zipped.",
            if model.is_empty() { "your model" } else { &model }
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
    page.append(&card);

    // 3. Import.
    let card = vbox(8);
    card.add_css_class("card");
    card.append(&label("3. Set up", &[]));
    let files_label = label("", &["muted", "small"]);
    card.append(&files_label);
    let row = hbox(8);
    let choose = gtk::Button::with_label("Choose files…");
    choose.add_css_class("outline");
    let rescan = gtk::Button::with_label("Look in Downloads again");
    rescan.add_css_class("outline");
    let go = gtk::Button::with_label("Set up AcerUltraX");
    go.add_css_class("outline");
    row.append(&choose);
    row.append(&rescan);
    row.append(&go);
    card.append(&row);
    let status = label("", &["small"]);
    card.append(&status);
    page.append(&card);

    // Wrap the explanations at a comfortable width instead of stretching the window.
    let mut w = page.first_child();
    while let Some(c) = w {
        let mut inner = c.first_child();
        while let Some(i) = inner {
            if let Some(l) = i.downcast_ref::<gtk::Label>() {
                l.set_max_width_chars(70);
            }
            inner = i.next_sibling();
        }
        w = c.next_sibling();
    }
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Set up AcerUltraX")
        .default_width(720)
        .child(&page)
        .build();
    window.add_css_class("aux");

    let files: Rc<std::cell::RefCell<Vec<PathBuf>>> = Rc::new(std::cell::RefCell::new(Vec::new()));
    let show = {
        let (files, files_label, go) = (files.clone(), files_label.clone(), go.clone());
        Rc::new(move || {
            let f = files.borrow();
            files_label.set_text(&if f.is_empty() {
                format!("No DTS downloads found in {} yet.", downloads_dir().display())
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
    let (f2, s2, w2) = (files.clone(), show.clone(), window.clone());
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

    let done = Rc::new(done);
    let (f2, w2, app2) = (files, window.clone(), app.clone());
    go.connect_clicked(move |b| {
        let Some(c) = importer::laptop_codec() else {
            status.set_text("No Acer sound chip found.");
            return;
        };
        b.set_sensitive(false);
        status.set_text("Setting up… (a few seconds)");
        let srcs = f2.borrow().clone();
        let (status, b, w3, app3, done) = (status.clone(), b.clone(), w2.clone(), app2.clone(), done.clone());
        glib::spawn_future_local(async move {
            let data = importer::data_dir();
            let result = gio::spawn_blocking(move || {
                std::fs::create_dir_all(&data).map_err(|e| e.to_string())?;
                importer::import(&srcs, &c.ssid, &data).map_err(|e| e.to_string())
            })
            .await
            .unwrap_or_else(|_| Err("the import stopped unexpectedly".into()));
            match result {
                Ok(m) => {
                    status.set_text(&format!("Done: DTS tuning for {} and {} headphone models.", m.ssid, m.headphones));
                    // Start the service now and at every login.
                    let cli = std::env::current_exe()
                        .map(|e| e.with_file_name("acerultrax-cli"))
                        .unwrap_or_else(|_| "acerultrax-cli".into());
                    let _ = std::process::Command::new(cli).args(["autostart", "on"]).status();
                    w3.close();
                    done(&app3);
                }
                Err(e) => {
                    status.set_text(&format!("Could not set up: {e}"));
                    b.set_sensitive(true);
                }
            }
        });
    });
    window
}

/// Whether the DTS files are set up.
pub fn needed() -> bool {
    importer::Manifest::load(&importer::data_dir()).is_err()
}
