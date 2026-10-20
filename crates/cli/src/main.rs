//! `acerultrax-cli`: set up the DTS files, check the laptop, and render audio files through the DTS chain.

mod autostart;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use dts_eagle::{Library, Location, Mode, OemConfig, Output};
use importer::{Manifest, Support};
use serde_json::json;

#[derive(Parser)]
#[command(name = "acerultrax-cli", version, about = "AcerUltraX command-line tool")]
struct Cli {
    /// Data directory (default: ~/.local/share/acerultrax).
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the laptop's sound chip and whether AcerUltraX supports it.
    Detect,
    /// Set up everything in one go: download Acer's DTS files for this laptop from Acer and import them.
    Setup {
        /// Skip the DTS app download (385 MB; adds headphone models and spatial sound).
        #[arg(long)]
        no_extras: bool,
    },
    /// Set up the DTS files from Acer's downloads for your laptop: the DTS driver ("XPERI DTS") and, for
    /// headphone models, the DTS app ("XPERI DTS Utility"). Zip files or extracted folders.
    Import {
        /// Zip files or folders (searched recursively).
        #[arg(required = true)]
        sources: Vec<PathBuf>,
        /// Sound chip subsystem ID to import the tuning for (default: this laptop's).
        #[arg(long)]
        ssid: Option<String>,
    },
    /// Show what is set up and what the background service is doing.
    Status,
    /// Use a profile (a DTS mode such as music, movie, game1, or off) on the current output device.
    Mode {
        name: String,
        /// Another device instead (its name from `devices`).
        #[arg(long)]
        device: Option<String>,
    },
    /// List output devices with the profile each uses.
    Devices,
    /// List headphone models (optionally only those matching a search).
    Headphones { search: Option<String> },
    /// Use a headphone model's EQ, as picked in Windows' DTS app: a name (e.g. "hyperx cloud core"),
    /// `default` for Acer's headphone EQ, or `auto` to recognise Bluetooth/USB headphones by name.
    Headphone {
        model: String,
        /// Output to set it for (from `devices`; default: the current one; `@headphones` = 3.5 mm jack).
        #[arg(long)]
        device: Option<String>,
    },
    /// List the available profiles.
    Profiles,
    /// Windows' spatial sound (DTS:X Ultra / Headphone:X) for 5.1 and 7.1 sound, on the current output.
    Spatial {
        state: OnOff,
        /// Another output (from `devices`).
        #[arg(long)]
        device: Option<String>,
    },
    /// Turn DTS on or off everywhere.
    Power { state: OnOff },
    /// Loudness-matched comparison: turn DTS down to the loudness of the plain sound (on/off).
    Compare { state: OnOff },
    /// Start AcerUltraX with your session (on), or stop doing so (off).
    Autostart { state: OnOff },
    /// Restart the background service (it then loads newly set-up files).
    Restart,
    /// Process a WAV file through the DTS chain, as Windows does for one app (1, 2, 6 or 8 channels in, stereo out).
    Render {
        input: PathBuf,
        output: PathBuf,
        #[arg(long, default_value = "music")]
        mode: String,
        #[arg(long, value_enum, default_value_t = OutputArg::Speakers)]
        output_device: OutputArg,
    },
    /// List the DTS engine's controls.
    Controls,
}

