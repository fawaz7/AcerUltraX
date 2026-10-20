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
/// the stream's input). Unpositioned or odd layouts get the standard one for their channel count.
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
    if width == n && ranks.len() == n {
        return ranks.iter().map(|&r| WAVE[r].1).collect();
    }
    match width {
        4 => vec!["FL", "FR", "RL", "RR"],
        6 => vec!["FL", "FR", "FC", "LFE", "RL", "RR"],
        8 => vec!["FL", "FR", "FC", "LFE", "RL", "RR", "SL", "SR"],
        _ => vec!["FL", "FR"],
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
    }
}
