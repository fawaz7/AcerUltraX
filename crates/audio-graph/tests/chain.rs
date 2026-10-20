//! The chain, fed block by block like PipeWire, against engines run independently on whole signals.
//! Needs `ACERULTRAX_VENDOR_DIR` (see docs/TESTING.md); skipped without it.

use std::path::PathBuf;

use audio_graph::{Chain, Prepared, StreamInput};
use dts_eagle::{Library, Location, Mode, OemConfig, Output};

fn signal(n: usize, seed: u32) -> Vec<f32> {
    // Deterministic noise plus a tone, loud enough to exercise the engines.
    let mut x = seed.wrapping_mul(2654435761) | 1;
    (0..n)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            0.2 * (i as f32 * 0.01 * (seed + 1) as f32).sin() + 0.05 * (x as f32 / u32::MAX as f32 - 0.5)
        })
        .collect()
}

#[test]
fn per_stream_engines_mix_then_mfx() {
    let Some(vendor) = std::env::var_os("ACERULTRAX_VENDOR_DIR").map(PathBuf::from) else {
        eprintln!("skipped: set ACERULTRAX_VENDOR_DIR");
        return;
    };
    let lib = Library::load(&vendor.join("dtstech64.dll"), Some(&vendor)).unwrap();
    let xml = std::fs::read_dir(&vendor)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().unwrap().to_string_lossy().starts_with("dts_apo4_oem_config_"))
        .unwrap();
    let oem = OemConfig::parse(&std::fs::read_to_string(xml).unwrap()).unwrap();
    let tuning = oem.tuning(Mode::Movie, Output::Speakers).unwrap().clone();
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let (rate, n) = (48000, 48000);
            // Streams: stereo (id 7), 7.1 (id 9), mono (id 11).
            let layouts = [(7u32, 2usize), (9, 8), (11, 1)];
            let sigs: Vec<Vec<Vec<f32>>> =
                layouts.iter().map(|&(id, ch)| (0..ch).map(|c| signal(n, id * 10 + c as u32)).collect()).collect();

            // Reference: each stream through its own SFX engine, summed, then MFX.
            let prepared = Prepared::build(lib, &tuning, rate).unwrap();
            let (mut sfx2, _) = dts_eagle::configured_engine(lib, &tuning, Location::Sfx, 2, rate).unwrap();
            let blob = sfx2.tuning_blob();
            let mut mix = vec![vec![0f32; n]; 2];
            for (s, &(_, ch)) in sigs.iter().zip(&layouts) {
                let width = if ch == 1 { 2 } else { ch };
                let mut e = dts_eagle::engine_from_blob(lib, &blob, Location::Sfx, width, rate).unwrap();
                let ins: Vec<&[f32]> = (0..width).map(|c| s[if ch == 1 { 0 } else { c }].as_slice()).collect();
                let mut o = vec![vec![0f32; n]; 2];
                {
                    let mut os: Vec<&mut [f32]> = o.iter_mut().map(|v| v.as_mut_slice()).collect();
                    e.process(&ins, &mut os).unwrap();
                }
                for c in 0..2 {
                    for i in 0..n {
                        mix[c][i] += o[c][i];
                    }
                }
            }
            let (mut mfx, _) = dts_eagle::configured_engine(lib, &tuning, Location::Mfx, 2, rate).unwrap();
            let mut want = vec![vec![0f32; n]; 2];
            {
                let mut os: Vec<&mut [f32]> = want.iter_mut().map(|v| v.as_mut_slice()).collect();
                mfx.process(&[&mix[0], &mix[1]], &mut os).unwrap();
            }

            // The chain, in 480-frame blocks.
            let mut chain = Chain::new(lib);
            chain.set_mode(prepared);
            let mut got = vec![vec![0f32; n]; 2];
            for off in (0..n).step_by(480) {
                let k = 480.min(n - off);
                let views: Vec<Vec<&[f32]>> =
                    sigs.iter().map(|s| s.iter().map(|c| &c[off..off + k]).collect()).collect();
                let inputs: Vec<StreamInput> =
                    layouts.iter().zip(&views).map(|(&(id, _), v)| StreamInput { id, channels: v }).collect();
                let [gl, gr] = &mut got[..] else { unreachable!() };
                chain.process(rate, &inputs, [&mut gl[off..off + k], &mut gr[off..off + k]]);
            }
            let bad =
                (0..2).map(|c| (0..n).filter(|&i| got[c][i].to_bits() != want[c][i].to_bits()).count()).sum::<usize>();
            assert_eq!(bad, 0, "chain output differs from independent engines");

            // Bypass mixes the front pairs unprocessed, with the volume applied.
            chain.bypass = true;
            chain.gain = 0.5;
            let views: Vec<Vec<&[f32]>> = sigs.iter().map(|s| s.iter().map(|c| &c[..64]).collect()).collect();
            let inputs: Vec<StreamInput> =
                layouts.iter().zip(&views).map(|(&(id, _), v)| StreamInput { id, channels: v }).collect();
            let (mut l, mut r) = (vec![0f32; 64], vec![0f32; 64]);
            chain.process(rate, &inputs, [&mut l, &mut r]);
            let expect_l = 0.5 * (sigs[0][0][3] + sigs[1][0][3] + sigs[2][0][3]);
            assert!((l[3] - expect_l).abs() < 1e-6);
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Silent streams rest and output exact silence; sound afterwards is processed again.
#[test]
fn idle_streams_rest_and_resume() {
    let Some(vendor) = std::env::var_os("ACERULTRAX_VENDOR_DIR").map(PathBuf::from) else {
        eprintln!("skipped: set ACERULTRAX_VENDOR_DIR");
        return;
    };
    let lib = Library::load(&vendor.join("dtstech64.dll"), Some(&vendor)).unwrap();
    let xml = std::fs::read_dir(&vendor)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().unwrap().to_string_lossy().starts_with("dts_apo4_oem_config_"))
        .unwrap();
    let oem = OemConfig::parse(&std::fs::read_to_string(xml).unwrap()).unwrap();
    let tuning = oem.tuning(Mode::Music, Output::Speakers).unwrap().clone();
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let rate = 48000;
            let mut chain = Chain::new(lib);
            chain.set_mode(Prepared::build(lib, &tuning, rate).unwrap());
            let zeros = vec![0f32; 1024];
            let tone = signal(1024, 3);
            let (mut l, mut r) = (vec![0f32; 1024], vec![0f32; 1024]);
            let mut run = |x: &[f32], l: &mut [f32], r: &mut [f32]| {
                let ch = [x, x];
                chain.process(rate, &[StreamInput { id: 5, channels: &ch }], [l, r]);
            };
            run(&tone, &mut l, &mut r);
            // 4 s of silence: everything comes to rest with exact zeros.
            for _ in 0..(4 * rate as usize / 1024) {
                run(&zeros, &mut l, &mut r);
            }
            assert!(l.iter().chain(&r).all(|&x| x == 0.0), "resting chain must output exact silence");
            // The engines have a short processing delay: give them a few blocks.
            for _ in 0..5 {
                run(&tone, &mut l, &mut r);
            }
            assert!(l.iter().any(|&x| x.abs() > 1e-3), "sound after a pause is processed again");
        })
        .unwrap()
        .join()
        .unwrap();
}