#[derive(Clone, Copy, ValueEnum)]
enum OnOff {
    On,
    Off,
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputArg {
    Speakers,
    Headphones,
}

impl From<OutputArg> for Output {
    fn from(o: OutputArg) -> Output {
        match o {
            OutputArg::Speakers => Output::Speakers,
            OutputArg::Headphones => Output::Headphones,
        }
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let cli = Cli::parse();
    let data = cli.data_dir.unwrap_or_else(importer::data_dir);
    match cli.command {
        Command::Detect => detect(),
        Command::Setup { no_extras } => setup(!no_extras, &data),
        Command::Import { sources, ssid } => import(&sources, ssid, &data),
        Command::Status => status(&data),
        Command::Mode { name, device } => {
            daemon(json!({"cmd": "set-profile", "name": name, "device": device})).map(|_| ())
        }
        Command::Headphones { search } => {
            for h in find_headphones(search.as_deref().unwrap_or(""))? {
                println!(
                    "{:40} {:20} {}",
                    format!("{} {}", h["brand"].as_str().unwrap_or(""), h["name"].as_str().unwrap_or("")),
                    h["style"].as_str().unwrap_or(""),
                    routes_text(&h)
                );
            }
            Ok(())
        }
        Command::Headphone { model, device } => {
            let id = match model.to_lowercase().as_str() {
                "auto" => None,
                "default" => Some("default".to_string()),
                q => {
                    let found = find_headphones(q)?;
                    let full = |h: &serde_json::Value| {
                        format!("{} {}", h["brand"].as_str().unwrap_or(""), h["name"].as_str().unwrap_or(""))
                            .to_lowercase()
                    };
                    let pick =
                        found.iter().find(|h| full(h) == q).or(if found.len() == 1 { found.first() } else { None });
                    match pick {
                        Some(h) => Some(h["id"].as_str().unwrap_or("").to_string()),
                        None if found.is_empty() => bail!("no headphone model matches {model:?}"),
                        None => bail!(
                            "{} models match {model:?}; be more specific (see `acerultrax-cli headphones {model}`)",
                            found.len()
                        ),
                    }
                }
            };
            daemon(json!({"cmd": "set-headphone", "id": id, "device": device})).map(|_| ())
        }
        Command::Devices => {
            for d in daemon(json!({"cmd": "devices"}))?.as_array().into_iter().flatten() {
                let mark = if d["active"].as_bool() == Some(true) { "*" } else { " " };
                println!(
                    "{mark} {:50} {:18} {}",
                    d["device_name"].as_str().unwrap_or(""),
                    d["kind"].as_str().unwrap_or(""),
                    d["profile"].as_str().unwrap_or("")
                );
                println!("    {}", d["device"].as_str().unwrap_or(""));
            }
            Ok(())
        }
        Command::Profiles => {
            for p in daemon(json!({"cmd": "profiles"}))?.as_array().into_iter().flatten() {
                println!("{}", p["name"].as_str().unwrap_or(""));
            }
            Ok(())
        }
        Command::Autostart { state: OnOff::On } => autostart::enable(),
        Command::Autostart { state: OnOff::Off } => autostart::disable(),
        Command::Restart => {
            if !autostart::restart()? {
                println!("The service is not running as a systemd user service; log out and back in to restart it.");
            }
            Ok(())
        }
        Command::Spatial { state, device } => {
            daemon(json!({"cmd": "spatial", "on": matches!(state, OnOff::On), "device": device})).map(|_| ())
        }
        Command::Power { state } => daemon(json!({"cmd": "enable", "on": matches!(state, OnOff::On)})).map(|_| ()),
        Command::Compare { state } => daemon(json!({"cmd": "compare", "on": matches!(state, OnOff::On)})).map(|_| ()),
        Command::Render { input, output, mode, output_device } => {
            let mode: Mode = mode.parse()?;
            on_engine_thread(move || render(&data, &input, &output, mode, output_device.into()))
        }
        Command::Controls => on_engine_thread(move || controls(&data)),
    }
}

/// DTS code needs a large stack; run engine work on a thread that has one.
fn on_engine_thread(f: impl FnOnce() -> Result<()> + Send + 'static) -> Result<()> {
    std::thread::Builder::new().name("dts".into()).stack_size(8 << 20).spawn(f)?.join().expect("engine thread panicked")
}

fn support_text(ssid: &str) -> String {
    match importer::support(ssid) {
        Support::Verified => {
            let model =
                importer::VERIFIED_MODELS.iter().find(|(s, _)| s.eq_ignore_ascii_case(ssid)).map_or("", |m| m.1);
            format!("supported ({model})")
        }
        Support::Experimental => "experimental: not tested yet, please report how it works".into(),
    }
}

fn detect() -> Result<()> {
    let codecs = importer::detect_codecs();
    if codecs.is_empty() {
        bail!("no sound chips found in /proc/asound");
    }
    for c in &codecs {
        println!("card {}: {} (vendor {}, subsystem {})", c.card, c.name, c.vendor_id, c.ssid);
    }
    match importer::laptop_codec() {
        Some(c) => println!("\nAcer laptop sound chip {}: {}", c.ssid, support_text(&c.ssid)),
        None => println!("\nNo Acer laptop sound chip found; AcerUltraX needs an Acer laptop with DTS."),
    }
    Ok(())
}

fn setup(extras: bool, data: &Path) -> Result<()> {
    use std::io::Write;
    let c = importer::laptop_codec().context("no Acer sound chip found; AcerUltraX needs an Acer laptop with DTS")?;
    if !importer::download::covers(&c.ssid) {
        bail!(
            "Acer's DTS package has no tuning for sound chip {} yet. If Acer's support page lists \"XPERI DTS\" \
             for your laptop, download it and run `acerultrax-cli import <file>`.",
            c.ssid
        );
    }
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut last = u64::MAX;
    let m = importer::download::setup(&c.ssid, data, extras, &cancel, &mut |s| match s {
        importer::download::Stage::Downloading { done, total } => {
            let mb = done >> 20;
            if mb != last {
                last = mb;
                print!("\rDownloading from Acer: {mb} of {} MB", total >> 20);
                let _ = std::io::stdout().flush();
            }
        }
        importer::download::Stage::Importing => println!("\nSetting up…"),
    })?;
    report(&m, data);
    // A running service loads the new files.
    if autostart::restart()? {
        println!("Restarted the AcerUltraX service.");
    }
    Ok(())
}

fn import(sources: &[PathBuf], ssid: Option<String>, data: &Path) -> Result<()> {
    let ssid = match ssid {
        Some(s) => s,
        None => importer::laptop_codec().context("no Acer sound chip found; pass --ssid")?.ssid,
    };
    std::fs::create_dir_all(data).with_context(|| format!("creating {}", data.display()))?;
    let m = importer::import(sources, &ssid, data)?;
    report(&m, data);
    Ok(())
}

fn report(m: &Manifest, data: &Path) {
    println!("Imported DTS tuning for {} ({}) into {}", m.ssid, support_text(&m.ssid), data.display());
    println!("{} tuning files", m.files.len());
    match m.headphones {
        0 => println!("No headphone models (add Acer's \"XPERI DTS Utility\" download to get them)"),
        n => println!("{n} headphone models"),
    }
    if m.spatial_presets > 0 {
        println!("Spatial sound: encoder and {} presets", m.spatial_presets);
    }
    if !m.engine_verified {
        println!("Note: this dtstech64.dll build has not been verified with AcerUltraX yet.");
    }
}

fn status(data: &Path) -> Result<()> {
    let m = Manifest::load(data).with_context(|| format!("nothing set up in {} (run import first)", data.display()))?;
    println!("Data directory: {}", data.display());
    println!("Laptop tuning:  {} ({})", m.ssid, support_text(&m.ssid));
    println!(
        "DTS engine:     {}{}",
        m.engine_sha256,
        if m.engine_verified { " (verified build)" } else { " (unverified build)" }
    );
    println!("Imported from:  {}", m.source.display());
    println!("Tuning files:   {}", m.files.len());
    match daemon(json!({"cmd": "status"})) {
        Ok(s) => {
            let txt = |k: &str| s[k].as_str().unwrap_or("-").to_string();
            println!("Service:        running");
            println!("DTS:            {}", if s["enabled"].as_bool() == Some(false) { "off" } else { "on" });
            println!("Output:         {} ({})", txt("device_name"), txt("kind"));
            println!("Profile:        {}", txt("profile"));
            if let Some(h) = s["headphone"].as_str() {
                println!("Headphones:     {h}");
            }
            println!("Comparison:     {}", if s["compare"].as_bool() == Some(true) { "on" } else { "off" });
            if s["spatial_available"].as_bool() == Some(true) {
                println!("Spatial sound:  {}", if s["spatial"].as_bool() == Some(true) { "on" } else { "off" });
            }
            println!("Apps:           {}", s["streams"]);
        }
        Err(_) => println!("Service:        not running"),
    }
    println!("Autostart:      {}", if autostart::enabled() { "on" } else { "off" });
    Ok(())
}

fn find_headphones(q: &str) -> Result<Vec<serde_json::Value>> {
    let q = q.to_lowercase();
    let all = daemon(json!({"cmd": "headphones"}))?;
    Ok(all
        .as_array()
        .into_iter()
        .flatten()
        .filter(|h| {
            format!("{} {}", h["brand"].as_str().unwrap_or(""), h["name"].as_str().unwrap_or(""))
                .to_lowercase()
                .contains(&q)
        })
        .cloned()
        .collect())
}

fn routes_text(h: &serde_json::Value) -> String {
    let names: Vec<&str> = h["routes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| match r.as_str()? {
            "line-out" => Some("3.5 mm"),
            "bluetooth" => Some("Bluetooth"),
            "usb" => Some("USB"),
            _ => None,
        })
        .collect();
    names.join(", ")
}

/// Send one request to the background service and return its result.
fn daemon(req: serde_json::Value) -> Result<serde_json::Value> {
    use std::io::{BufRead, BufReader, Write};
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let mut s = std::os::unix::net::UnixStream::connect(dir.join("acerultrax.sock"))
        .context("the AcerUltraX service is not running")?;
    writeln!(s, "{req}")?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line)?;
    let reply: serde_json::Value = serde_json::from_str(&line)?;
    if reply["ok"].as_bool() != Some(true) {
        bail!("{}", reply["error"].as_str().unwrap_or("request failed"));
    }
    Ok(reply["result"].clone())
}

