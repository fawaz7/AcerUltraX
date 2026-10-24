//! Loudness matching for fair comparisons: DTS is several dB louder than the unprocessed audio, and louder
//! always sounds "better". Both signals are K-weighted (ITU-R BS.1770), the input delayed by about DTS's own
//! delay, and their energies smoothed over 1 s; the output is scaled to the input's loudness, only ever down, by
//! at most 30 dB, the gain smoothed over 0.3 s. Measuring runs all the time, so switching it on is right at once.

/// About how late DTS's output is (42-60 ms depending on the mode, 25-32 ms through spatial sound): without it,
/// a short sound is measured in the input while DTS has not played it yet, then played unmatched.
const DELAY: usize = 2560;
/// Energy smoothing: DTS's loudness control lifts quiet passages much more than loud ones, so this has to follow
/// it within about a second (3 s lagged by up to 10 dB after a change).
const WINDOW_SECS: f64 = 1.0;

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// The two BS.1770 K-weighting stages (high shelf, then high-pass) for any sample rate.
#[allow(clippy::excessive_precision)] // the published filter constants
fn k_weighting(fs: f64) -> [Biquad; 2] {
    use std::f64::consts::PI;
    let (g, q, fc) = (3.99984385397, 0.7071752369554193, 1681.9744509555319);
    let a = 10f64.powf(g / 40.0);
    let (w, sa) = (2.0 * PI * fc / fs, a.sqrt());
    let (c, al) = (w.cos(), w.sin() / (2.0 * q));
    let a0 = (a + 1.0) - (a - 1.0) * c + 2.0 * sa * al;
    let shelf = Biquad {
        b0: a * ((a + 1.0) + (a - 1.0) * c + 2.0 * sa * al) / a0,
        b1: -2.0 * a * ((a - 1.0) + (a + 1.0) * c) / a0,
        b2: a * ((a + 1.0) + (a - 1.0) * c - 2.0 * sa * al) / a0,
        a1: 2.0 * ((a - 1.0) - (a + 1.0) * c) / a0,
        a2: ((a + 1.0) - (a - 1.0) * c - 2.0 * sa * al) / a0,
        ..Default::default()
    };
    let (q, fc) = (0.5003270373253953, 38.13547087613982);
    let w = 2.0 * PI * fc / fs;
    let (c, al) = (w.cos(), w.sin() / (2.0 * q));
    let a0 = 1.0 + al;
    let hp = Biquad {
        b0: (1.0 + c) / 2.0 / a0,
        b1: -(1.0 + c) / a0,
        b2: (1.0 + c) / 2.0 / a0,
        a1: -2.0 * c / a0,
        a2: (1.0 - al) / a0,
        ..Default::default()
    };
    [shelf, hp]
}

pub struct LoudnessMatch {
    rate: u32,
    k_in: [[Biquad; 2]; 2],
    k_out: [[Biquad; 2]; 2],
    /// The last [`DELAY`] input frames, oldest at `pos`.
    delay: Box<[[f32; 2]; DELAY]>,
    pos: usize,
    e_in: f64,
    e_out: f64,
    /// Gain matching the loudness, followed all the time.
    gain: f64,
    /// Gain applied: follows `gain` while matching is on and returns to 1 while it is off.
    applied: f64,
}

impl Default for LoudnessMatch {
    fn default() -> Self {
        LoudnessMatch {
            rate: 0,
            k_in: Default::default(),
            k_out: Default::default(),
            delay: Box::new([[0.0; 2]; DELAY]),
            pos: 0,
            e_in: 0.0,
            e_out: 0.0,
            gain: 1.0,
            applied: 1.0,
        }
    }
}

fn energy(k: &mut [[Biquad; 2]; 2], l: &[f32], r: &[f32]) -> f64 {
    let mut e = 0.0;
    for (&a, &b) in l.iter().zip(r) {
        let x = k[0][0].run(a as f64);
        let x = k[0][1].run(x);
        let y = k[1][0].run(b as f64);
        let y = k[1][1].run(y);
        e += x * x + y * y;
    }
    if l.is_empty() { 0.0 } else { e / l.len() as f64 }
}

