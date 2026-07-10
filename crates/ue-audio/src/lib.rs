//! ue-audio: timeline mixing and playback.
//!
//! Design (PLAN §5.4):
//! - All audio is conformed on import to WAV PCM s16le 48 kHz stereo (ue-media).
//! - `WavMap` reads those WAVs via mmap with random per-frame access.
//! - `MixItem`/`mix_frame` is the pure mixer (tested with synthetic signals):
//!   per-clip gain + track volume, linear fades, sum and clamp.
//! - `Player` is the cpal output: AUDIO is the master clock; the position is
//!   derived from the frames served to the device (with rate conversion).

pub mod items;
pub mod mixer;
pub mod player;
pub mod wav;

use thiserror::Error;

/// The project's internal rate (conforming guarantees it).
pub const RATE: u32 = 48_000;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid WAV ({0}): {1}")]
    BadWav(String, String),
    #[error("no audio output device")]
    NoDevice,
    #[error("cpal: {0}")]
    Cpal(String),
}

pub type AudioResult<T> = Result<T, AudioError>;

/// µs (48k frames) ↔ audio frames.
pub fn us_to_frames(us: i64) -> i64 {
    us * RATE as i64 / 1_000_000
}
pub fn frames_to_us(frames: i64) -> i64 {
    frames * 1_000_000 / RATE as i64
}
