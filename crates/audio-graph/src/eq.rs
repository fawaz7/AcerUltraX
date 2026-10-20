//! A 10-band graphic equaliser after DTS, for profiles made by the user: one-octave peaking filters (RBJ
//! cookbook) at the standard centres. Boosts lower the overall level by the largest boost so the result never
//! clips.

/// Band centres in Hz, lowest first.
pub const BANDS: [f32; 10] = [31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
/// One octave wide.
const Q: f64 = std::f64::consts::SQRT_2;

#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
}

impl Biquad {
    fn peaking(f: f64, gain_db: f64, rate: f64) -> Biquad {
        let a = 10f64.powf(gain_db / 40.0);
        let w = 2.0 * std::f64::consts::PI * f / rate;
        let alpha = w.sin() / (2.0 * Q);
        let a0 = 1.0 + alpha / a;
        Biquad {
            b: [(1.0 + alpha * a) / a0, -2.0 * w.cos() / a0, (1.0 - alpha * a) / a0],
            a: [-2.0 * w.cos() / a0, (1.0 - alpha / a) / a0],
        }
    }

    /// Gain at `f` Hz, in dB.
    fn response_db(&self, f: f64, rate: f64) -> f64 {
        let w = 2.0 * std::f64::consts::PI * f / rate;
        let z = |c: [f64; 3]| {
            let (re, im) = (c[0] + c[1] * w.cos() + c[2] * (2.0 * w).cos(), -(c[1] * w.sin() + c[2] * (2.0 * w).sin()));
            re * re + im * im
        };
        10.0 * (z(self.b) / z([1.0, self.a[0], self.a[1]])).log10()
    }
}

pub struct Eq {
    filters: Vec<Biquad>,
    /// Per channel, per filter: the two state values (transposed direct form II).
    state: [Vec<[f64; 2]>; 2],
    pre: f64,
    rate: f64,
}

impl Eq {
    /// Gains per band in tenths of a dB (clamped to ±15 dB). Bands at 0 (or above the rate's limit) are skipped.
    pub fn new(gains: [i32; 10], rate: u32) -> Eq {
        let rate = rate as f64;
        let mut filters = Vec::new();
        for (f, g) in BANDS.iter().zip(gains) {
            let g = g.clamp(-150, 150) as f64 / 10.0;
            if g != 0.0 && (*f as f64) < rate * 0.45 {
                filters.push(Biquad::peaking(*f as f64, g, rate));
            }
        }
        let n = filters.len();
        let boost = gains.iter().copied().max().unwrap_or(0).clamp(0, 150) as f64 / 10.0;
        Eq { filters, state: [vec![[0.0; 2]; n], vec![[0.0; 2]; n]], pre: 10f64.powf(-boost / 20.0), rate }
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (ch, buf) in [l, r].into_iter().enumerate() {
            for x in buf.iter_mut() {
                let mut v = *x as f64 * self.pre;
                for (f, s) in self.filters.iter().zip(self.state[ch].iter_mut()) {
                    let y = f.b[0] * v + s[0];
                    s[0] = f.b[1] * v - f.a[0] * y + s[1];
                    s[1] = f.b[2] * v - f.a[1] * y;
                    v = y;
                }
                *x = v as f32;
            }
        }
    }

    /// The equaliser's gain at `f` Hz in dB, input gain included.
    pub fn response_db(&self, f: f32) -> f32 {
        let fs: f64 = self.filters.iter().map(|b| b.response_db(f as f64, self.rate)).sum();
        (fs + 20.0 * self.pre.log10()) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_and_headroom() {
        let mut g = [0; 10];
        g[2] = 60; // +6 dB at 125 Hz
        g[7] = -90; // -9 dB at 4 kHz
        let eq = Eq::new(g, 48000);
        // The largest boost is taken off the overall level: +6 dB band peaks at 0 dB, flat parts sit at -6 dB.
        assert!((eq.response_db(125.0) - 0.0).abs() < 0.3, "{}", eq.response_db(125.0));
        assert!((eq.response_db(4000.0) - -15.0).abs() < 0.5, "{}", eq.response_db(4000.0));
        assert!((eq.response_db(1000.0) - -6.0).abs() < 0.7, "{}", eq.response_db(1000.0));
        // A sine through the filter matches the computed response.
        let mut e = Eq::new(g, 48000);
        let (mut l, mut r): (Vec<f32>, Vec<f32>) =
            (0..48000).map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 4000.0 / 48000.0).sin()).unzip_pair();
        e.process(&mut l, &mut r);
        let peak = l[24000..].iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!((20.0 * peak.log10() - eq.response_db(4000.0)).abs() < 0.2);
        assert!(Eq::new([0; 10], 48000).filters.is_empty());
    }

    trait UnzipPair {
        fn unzip_pair(self) -> (Vec<f32>, Vec<f32>);
    }
    impl<I: Iterator<Item = f32>> UnzipPair for I {
        fn unzip_pair(self) -> (Vec<f32>, Vec<f32>) {
            let v: Vec<f32> = self.collect();
            (v.clone(), v)
        }
    }
}
