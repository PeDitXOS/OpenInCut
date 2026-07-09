//! ue-audio: mezcla y reproducción del timeline.
//!
//! Diseño (PLAN §5.4):
//! - Todo audio se conforma al importar a WAV PCM s16le 48 kHz estéreo (ue-media).
//! - `WavMap` lee esos WAV por mmap con acceso aleatorio por frame.
//! - `MixItem`/`mix_frame` es el mezclador puro (testeado con señales sintéticas):
//!   ganancia por clip + volumen de pista, fades lineales, suma y clamp.
//! - `Player` es la salida cpal: el AUDIO es el reloj maestro; la posición se
//!   deriva de los frames servidos al dispositivo (con conversión de tasa).

pub mod items;
pub mod mixer;
pub mod player;
pub mod wav;

use thiserror::Error;

/// Tasa interna del proyecto (el conformado la garantiza).
pub const RATE: u32 = 48_000;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("WAV inválido ({0}): {1}")]
    BadWav(String, String),
    #[error("sin dispositivo de salida de audio")]
    NoDevice,
    #[error("cpal: {0}")]
    Cpal(String),
}

pub type AudioResult<T> = Result<T, AudioError>;

/// µs (48k frames) ↔ frames de audio.
pub fn us_to_frames(us: i64) -> i64 {
    us * RATE as i64 / 1_000_000
}
pub fn frames_to_us(frames: i64) -> i64 {
    frames * 1_000_000 / RATE as i64
}
