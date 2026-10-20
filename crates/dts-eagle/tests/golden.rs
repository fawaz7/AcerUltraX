//! Bit-exact comparison with engine input/output recorded inside the Windows DTS APO.
//!
//! Needs the user's DTS files and the recordings, which are not part of this repository:
//! - `ACERULTRAX_VENDOR_DIR`: directory with `dtstech64.dll`, the OEM XML and the tuning files;
//! - `ACERULTRAX_GOLDEN_DIR`: directory with `index.json` and one folder per case (`sfx.in.f32`, `sfx.out.f32`,
//!   `mfx.in.f32`, `mfx.out.f32`, `blob.sfx.bin`, `blob.mfx.bin`; float32 little-endian, interleaved).
//!
//! Skipped (passes with a note) when either is unset.

use std::path::{Path, PathBuf};

use dts_eagle::{Blob, Library, Location, OemConfig, Output};

struct Case {
    dir: PathBuf,
    rate: u32,
    mode: dts_eagle::Mode,
    output: Output,
    channels: usize,
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_dir())
}

fn setup() -> Option<(&'static Library, OemConfig, Vec<Case>)> {
    let (Some(vendor), Some(golden)) = (env_dir("ACERULTRAX_VENDOR_DIR"), env_dir("ACERULTRAX_GOLDEN_DIR")) else {
        eprintln!("skipped: set ACERULTRAX_VENDOR_DIR and ACERULTRAX_GOLDEN_DIR");
        return None;
    };
    let _ = env_logger::builder().is_test(true).try_init();
    let lib = Library::load(&vendor.join("dtstech64.dll"), Some(&vendor)).expect("load engine");
    let xml = std::fs::read_dir(&vendor)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().unwrap().to_string_lossy().starts_with("dts_apo4_oem_config_"))
        .expect("OEM XML in vendor dir");
    let oem = OemConfig::parse(&std::fs::read_to_string(xml).unwrap()).unwrap();
    let cases = read_index(&golden);
    assert!(cases.len() >= 33, "expected at least 33 golden cases, found {}", cases.len());
    Some((lib, oem, cases))
}

/// Minimal reader for the flat `index.json` (an array of objects with scalar values).
fn read_index(golden: &Path) -> Vec<Case> {
    let text = std::fs::read_to_string(golden.join("index.json")).unwrap();
    let mut cases = Vec::new();
    for obj in text.split('{').skip(1) {
        let body = obj.split('}').next().unwrap();
        let field = |k: &str| -> Option<String> {
            let at = body.find(&format!("\"{k}\""))?;
            let rest = body[at + k.len() + 2..].trim_start().strip_prefix(':')?.trim_start();
            let v = rest.split([',', '\n']).next()?.trim();
            Some(v.trim_matches('"').to_string())
        };
        let output = match field("output").as_deref() {
            Some("spk") => Output::Speakers,
            Some("hp") => Output::Headphones,
            other => panic!("unknown output {other:?}"),
        };
        cases.push(Case {
            dir: golden.join(field("dir").unwrap()),
            rate: field("rate").unwrap().parse().unwrap(),
            mode: field("mode").unwrap().parse().unwrap(),
            output,
            channels: field("channels").unwrap().parse().unwrap(),
        });
    }
    cases
}

fn read_f32(p: &Path) -> Vec<f32> {
    std::fs::read(p).unwrap().chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect()
}

fn deinterleave(x: &[f32], ch: usize) -> Vec<Vec<f32>> {
    (0..ch).map(|c| x.iter().skip(c).step_by(ch).copied().collect()).collect()
}

/// Run one engine over a case's recorded input; returns the number of output samples that differ in any bit.
fn run(mut e: dts_eagle::Engine, input: &Path, expected: &Path, channels: usize) -> usize {
    let inp = deinterleave(&read_f32(input), channels);
    let frames = inp[0].len();
    let ins: Vec<&[f32]> = inp.iter().map(|v| v.as_slice()).collect();
    let mut out = vec![vec![0f32; frames]; 2];
    let mut outs: Vec<&mut [f32]> = out.iter_mut().map(|v| v.as_mut_slice()).collect();
    e.process(&ins, &mut outs).unwrap();
    let exp = deinterleave(&read_f32(expected), 2);
    assert_eq!(exp[0].len(), frames, "{}", expected.display());
    (0..2).map(|c| (0..frames).filter(|&i| out[c][i].to_bits() != exp[c][i].to_bits()).count()).sum()
}

