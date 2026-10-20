//! The signal chain Windows runs for DTS:X Ultra, for one output:
//!
//! ```text
//! app stream (1/2/6/8 ch) ─▶ SFX engine of its own ─┐
//! app stream ...          ─▶ SFX engine of its own ─┼▶ mix ─▶ MFX engine ─▶ [loudness match] ─▶ volume ─▶ out
//! 4-channel stream (front pair, unprocessed)       ─┘
//! ```
//!
//! [`Chain::process`] runs on the audio thread. Mode changes are prepared off it with [`Prepared::build`] and
//! swapped in with [`Chain::set_mode`]; a new stream's engine is created from the mode's tuning blob (about 1 ms).

mod eq;
mod loudness;
mod response;

use std::collections::HashMap;

use dts_eagle::{Blob, Engine, Library, Location, MAX_BLOCK, ModeTuning};

pub use eq::{BANDS as EQ_BANDS, Eq};
pub use loudness::LoudnessMatch;
pub use response::response;

/// Largest block [`Chain::process`] handles in one call.
pub const MAX_FRAMES: usize = 8192;

/// A mode ready to swap in: the per-stream engine state and a configured mix engine.
pub struct Prepared {
    rate: u32,
    sfx_blob: Blob,
    mfx: Engine,
    /// The user's equaliser after DTS, if the profile has one.
    eq: Option<Eq>,
}

impl Prepared {
    /// Configure a mode for `rate` (slow: reads tuning files; call off the audio thread).
    /// `eq`: equaliser gains in tenths of a dB per band ([`EQ_BANDS`]), applied after DTS.
    pub fn build(
        lib: &'static Library,
        tuning: &ModeTuning,
        rate: u32,
        eq: Option<[i32; 10]>,
    ) -> Result<Prepared, dts_eagle::Error> {
        let (mut sfx, _) = dts_eagle::configured_engine(lib, tuning, Location::Sfx, 2, rate)?;
        let (mfx, _) = dts_eagle::configured_engine(lib, tuning, Location::Mfx, 2, rate)?;
        Ok(Prepared { rate, sfx_blob: sfx.tuning_blob(), mfx, eq: eq.map(|g| Eq::new(g, rate)) })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }
}

/// One app stream's audio for this block: planar channels in WAVEFORMATEXTENSIBLE order
/// (FL FR FC LFE BL BR SL SR).
pub struct StreamInput<'a> {
    /// Stable id of the stream (e.g. its PipeWire serial).
    pub id: u32,
    pub channels: &'a [&'a [f32]],
}

struct StreamEngine {
    channels: usize,
    /// None when the stream is not processed by SFX (4 channels) or its engine failed.
    engine: Option<Engine>,
    /// Consecutive frames of digital silence on this stream's input.
    silent: usize,
}

/// An idle stream's engine rests after this many seconds of digital silence (apps often keep a silent stream
/// open; Windows does not process a stopped stream either). It resumes on the first non-zero sample.
const STREAM_REST_SECS: usize = 1;
/// After this long idle, a stream's engine is freed (about 3 MB each) and made afresh when sound returns.
const STREAM_FREE_SECS: usize = 30;
/// The mode engine rests once the mix has been silent this long, so its tail has finished.
const MIX_REST_SECS: usize = 2;

pub struct Chain {
    lib: &'static Library,
    mode: Option<Prepared>,
    streams: HashMap<u32, StreamEngine>,
    seen_ids: Vec<u32>,
    mix: [Vec<f32>; 2],
    /// Consecutive frames of digital silence going into the mode engine.
    mix_silent: usize,
    /// Pass audio through without DTS (front pairs mixed).
    pub bypass: bool,
    /// Volume applied after DTS, as Windows' endpoint volume (linear).
    pub gain: f32,
    /// Scale DTS output down to the loudness of the unprocessed audio (for fair comparisons).
    pub match_loudness: bool,
    matcher: LoudnessMatch,
}

impl Chain {
    pub fn new(lib: &'static Library) -> Chain {
        Chain {
            lib,
            mode: None,
            streams: HashMap::new(),
            seen_ids: Vec::new(),
            mix: [vec![0.0; MAX_FRAMES], vec![0.0; MAX_FRAMES]],
            mix_silent: 0,
            bypass: false,
            gain: 1.0,
            match_loudness: false,
            matcher: LoudnessMatch::default(),
        }
    }

    /// Swap in a new mode (or rate). Stream engines are recreated from it as audio arrives. Returns the old mode
    /// so the caller can drop it off the audio thread.
    pub fn set_mode(&mut self, mode: Prepared) -> Option<Prepared> {
        self.streams.clear();
        self.mode.replace(mode)
    }

    /// Sample rate of the current mode, if any.
    pub fn rate(&self) -> Option<u32> {
        self.mode.as_ref().map(|m| m.rate)
    }