impl LoudnessMatch {
    /// Measure `out` (processed) against `inp` (unprocessed), and when `on`, scale `out` towards the input's
    /// loudness, in place. Call it for every block, on or off. Allocates nothing.
    pub fn process(&mut self, rate: u32, on: bool, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        if self.rate != rate {
            let k = k_weighting(rate as f64);
            (self.rate, self.k_in, self.k_out) = (rate, [k, k], [k, k]);
            self.delay.fill([0.0; 2]);
            (self.e_in, self.e_out, self.gain) = (0.0, 0.0, 1.0);
        }
        let n = out_l.len();
        let mut ei = 0.0;
        for (&a, &b) in in_l.iter().zip(in_r) {
            let [dl, dr] = std::mem::replace(&mut self.delay[self.pos], [a, b]);
            self.pos = (self.pos + 1) % DELAY;
            let x = self.k_in[0][0].run(dl as f64);
            let x = self.k_in[0][1].run(x);
            let y = self.k_in[1][0].run(dr as f64);
            let y = self.k_in[1][1].run(y);
            ei += x * x + y * y;
        }
        let ei = if n == 0 { 0.0 } else { ei / n as f64 };
        let eo = energy(&mut self.k_out, out_l, out_r);
        // Near-silence (below about -70 dB) does not move the estimate.
        if ei > 1e-7 {
            let a = 1.0 - (-(n as f64) / (rate as f64 * WINDOW_SECS)).exp();
            if self.e_in <= 0.0 {
                (self.e_in, self.e_out) = (ei, eo);
            }
            self.e_in += a * (ei - self.e_in);
            self.e_out += a * (eo - self.e_out);
        }
        let target = if self.e_out > 0.0 { (self.e_in / self.e_out).sqrt() } else { 1.0 }.clamp(0.0316, 1.0);
        self.gain += (target - self.gain) * (1.0 - (-(n as f64) / (rate as f64 * 0.3)).exp());
        // Switching on or off fades in 50 ms instead of jumping.
        let want = if on { self.gain } else { 1.0 };
        let next = self.applied + (want - self.applied) * (1.0 - (-(n as f64) / (rate as f64 * 0.05)).exp());
        if self.applied != 1.0 || next != 1.0 {
            for i in 0..n {
                let g = (self.applied + (next - self.applied) * (i + 1) as f64 / n as f64) as f32;
                out_l[i] *= g;
                out_r[i] *= g;
            }
        }
        self.applied = if (next - 1.0).abs() < 1e-6 && !on { 1.0 } else { next };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_louder_output_down_to_input_level() {
        let rate = 48000;
        let tone: Vec<f32> = (0..rate as usize).map(|i| (i as f32 * 0.05).sin() * 0.1).collect();
        let mut m = LoudnessMatch::default();
        let mut last = 0.0;
        for _ in 0..20 {
            for chunk in tone.chunks(1024) {
                let (mut l, mut r): (Vec<f32>, Vec<f32>) =
                    (chunk.iter().map(|x| x * 2.0).collect(), chunk.iter().map(|x| x * 2.0).collect());
                m.process(rate, true, chunk, chunk, &mut l, &mut r);
                last = l.iter().zip(chunk).map(|(o, i)| if i.abs() > 0.05 { o / i } else { 0.0 }).fold(0.0, f32::max);
            }
        }
        // 2x louder output ends up at the input's level.
        assert!((last - 1.0).abs() < 0.02, "ratio {last}");
    }

    /// Short beeps that come out of DTS late and louder are matched, also right after switching on.
    #[test]
    fn late_beeps_matched_once_switched_on() {
        let (rate, late) = (48000, 2400);
        // A 50 ms beep every half second; the "processed" copy is 2x louder and 50 ms late.
        let inp: Vec<f32> = (0..rate as usize * 10)
            .map(|i| if i % 24000 < 2400 { (i as f32 * 0.06).sin() * 0.1 } else { 0.0 })
            .collect();
        let out: Vec<f32> = (0..inp.len()).map(|i| if i >= late { 2.0 * inp[i - late] } else { 0.0 }).collect();
        let mut m = LoudnessMatch::default();
        let mut got = Vec::new();
        for (k, (i, o)) in inp.chunks(1024).zip(out.chunks(1024)).enumerate() {
            let (mut l, mut r) = (o.to_vec(), o.to_vec());
            // Off for 8 s (measuring only), then on.
            m.process(rate, k * 1024 >= 8 * rate as usize, i, i, &mut l, &mut r);
            got.extend(l);
        }
        let rms = |x: &[f32]| (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
        let s = 8 * rate as usize;
        assert!((rms(&got[s - 48000..s]) / rms(&out[s - 48000..s]) - 1.0).abs() < 1e-6, "untouched while off");
        // From 0.2 s after switching on, the beeps play at the input's level.
        let ratio = rms(&got[s + 9600..]) / rms(&inp[s + 9600 - late..inp.len() - late]);
        assert!((ratio - 1.0).abs() < 0.05, "ratio {ratio}");
    }
}
