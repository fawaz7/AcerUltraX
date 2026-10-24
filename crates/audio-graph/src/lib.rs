//! The signal chain Windows runs for DTS:X Ultra, for one output:
//!
//! ```text
//! app stream (1/2/6/8 ch) ─▶ SFX engine of its own ─┐
//! app stream ...          ─▶ SFX engine of its own ─┼▶ mix ─▶ MFX engine ─▶ [loudness match] ─▶ volume ─▶ out
//! 4-channel stream (front pair, unprocessed)       ─┘
//! ```
//!
//! [`Chain::process`] runs on the audio thread. Mode changes are prepared off it with [`Prepared::build`] (with
//! ready-made engines for the streams already playing, [`Prepared::add_spares`]) and swapped in with
//! [`Chain::set_mode`]; only a stream that starts later gets its engine made on the audio thread (about 1 ms).
//! Nothing is freed on the audio thread: whatever the chain lets go of goes to [`Chain::take_trash`].

mod eq;
mod loudness;
mod response;

use std::collections::HashMap;

use std::sync::Arc;

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
    /// Spatial sound for 5.1/7.1 streams, if on.
    spatial: Option<Spatial>,
    /// Stereo stream engines made in advance, taken by streams as they need one.
    spares: Vec<Engine>,
}

/// Something the chain no longer needs, to be freed off the audio thread.
pub enum Garbage {
    Mode(Prepared),
    Engine(Engine),
    Spatial(dts_spatial::Encoder),
}

/// How much the chain can set aside between two [`Chain::take_trash`] calls; beyond that it frees in place.
pub const TRASH: usize = 64;

fn discard(trash: &mut Vec<Garbage>, g: Garbage) {
    if trash.len() < trash.capacity() {
        trash.push(g);
    }
}

fn discard_stream(trash: &mut Vec<Garbage>, st: StreamEngine) {
    if let Some(e) = st.engine {
        discard(trash, Garbage::Engine(e));
    }
    if let Some(e) = st.spatial {
        discard(trash, Garbage::Spatial(e));
    }
}

/// Windows' spatial sound for this output: multichannel streams go through the spatial encoder (instead of
/// their stream engine) and on into the mix, as Windows feeds the encoder's output to the rest of the chain.
#[derive(Clone)]
pub struct Spatial {
    pub module: &'static pe_loader::Module,
    pub render: dts_spatial::Render,
    /// The spatial preset and device profile.
    pub preset: Arc<Vec<u8>>,
    pub device: Arc<Vec<u8>>,
}

impl Prepared {
    /// Configure a mode for `rate` (slow: reads tuning files; call off the audio thread).
    /// `eq`: equaliser gains in tenths of a dB per band ([`EQ_BANDS`]), applied after DTS.
    pub fn build(
        lib: &'static Library,
        tuning: &ModeTuning,
        rate: u32,
        eq: Option<[i32; 10]>,
        spatial: Option<Spatial>,
    ) -> Result<Prepared, dts_eagle::Error> {
        let (mut sfx, _) = dts_eagle::configured_engine(lib, tuning, Location::Sfx, 2, rate)?;
        let (mfx, _) = dts_eagle::configured_engine(lib, tuning, Location::Mfx, 2, rate)?;
        let spatial = spatial.filter(|_| rate == dts_spatial::RATE);
        Ok(Prepared {
            rate,
            sfx_blob: sfx.tuning_blob(),
            mfx,
            eq: eq.map(|g| Eq::new(g, rate)),
            spatial,
            spares: Vec::new(),
        })
    }