    /// Process one block. Streams not present this block lose their engine. Without a mode for `rate`, or in
    /// bypass, the streams' front pairs are mixed unprocessed. `out` must be at most [`MAX_FRAMES`] long.
    pub fn process(&mut self, rate: u32, streams: &[StreamInput], out: [&mut [f32]; 2]) {
        let n = out[0].len().min(out[1].len()).min(MAX_FRAMES);
        let [l, r] = out;
        let (l, r) = (&mut l[..n], &mut r[..n]);
        l.fill(0.0);
        r.fill(0.0);
        let [dry_l, dry_r] = &mut self.mix;
        let (dry_l, dry_r) = (&mut dry_l[..n], &mut dry_r[..n]);
        dry_l.fill(0.0);
        dry_r.fill(0.0);
        for s in streams {
            let (a, b) = front_pair(s.channels);
            add(dry_l, &a[..n]);
            add(dry_r, &b[..n]);
        }

        let mode = match &mut self.mode {
            Some(m) if m.rate == rate && !self.bypass => m,
            _ => {
                self.streams.clear();
                apply_gain(dry_l, l, self.gain);
                apply_gain(dry_r, r, self.gain);
                return;
            }
        };

        // Drop engines of streams that are gone.
        self.seen_ids.clear();
        self.seen_ids.extend(streams.iter().map(|s| s.id));
        let seen = &self.seen_ids;
        self.streams.retain(|id, _| seen.contains(id));

        let mut sfx_out = [[0f32; MAX_BLOCK]; 2];
        for s in streams {
            let ch = s.channels.len();
            let st = self.streams.entry(s.id).or_insert_with(|| StreamEngine { channels: ch, engine: None, silent: 0 });
            let silent = s.channels.iter().all(|c| c[..n].iter().all(|&x| x == 0.0));
            st.silent = if silent { st.silent + n } else { 0 };
            if st.silent > STREAM_REST_SECS * rate as usize {
                if st.silent > STREAM_FREE_SECS * rate as usize {
                    st.engine = None;
                }
                continue;
            }
            if st.channels != ch || (st.engine.is_none() && ch != 4) {
                st.channels = ch;
                st.engine = new_stream_engine(self.lib, &mode.sfx_blob, ch, rate);
            }
            match &mut st.engine {
                None => {
                    // 4 channels (Windows passes the front pair without SFX), or the engine could not be made.
                    let (a, b) = front_pair(s.channels);
                    add(l, &a[..n]);
                    add(r, &b[..n]);
                }
                Some(e) => {
                    for off in (0..n).step_by(MAX_BLOCK) {
                        let k = (n - off).min(MAX_BLOCK);
                        let mut ins = [&[][..]; 8];
                        let width = if ch == 1 { 2 } else { ch };
                        for (c, slot) in ins.iter_mut().enumerate().take(width) {
                            *slot = &s.channels[if ch == 1 { 0 } else { c }][off..off + k];
                        }
                        let [o0, o1] = &mut sfx_out;
                        if e.process(&ins[..width], &mut [&mut o0[..k], &mut o1[..k]]).is_err() {
                            continue;
                        }
                        add(&mut l[off..off + k], &o0[..k]);
                        add(&mut r[off..off + k], &o1[..k]);
                    }
                }
            }
        }

        // MFX on the mix, in place.
        let silent = l.iter().chain(r.iter()).all(|&x| x == 0.0);
        self.mix_silent = if silent { self.mix_silent + n } else { 0 };
        let resting = self.mix_silent > MIX_REST_SECS * rate as usize;
        for off in (0..n).step_by(MAX_BLOCK).filter(|_| !resting) {
            let k = (n - off).min(MAX_BLOCK);
            let mut m = [[0f32; MAX_BLOCK]; 2];
            m[0][..k].copy_from_slice(&l[off..off + k]);
            m[1][..k].copy_from_slice(&r[off..off + k]);
            let (lo, ro) = (&mut l[off..off + k], &mut r[off..off + k]);
            let _ = mode.mfx.process(&[&m[0][..k], &m[1][..k]], &mut [lo, ro]);
        }
        if let Some(eq) = &mut mode.eq {
            eq.process(l, r);
        }
        if self.match_loudness {
            self.matcher.process(rate, dry_l, dry_r, l, r);
        }
        apply_gain_in_place(l, self.gain);
        apply_gain_in_place(r, self.gain);
    }
}

fn new_stream_engine(lib: &'static Library, blob: &Blob, ch: usize, rate: u32) -> Option<Engine> {
    let width = match ch {
        1 => 2, // mono is duplicated to both sides, as the Windows APO does
        2 | 6 | 8 => ch,
        _ => return None,
    };
    match dts_eagle::engine_from_blob(lib, blob, Location::Sfx, width, rate) {
        Ok(e) => Some(e),
        Err(err) => {
            log::warn!("cannot create a {ch}-channel stream engine: {err}");
            None
        }
    }
}

fn front_pair<'a>(ch: &[&'a [f32]]) -> (&'a [f32], &'a [f32]) {
    (ch[0], if ch.len() > 1 { ch[1] } else { ch[0] })
}

fn add(dst: &mut [f32], src: &[f32]) {
    dst.iter_mut().zip(src).for_each(|(d, s)| *d += s);
}

fn apply_gain(src: &[f32], dst: &mut [f32], g: f32) {
    dst.iter_mut().zip(src).for_each(|(d, s)| *d = s * g);
}

fn apply_gain_in_place(x: &mut [f32], g: f32) {
    if g != 1.0 {
        x.iter_mut().for_each(|v| *v *= g);
    }
}
