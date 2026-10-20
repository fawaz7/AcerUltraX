//! Windows' "Spatial sound: DTS:X Ultra" / "DTS Headphone:X": the spatial encoder (`DtsHrtfEnc.dll`) and its
//! presets, from the DTS app in Acer's "XPERI DTS Utility" download (DTS Sound Unbound inside).
//!
//! Presets (`Data/SAD/*.SPAC.crypt`, `Data/HeadphoneTuning/*.rdev.crypt`) are encrypted like the headphone EQs;
//! decrypted, they are `u32 length | blob | zero padding`. They are stored as plain blobs in `dts/spatial/`.

use std::path::Path;

use crate::headphones::{KEY_DLLS, aes_ecb_pkcs7, key_candidates};
use crate::sources::Entry;
use crate::{Error, io};

pub const ENCODER: &str = "DtsHrtfEnc.dll";

/// The blob inside a decrypted preset.
fn preset_blob(plain: &[u8]) -> Option<&[u8]> {
    let n = u32::from_le_bytes(plain.get(..4)?.try_into().ok()?) as usize;
    let blob = plain.get(4..4 + n)?;
    (n > 0 && plain[4 + n..].iter().all(|&b| b == 0)).then_some(blob)
}

/// Import the encoder and presets into `dts_dir/spatial`. Returns the number of presets (0: none in the sources).
pub fn import(entries: &[Entry], dts_dir: &Path) -> Result<usize, Error> {
    let x64 = |e: &Entry| e.parts.iter().any(|p| p.eq_ignore_ascii_case("x64"));
    let Some(encoder) = entries.iter().find(|e| e.name().eq_ignore_ascii_case(ENCODER) && x64(e)) else { return Ok(0) };
    let presets: Vec<&Entry> = entries
        .iter()
        .filter(|e| {
            let n = e.name().to_ascii_lowercase();
            n.ends_with(".spac.crypt") || n.ends_with(".rdev.crypt")
        })
        .collect();
    let Some(sample) = presets.first() else { return Ok(0) };
    let sample = sample.read().map_err(io(Path::new(&sample.describe())))?;
    let key = entries
        .iter()
        .filter(|e| KEY_DLLS.iter().any(|d| d.eq_ignore_ascii_case(e.name())))
        .filter_map(|e| e.read().ok())
        .flat_map(|b| key_candidates(&b))
        .find(|k| aes_ecb_pkcs7(k, &sample).as_deref().and_then(preset_blob).is_some())
        .ok_or_else(|| Error::Manifest("spatial presets found, but not the DTS app needed to read them".into()))?;

    let out = dts_dir.join("spatial");
    let staging = dts_dir.join("spatial.new");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(io(&staging))?;
    let bytes = encoder.read().map_err(io(Path::new(&encoder.describe())))?;
    std::fs::write(staging.join(ENCODER), bytes).map_err(io(&staging))?;
    let mut n = 0;
    for e in presets {
        let data = e.read().map_err(io(Path::new(&e.describe())))?;
        let Some(plain) = aes_ecb_pkcs7(&key, &data) else { continue };
        let Some(blob) = preset_blob(&plain) else { continue };
        let name = e.name().trim_end_matches(".crypt").trim_end_matches(".CRYPT");
        std::fs::write(staging.join(name), blob).map_err(io(&staging))?;
        n += 1;
    }
    let _ = std::fs::remove_dir_all(&out);
    std::fs::rename(&staging, &out).map_err(io(&out))?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_layout() {
        let mut p = 3u32.to_le_bytes().to_vec();
        p.extend([1, 2, 3, 0, 0]);
        assert_eq!(preset_blob(&p), Some(&[1u8, 2, 3][..]));
        p.push(9);
        assert_eq!(preset_blob(&p), None, "anything but zero padding after the blob is rejected");
        assert_eq!(preset_blob(&[0, 0, 0, 0]), None);
    }
}
