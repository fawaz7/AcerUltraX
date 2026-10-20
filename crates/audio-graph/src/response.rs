//! The frequency response of a mode as configured: noise through its stream and mode engines, offline, compared
//! with what went in. This shows what DTS really does (tuning files, headphone EQ and adjustments included),
//! not a drawing of nominal filter shapes.

use dts_eagle::{Library, Location, ModeTuning};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

/// (frequency in Hz, gain in dB) at 1/6-octave points from 20 Hz to 20 kHz, for sound in the centre of the
/// stereo image (the same signal on both channels).
/// `eq`: the user's equaliser after DTS (its exact response is added).
pub fn response(
    lib: &'static Library,
    tuning: &ModeTuning,
    rate: u32,
    eq: Option<[i32; 10]>,
) -> Result<Vec<(f32, f32)>, dts_eagle::Error> {
    let eq = eq.map(|g| crate::Eq::new(g, rate));
    let n = rate as usize * 4;
    // xorshift64* white noise (a plain LCG's patterns throw off the delay search).
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    let noise: Vec<f32> = (0..n)
        .map(|_| {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.25
        })
        .collect();
    let (mut sfx, _) = dts_eagle::configured_engine(lib, tuning, Location::Sfx, 2, rate)?;
    let (mut mfx, _) = dts_eagle::configured_engine(lib, tuning, Location::Mfx, 2, rate)?;
    let (mut a, mut b) = (vec![0f32; n], vec![0f32; n]);
    sfx.process(&[&noise, &noise], &mut [&mut a, &mut b])?;
    let (mut l, mut r) = (vec![0f32; n], vec![0f32; n]);
    mfx.process(&[&a, &b], &mut [&mut l, &mut r])?;
    let out: Vec<f32> = l.iter().zip(&r).map(|(l, r)| (l + r) / 2.0).collect();
    // The engines delay the sound; line output up with input before comparing.
    let delay = delay(&noise, &out, rate as usize);
    let out = &out[delay..];
    let noise = &noise[..n - delay];
    let n = n - delay;

    // One long FFT of the aligned signals after the engines have settled; per band, the median of |Y/X|.
    let size = 1 << 17;
    let at = n - size;
    let fft = FftPlanner::new().plan_fft_forward(size);
    let spec = |s: &[f32]| {
        let mut v: Vec<Complex<f32>> = s[at..at + size].iter().map(|&a| Complex::new(a, 0.0)).collect();
        fft.process(&mut v);
        v
    };
    let (xs, ys) = (spec(noise), spec(out));
    let h: Vec<f32> = (0..size / 2).map(|k| ys[k].norm() / xs[k].norm().max(1e-12)).collect();
    let bin = rate as f32 / size as f32;
    let mut points = Vec::new();
    let mut f = 20f32;
    while f <= 20000.0 && f < rate as f32 / 2.0 {
        let (lo, hi) =
            ((f / 2f32.powf(1.0 / 12.0) / bin) as usize, ((f * 2f32.powf(1.0 / 12.0) / bin) as usize).max(1));
        let mut band: Vec<f32> = h[lo.max(1)..=hi.min(size / 2 - 1)].to_vec();
        if !band.is_empty() {
            band.sort_by(f32::total_cmp);
            let user = eq.as_ref().map_or(0.0, |e| e.response_db(f));
            points.push((f, 20.0 * band[band.len() / 2].max(1e-9).log10() + user));
        }
        f *= 2f32.powf(1.0 / 6.0);
    }
    Ok(points)
}

/// Delay (in samples, up to `max`) at which `y` correlates best with `x`, from a 1-second stretch.
fn delay(x: &[f32], y: &[f32], max: usize) -> usize {
    let len = (1 << 16).min(x.len() / 2);
    let size = (len + max).next_power_of_two() * 2;
    let mut planner = FftPlanner::new();
    let (fwd, inv) = (planner.plan_fft_forward(size), planner.plan_fft_inverse(size));
    let at = x.len() / 3;
    let pad = |s: &[f32], n: usize| {
        let mut v: Vec<Complex<f32>> = s[..n].iter().map(|&a| Complex::new(a, 0.0)).collect();
        v.resize(size, Complex::new(0.0, 0.0));
        v
    };
    let (mut a, mut b) = (pad(&x[at..], len), pad(&y[at..], (len + max).min(y.len() - at)));
    fwd.process(&mut a);
    fwd.process(&mut b);
    let mut c: Vec<Complex<f32>> = b.iter().zip(&a).map(|(b, a)| b * a.conj()).collect();
    inv.process(&mut c);
    (0..max).max_by(|&i, &j| c[i].re.total_cmp(&c[j].re)).unwrap_or(0)
}