fn stage_files(loc: Location) -> (&'static str, &'static str, &'static str) {
    match loc {
        Location::Sfx => ("sfx.in.f32", "sfx.out.f32", "blob.sfx.bin"),
        Location::Mfx => ("mfx.in.f32", "mfx.out.f32", "blob.mfx.bin"),
    }
}

/// Engines run on a thread with a large stack, as the DTS code needs.
fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(8 << 20).spawn(f).unwrap().join().unwrap()
}

#[test]
fn golden_from_blobs() {
    let Some((lib, _oem, cases)) = setup() else { return };
    let failures = on_big_stack(move || {
        let mut failures = Vec::new();
        let mut checked = 0;
        for c in &cases {
            for loc in [Location::Sfx, Location::Mfx] {
                let (inp, exp, blob) = stage_files(loc);
                let ch = if loc == Location::Sfx { c.channels } else { 2 };
                if loc == Location::Mfx && !c.dir.join(inp).exists() {
                    continue;
                }
                let blob = Blob::from_bytes(&std::fs::read(c.dir.join(blob)).unwrap()).unwrap();
                let e = dts_eagle::engine_from_blob(lib, &blob, loc, ch, c.rate).unwrap();
                let bad = run(e, &c.dir.join(inp), &c.dir.join(exp), ch);
                if bad != 0 {
                    failures.push(format!("{} {:?}: {bad} samples differ", c.dir.display(), loc));
                }
                checked += 1;
            }
        }
        eprintln!("{checked} engine runs compared");
        failures
    });
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Engines configured setting by setting from the OEM configuration, tuning files included, as Windows does.
#[test]
fn golden_from_controls() {
    let Some((lib, oem, cases)) = setup() else { return };
    let failures = on_big_stack(move || {
        let mut failures = Vec::new();
        let mut checked = 0;
        for c in &cases {
            let tuning = oem.tuning(c.mode, c.output).expect("mode in OEM config");
            for loc in [Location::Sfx, Location::Mfx] {
                let (inp, exp, _) = stage_files(loc);
                if !c.dir.join(inp).exists() {
                    continue;
                }
                let ch = if loc == Location::Sfx { c.channels } else { 2 };
                let (e, _skipped) = dts_eagle::configured_engine(lib, tuning, loc, ch, c.rate).unwrap();
                let bad = run(e, &c.dir.join(inp), &c.dir.join(exp), ch);
                if bad != 0 {
                    failures.push(format!("{} {:?}: {bad} samples differ", c.dir.display(), loc));
                }
                checked += 1;
            }
        }
        eprintln!("{checked} engine runs compared");
        failures
    });
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Several engines processing at the same time on different threads (one per app stream, as the audio graph
/// will run them) stay bit-exact.
#[test]
fn golden_concurrent_threads() {
    let Some((lib, _oem, cases)) = setup() else { return };
    let cases: Vec<Case> = cases.into_iter().filter(|c| c.rate == 48000).collect();
    let cases = std::sync::Arc::new(cases);
    let workers: Vec<_> = (0..4)
        .map(|w| {
            let cases = cases.clone();
            std::thread::Builder::new()
                .stack_size(8 << 20)
                .spawn(move || {
                    let mut bad = Vec::new();
                    for c in cases.iter().skip(w).step_by(4) {
                        for loc in [Location::Sfx, Location::Mfx] {
                            let (inp, exp, blob) = stage_files(loc);
                            if !c.dir.join(inp).exists() {
                                continue;
                            }
                            let ch = if loc == Location::Sfx { c.channels } else { 2 };
                            let blob = Blob::from_bytes(&std::fs::read(c.dir.join(blob)).unwrap()).unwrap();
                            let e = dts_eagle::engine_from_blob(lib, &blob, loc, ch, c.rate).unwrap();
                            if run(e, &c.dir.join(inp), &c.dir.join(exp), ch) != 0 {
                                bad.push(format!("{} {loc:?}", c.dir.display()));
                            }
                        }
                    }
                    bad
                })
                .unwrap()
        })
        .collect();
    let failures: Vec<String> = workers.into_iter().flat_map(|w| w.join().unwrap()).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
