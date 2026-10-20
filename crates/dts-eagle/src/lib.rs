//! The DTS Eagle engine that Acer's Windows DTS:X Ultra software runs, loaded natively from the user's own copy
//! of `dtstech64.dll`, and the OEM tuning that configures it.
//!
//! Windows runs one engine per app stream (SFX, in the stream's channel layout) and one on the mix (MFX). Each is
//! configured either control by control from the OEM configuration or, equivalently, from a tuning blob.

mod engine;
pub mod oem;

pub use engine::{BLOB_SIZE, Blob, ControlInfo, Engine, Library, Location, MAX_BLOCK, MAX_CHANNELS, RATES, ValueKind};
pub use oem::{Mode, ModeTuning, OemConfig, Output};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot load the DTS engine: {0}")]
    Load(String),
    #[error("DTS engine: {0}")]
    Engine(String),
    #[error("the DTS engine has no control {0:?}")]
    UnknownControl(String),
    #[error("the DTS engine rejected {0} = {1:?}")]
    Control(String, String),
    #[error("tuning blob has {0} bytes, expected {BLOB_SIZE}")]
    BlobSize(usize),
    #[error("OEM configuration: {0}")]
    Config(String),
}

/// Create an engine for `loc`, initialise it and apply a mode's factory settings, as the Windows DTS software
/// does when a mode is selected.
///
/// OEM files contain some settings this engine version does not know or accepts only for other rates; like
/// Windows, those are skipped. They are returned alongside the engine.
pub fn configured_engine(
    lib: &'static Library,
    tuning: &ModeTuning,
    loc: Location,
    channels: usize,
    rate: u32,
) -> Result<(Engine, Vec<Error>), Error> {
    let mut e = Engine::new(lib, loc)?;
    e.init(channels, rate)?;
    let mut skipped = Vec::new();
    for (name, value) in tuning.engine_settings(loc) {
        if let Err(err) = e.set(&name, &value) {
            log::debug!("{} {}: {err}", loc.as_str(), tuning.mode);
            skipped.push(err);
        }
    }
    Ok((e, skipped))
}

/// Create an engine for `loc` and load its complete state from a tuning blob.
pub fn engine_from_blob(
    lib: &'static Library,
    blob: &Blob,
    loc: Location,
    channels: usize,
    rate: u32,
) -> Result<Engine, Error> {
    let mut e = Engine::new(lib, loc)?;
    e.init(channels, rate)?;
    e.set_tuning_blob(blob)?;
    Ok(e)
}