    /// Make `n` stereo stream engines in advance (off the audio thread), so streams already playing switch
    /// mode without engines being made on the audio thread.
    pub fn add_spares(&mut self, lib: &'static Library, n: usize) {
        self.spares.extend((0..n).filter_map(|_| new_stream_engine(lib, &self.sfx_blob, 2, self.rate)));
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
    /// The spatial encoder, for a multichannel stream while spatial sound is on.
    spatial: Option<dts_spatial::Encoder>,
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
    /// Stream ids that went away this block (scratch).
    gone: Vec<u32>,
    trash: Vec<Garbage>,
    /// Scratch for a stream's spatial encoder output.
    spatial_out: [Vec<f32>; 2],
    /// Pass audio through without DTS (streams mixed down to stereo).
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
            gone: Vec::with_capacity(64),
            trash: Vec::with_capacity(TRASH),
            spatial_out: [vec![0.0; MAX_FRAMES], vec![0.0; MAX_FRAMES]],
            bypass: false,
            gain: 1.0,
            match_loudness: false,
            matcher: LoudnessMatch::default(),
        }
    }

    /// Swap in a new mode (or rate). Stream engines are recreated from it as audio arrives; the old ones and the
    /// old mode go to the trash.
    pub fn set_mode(&mut self, mode: Prepared) {
        for (_, st) in self.streams.drain() {
            discard_stream(&mut self.trash, st);
        }
        if let Some(old) = self.mode.replace(mode) {
            discard(&mut self.trash, Garbage::Mode(old));
        }
    }

    /// Stereo (or mono) streams being processed right now (not resting): how many ready-made engines the next
    /// mode should bring ([`Prepared::add_spares`]).
    pub fn busy_streams(&self) -> usize {
        self.streams.values().filter(|s| s.engine.is_some() && s.channels <= 2 && s.silent == 0).count()
    }

    /// Hand over what the chain no longer needs, for the caller to free off the audio thread. `bin` must be
    /// empty; the chain keeps `bin`'s capacity for next time (give it [`TRASH`] room). Allocates nothing.
    pub fn take_trash(&mut self, bin: &mut Vec<Garbage>) {
        if !self.trash.is_empty() && bin.is_empty() {
            std::mem::swap(&mut self.trash, bin);
        }
    }

    /// Sample rate of the current mode, if any.
    pub fn rate(&self) -> Option<u32> {
        self.mode.as_ref().map(|m| m.rate)
    }

    /// Process one block. Streams not present this block lose their engine. Without a mode for `rate`, or in
    /// bypass, the streams are mixed down to stereo unprocessed ([`downmix`]). `out` must be at most
    /// [`MAX_FRAMES`] long.
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
            downmix(s.channels, dry_l, dry_r);
        }

        let mode = match &mut self.mode {
            Some(m) if m.rate == rate && !self.bypass => m,
            _ => {
                for (_, st) in self.streams.drain() {
                    discard_stream(&mut self.trash, st);
                }
                apply_gain(dry_l, l, self.gain);
                apply_gain(dry_r, r, self.gain);
                return;
            }
        };

        // Drop engines of streams that are gone.
        self.seen_ids.clear();
        self.seen_ids.extend(streams.iter().map(|s| s.id));
        self.gone.clear();
        self.gone.extend(self.streams.keys().filter(|id| !self.seen_ids.contains(id)));
        for id in &self.gone {
            if let Some(st) = self.streams.remove(id) {
                discard_stream(&mut self.trash, st);
            }
        }

        let mut sfx_out = [[0f32; MAX_BLOCK]; 2];
        for s in streams {
            let ch = s.channels.len();
            let st = self.streams.entry(s.id).or_insert_with(|| StreamEngine {
                channels: ch,
                engine: None,
                spatial: None,
                silent: 0,
            });
            let silent = s.channels.iter().all(|c| c[..n].iter().all(|&x| x == 0.0));
            st.silent = if silent { st.silent + n } else { 0 };
            if st.silent > STREAM_REST_SECS * rate as usize {
                if st.silent > STREAM_FREE_SECS * rate as usize {
                    if let Some(e) = st.engine.take() {
                        discard(&mut self.trash, Garbage::Engine(e));
                    }
                    if let Some(e) = st.spatial.take() {
                        discard(&mut self.trash, Garbage::Spatial(e));
                    }
                }
                continue;
            }
            if let (Some(sp), 6 | 8) = (&mode.spatial, ch) {
                if st.spatial.is_none() || st.channels != ch {
                    st.channels = ch;
                    // shortcut: the encoder starts on the audio thread (~5 ms, once per stream); prepare it off
                    // the audio thread if that ever causes a dropout at small quantum sizes.
                    let new = dts_spatial::Encoder::new(sp.module, sp.render, &sp.preset, &sp.device, ch)
                        .map_err(|e| log::warn!("spatial sound unavailable for a {ch}-channel stream: {e}"))
                        .ok();
                    if let Some(old) = std::mem::replace(&mut st.spatial, new) {
                        discard(&mut self.trash, Garbage::Spatial(old));
                    }
                }
                if let Some(enc) = &mut st.spatial {
                    let [a, b] = &mut self.spatial_out;
                    if enc.process(s.channels, [&mut a[..n], &mut b[..n]]).is_ok() {
                        add(l, &a[..n]);
                        add(r, &b[..n]);
                        continue;
                    }
                }
            }
            if st.channels != ch || (st.engine.is_none() && ch != 4) {
                st.channels = ch;
                let new = match ch {
                    1 | 2 => mode.spares.pop().or_else(|| new_stream_engine(self.lib, &mode.sfx_blob, ch, rate)),
                    _ => new_stream_engine(self.lib, &mode.sfx_blob, ch, rate),
                };
                if let Some(old) = std::mem::replace(&mut st.engine, new) {
                    discard(&mut self.trash, Garbage::Engine(old));
                }
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
        self.matcher.process(rate, self.match_loudness, dry_l, dry_r, l, r);
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

/// Level of the centre and surround channels in a stereo downmix (-3 dB).
const DOWNMIX: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// Level of the LFE channel in it (-9 dB).
const DOWNMIX_LFE: f32 = 0.5 * DOWNMIX;

/// Add a stream to a stereo mix exactly as PipeWire mixes it into a stereo device without AcerUltraX
/// (measured, PipeWire 1.6 defaults): mono on both sides; centre and surrounds at [`DOWNMIX`]; LFE at
/// [`DOWNMIX_LFE`] on both sides. Channels are 1, 2, FL FR + a surround pair (4), FL FR FC LFE + a surround pair
/// (6) or FL FR FC LFE BL BR SL SR (8); any other count contributes its front pair.
fn downmix(ch: &[&[f32]], l: &mut [f32], r: &mut [f32]) {
    let n = l.len();
    let (fl, fr) = front_pair(ch);
    add(l, &fl[..n]);
    add(r, &fr[..n]);
    let (centre, sides): (Option<&[f32]>, &[&[f32]]) = match ch.len() {
        4 => (None, &ch[2..]),
        6 | 8 => (Some(ch[2]), &ch[4..]),
        _ => (None, &[]),
    };
    if let Some(c) = centre {
        add_scaled(l, &c[..n], DOWNMIX);
        add_scaled(r, &c[..n], DOWNMIX);
        add_scaled(l, &ch[3][..n], DOWNMIX_LFE);
        add_scaled(r, &ch[3][..n], DOWNMIX_LFE);
    }
    for pair in sides.chunks_exact(2) {
        add_scaled(l, &pair[0][..n], DOWNMIX);
        add_scaled(r, &pair[1][..n], DOWNMIX);
    }
}

fn add_scaled(dst: &mut [f32], src: &[f32], g: f32) {
    dst.iter_mut().zip(src).for_each(|(d, s)| *d += s * g);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One channel at 1.0, the others silent: where it lands in the stereo downmix.
    fn place(width: usize, on: usize) -> (f32, f32) {
        let chans: Vec<Vec<f32>> = (0..width).map(|c| vec![if c == on { 1.0 } else { 0.0 }]).collect();
        let views: Vec<&[f32]> = chans.iter().map(|c| c.as_slice()).collect();
        let (mut l, mut r) = ([0f32], [0f32]);
        downmix(&views, &mut l, &mut r);
        (l[0], r[0])
    }

    #[test]
    fn downmix_like_pipewire() {
        let (h, q) = (DOWNMIX, DOWNMIX_LFE);
        assert_eq!(place(1, 0), (1.0, 1.0));
        assert_eq!([place(2, 0), place(2, 1)], [(1.0, 0.0), (0.0, 1.0)]);
        assert_eq!([place(4, 2), place(4, 3)], [(h, 0.0), (0.0, h)]);
        let six: Vec<_> = (0..6).map(|c| place(6, c)).collect();
        assert_eq!(six, [(1.0, 0.0), (0.0, 1.0), (h, h), (q, q), (h, 0.0), (0.0, h)]);
        let eight: Vec<_> = (0..8).map(|c| place(8, c)).collect();
        assert_eq!(eight[2..], [(h, h), (q, q), (h, 0.0), (0.0, h), (h, 0.0), (0.0, h)]);
    }
}