fn load(data: &Path) -> Result<(&'static Library, OemConfig)> {
    let m = Manifest::load(data).with_context(|| format!("nothing set up in {} (run import first)", data.display()))?;
    let lib = Library::load(&Manifest::engine_path(data), Some(&Manifest::dts_dir(data)))?;
    let xml = std::fs::read_to_string(m.oem_config_path(data))?;
    Ok((lib, OemConfig::parse(&xml)?))
}

fn controls(data: &Path) -> Result<()> {
    let (lib, _) = load(data)?;
    let e = dts_eagle::Engine::new(lib, Location::Sfx)?;
    for c in e.controls() {
        let range = if c.choices.is_empty() {
            format!("{}..{} (default {})", c.min, c.max, c.default)
        } else {
            c.choices.join(" | ")
        };
        println!("{:42} {:?} {range}", c.name, c.kind);
    }
    Ok(())
}

fn render(data: &Path, input: &Path, output: &Path, mode: Mode, out: Output) -> Result<()> {
    let (lib, oem) = load(data)?;
    let tuning = oem.tuning(mode, out).with_context(|| format!("no {mode} tuning for {out:?}"))?;
    let mut reader = hound::WavReader::open(input).with_context(|| format!("opening {}", input.display()))?;
    let spec = reader.spec();
    let ch = spec.channels as usize;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader.samples::<i32>().map(|s| s.map(|v| v as f32 * scale)).collect::<Result<_, _>>()?
        }
    };
    let planar: Vec<Vec<f32>> = (0..ch).map(|c| samples.iter().skip(c).step_by(ch).copied().collect()).collect();
    // Windows: mono is duplicated to stereo; 2, 6 and 8 channels get an SFX engine of that width.
    let sfx_in: Vec<&[f32]> = match ch {
        1 => vec![&planar[0], &planar[0]],
        2 | 6 | 8 => planar.iter().map(|v| v.as_slice()).collect(),
        n => bail!("{n}-channel input is not supported (use 1, 2, 6 or 8 channels)"),
    };
    let frames = planar[0].len();
    let (mut sfx, _) = dts_eagle::configured_engine(lib, tuning, Location::Sfx, sfx_in.len(), spec.sample_rate)?;
    let (mut mfx, _) = dts_eagle::configured_engine(lib, tuning, Location::Mfx, 2, spec.sample_rate)?;
    let mut mid = vec![vec![0f32; frames]; 2];
    {
        let mut m: Vec<&mut [f32]> = mid.iter_mut().map(|v| v.as_mut_slice()).collect();
        sfx.process(&sfx_in, &mut m)?;
    }
    let mut res = vec![vec![0f32; frames]; 2];
    {
        let mut r: Vec<&mut [f32]> = res.iter_mut().map(|v| v.as_mut_slice()).collect();
        mfx.process(&[&mid[0], &mid[1]], &mut r)?;
    }
    let ospec = hound::WavSpec {
        channels: 2,
        sample_rate: spec.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(output, ospec)?;
    for (l, r) in res[0].iter().zip(&res[1]) {
        w.write_sample(*l)?;
        w.write_sample(*r)?;
    }
    w.finalize()?;
    println!("Rendered {} frames in {mode} mode for {out:?} to {}", frames, output.display());
    Ok(())
}
