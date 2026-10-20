//! Loudness matching for fair comparisons: DTS is several dB louder than the unprocessed audio, and louder
//! always sounds "better". Both signals are K-weighted (ITU-R BS.1770) and their energies smoothed over 3 s;
//! the output is scaled to the input's loudness, only ever down, by at most 30 dB, the gain smoothed over 0.3 s.

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

#[derive(Default)]
pub struct LoudnessMatch {
    rate: u32,
    k_in: [[Biquad; 2]; 2],
    k_out: [[Biquad; 2]; 2],
    e_in: f64,
    e_out: f64,
    gain: f64,
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
    /// Scale `out` (processed) towards the loudness of `inp` (unprocessed), in place.
    pub fn process(&mut self, rate: u32, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        if self.rate != rate {
            *self = LoudnessMatch { rate, gain: 1.0, ..Default::default() };
            let k = k_weighting(rate as f64);
            self.k_in = [k, k];
            self.k_out = [k, k];
        }
        let n = out_l.len();
        let (ei, eo) = (energy(&mut self.k_in, in_l, in_r), energy(&mut self.k_out, out_l, out_r));
        // Near-silence (below about -70 dB) does not move the estimate.
        if ei > 1e-7 {
            let a = 1.0 - (-(n as f64) / (rate as f64 * 3.0)).exp();
            if self.e_in <= 0.0 {
                (self.e_in, self.e_out) = (ei, eo);
            }
            self.e_in += a * (ei - self.e_in);
            self.e_out += a * (eo - self.e_out);
        }
        let target = if self.e_out > 0.0 { (self.e_in / self.e_out).sqrt() } else { 1.0 }.clamp(0.0316, 1.0);
        let next = self.gain + (target - self.gain) * (1.0 - (-(n as f64) / (rate as f64 * 0.3)).exp());
        for i in 0..n {
            let g = (self.gain + (next - self.gain) * (i + 1) as f64 / n as f64) as f32;
            out_l[i] *= g;
            out_r[i] *= g;
        }
        self.gain = next;
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
                m.process(rate, chunk, chunk, &mut l, &mut r);
                last = l.iter().zip(chunk).map(|(o, i)| if i.abs() > 0.05 { o / i } else { 0.0 }).fold(0.0, f32::max);
            }
        }
        // 2x louder output ends up at the input's level.
        assert!((last - 1.0).abs() < 0.02, "ratio {last}");
    }
}
