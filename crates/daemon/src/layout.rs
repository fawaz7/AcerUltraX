//! Channel layouts: which channels a stream's DTS input gets, in the order the DTS engine expects
//! (WAVEFORMATEXTENSIBLE: FL FR FC LFE BL BR SL SR).

use libspa_sys as spa;

/// (SPA position, PipeWire name), in WAVEFORMATEXTENSIBLE order.
const WAVE: &[(u32, &str)] = &[
    (spa::SPA_AUDIO_CHANNEL_FL, "FL"),
    (spa::SPA_AUDIO_CHANNEL_FR, "FR"),
    (spa::SPA_AUDIO_CHANNEL_FC, "FC"),
    (spa::SPA_AUDIO_CHANNEL_LFE, "LFE"),
    (spa::SPA_AUDIO_CHANNEL_RL, "RL"),
    (spa::SPA_AUDIO_CHANNEL_RR, "RR"),
    (spa::SPA_AUDIO_CHANNEL_FLC, "FLC"),
    (spa::SPA_AUDIO_CHANNEL_FRC, "FRC"),
    (spa::SPA_AUDIO_CHANNEL_RC, "RC"),
    (spa::SPA_AUDIO_CHANNEL_SL, "SL"),
    (spa::SPA_AUDIO_CHANNEL_SR, "SR"),
];

/// The channels of a stream's DTS input. Windows processes 1 (as stereo), 2, 6 and 8 channels in their own
/// layout and passes 4 channels' front pair through; anything else becomes stereo (PipeWire downmixes it into
/// the stream's input). The result is always FL FR, then FC LFE (6 and 8 channels), then one surround pair (4
/// and 6) or two (8): the chain relies on that. Other layouts get the standard one for their channel count and
/// PipeWire remixes them into it.
pub fn group_layout(positions: &[u32]) -> Vec<&'static str> {
    let n = positions.len();
    let width = if matches!(n, 1 | 2 | 4 | 6 | 8) { n } else { 2 };
    if width == 1 {
        return vec!["MONO"];
    }
    let rank = |p: u32| WAVE.iter().position(|w| w.0 == p);
    let mut ranks: Vec<usize> = positions.iter().filter_map(|&p| rank(p)).collect();
    ranks.sort_unstable();
    ranks.dedup();
    let names: Vec<&str> = ranks.iter().map(|&r| WAVE[r].1).collect();
    let fits = match names.as_slice() {
        ["FL", "FR"] => true,
        ["FL", "FR", s, _] => matches!(*s, "RL" | "SL"),
        ["FL", "FR", "FC", "LFE", s, _] => matches!(*s, "RL" | "SL"),
        ["FL", "FR", "FC", "LFE", "RL", "RR", "SL", "SR"] => true,
        _ => false,
    };
    if width == n && fits {
        return names;
    }
    match width {
        4 => vec!["FL", "FR", "RL", "RR"],
        6 => vec!["FL", "FR", "FC", "LFE", "RL", "RR"],
        8 => vec!["FL", "FR", "FC", "LFE", "RL", "RR", "SL", "SR"],
        _ => vec!["FL", "FR"],
    }
}

/// Which of the processed output's ports (FL, FR or their MONO mix) feeds a device channel. Stereo devices get
/// the front pair; devices without one (mono, or unnamed channels) get the mono mix everywhere. The laptop's
/// speakers also get their extra channels fed, as the kernel does for a stereo stream (Linux shows bass or rear
/// speaker pairs as 2.1/4.0 layouts once another profile is picked); other surround devices keep stereo on
/// the front pair, as Windows plays it.
pub fn output_source(channel: &str, stereo: bool, speakers: bool) -> Option<&'static str> {
    match channel {
        "FL" | "FR" if stereo => Some(if channel == "FL" { "FL" } else { "FR" }),
        _ if !stereo => Some("MONO"),
        _ if !speakers => None,
        c if c.ends_with('L') => Some("FL"),
        c if c.ends_with('R') => Some("FR"),
        _ => Some("MONO"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libspa_sys::*;

    #[test]
    fn layouts() {
        assert_eq!(group_layout(&[SPA_AUDIO_CHANNEL_MONO]), ["MONO"]);
        // Spotify-style unpositioned stereo.
        assert_eq!(group_layout(&[SPA_AUDIO_CHANNEL_UNKNOWN, SPA_AUDIO_CHANNEL_UNKNOWN]), ["FL", "FR"]);
        // 7.1 in PipeWire order comes out in WAVE order.
        let pw71 = [
            SPA_AUDIO_CHANNEL_FL,
            SPA_AUDIO_CHANNEL_FR,
            SPA_AUDIO_CHANNEL_FC,
            SPA_AUDIO_CHANNEL_LFE,
            SPA_AUDIO_CHANNEL_SL,
            SPA_AUDIO_CHANNEL_SR,
            SPA_AUDIO_CHANNEL_RL,
            SPA_AUDIO_CHANNEL_RR,
        ];
        assert_eq!(group_layout(&pw71), ["FL", "FR", "FC", "LFE", "RL", "RR", "SL", "SR"]);
        // 5.1 with side channels keeps them.
        let side51 = &pw71[..6];
        assert_eq!(group_layout(side51), ["FL", "FR", "FC", "LFE", "SL", "SR"]);
        // 3 channels become stereo.
        assert_eq!(group_layout(&pw71[..3]), ["FL", "FR"]);
        // Quad keeps its surround pair; 3.1 and 6.0 get the standard layout (PipeWire remixes into it).
        assert_eq!(group_layout(&[pw71[0], pw71[1], pw71[4], pw71[5]]), ["FL", "FR", "SL", "SR"]);
        assert_eq!(group_layout(&pw71[..4]), ["FL", "FR", "RL", "RR"]);
        let six0 = [pw71[0], pw71[1], pw71[2], SPA_AUDIO_CHANNEL_RC, pw71[4], pw71[5]];
        assert_eq!(group_layout(&six0), ["FL", "FR", "FC", "LFE", "RL", "RR"]);
    }

    #[test]
    fn output_channels() {
        let map = |chs: &[&str], speakers| {
            let stereo = chs.contains(&"FL") && chs.contains(&"FR");
            chs.iter().map(|c| output_source(c, stereo, speakers)).collect::<Vec<_>>()
        };
        assert_eq!(map(&["FL", "FR"], true), [Some("FL"), Some("FR")]);
        assert_eq!(map(&["MONO"], false), [Some("MONO")]);
        // Laptop speakers on a 2.1 or 4.0 profile: woofer gets the mono mix, rear pair a copy of the front.
        assert_eq!(map(&["FL", "FR", "LFE"], true), [Some("FL"), Some("FR"), Some("MONO")]);
        assert_eq!(map(&["FL", "FR", "RL", "RR"], true), [Some("FL"), Some("FR"), Some("FL"), Some("FR")]);
        // A 5.1 receiver keeps stereo on its front speakers.
        assert_eq!(map(&["FL", "FR", "FC", "LFE", "RL", "RR"], false)[2..], [None, None, None, None]);
    }
}
