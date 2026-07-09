//! Lector mmap de WAV PCM s16le estéreo (el formato que produce el conformado).

use std::fs::File;
use std::path::Path;

use memmap2::Mmap;

use crate::{AudioError, AudioResult};

pub struct WavMap {
    map: Mmap,
    data_offset: usize,
    frames: i64,
    channels: u16,
    pub sample_rate: u32,
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

impl WavMap {
    pub fn open(path: &Path) -> AudioResult<WavMap> {
        let name = path.display().to_string();
        let file = File::open(path)?;
        let map = unsafe { Mmap::map(&file)? };
        let b = &map[..];
        if b.len() < 44 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
            return Err(AudioError::BadWav(name, "cabecera RIFF/WAVE".into()));
        }
        let mut off = 12usize;
        let mut fmt: Option<(u16, u16, u32, u16)> = None; // format, channels, rate, bits
        let mut data: Option<(usize, usize)> = None;
        while off + 8 <= b.len() {
            let id = &b[off..off + 4];
            let size = u32le(b, off + 4) as usize;
            let body = off + 8;
            if body + size > b.len() {
                break;
            }
            match id {
                b"fmt " if size >= 16 => {
                    fmt = Some((
                        u16le(b, body),
                        u16le(b, body + 2),
                        u32le(b, body + 4),
                        u16le(b, body + 14),
                    ));
                }
                b"data" => data = Some((body, size)),
                _ => {}
            }
            off = body + size + (size & 1); // padding a par
        }
        let (format, channels, sample_rate, bits) =
            fmt.ok_or_else(|| AudioError::BadWav(name.clone(), "sin chunk fmt".into()))?;
        let (data_offset, data_len) =
            data.ok_or_else(|| AudioError::BadWav(name.clone(), "sin chunk data".into()))?;
        if format != 1 || bits != 16 {
            return Err(AudioError::BadWav(name, format!("se espera PCM s16, fue fmt={format} bits={bits}")));
        }
        if channels == 0 || channels > 2 {
            return Err(AudioError::BadWav(name, format!("canales no soportados: {channels}")));
        }
        let frames = (data_len / (2 * channels as usize)) as i64;
        Ok(WavMap { map, data_offset, frames, channels, sample_rate })
    }

    pub fn frames(&self) -> i64 {
        self.frames
    }

    /// Frame estéreo en f32 [-1, 1]. Fuera de rango → silencio.
    /// Mono se duplica a ambos canales.
    #[inline]
    pub fn frame(&self, idx: i64) -> (f32, f32) {
        if idx < 0 || idx >= self.frames {
            return (0.0, 0.0);
        }
        let ch = self.channels as usize;
        let base = self.data_offset + idx as usize * 2 * ch;
        let b = &self.map[..];
        let s = |o: usize| i16::from_le_bytes([b[o], b[o + 1]]) as f32 / 32768.0;
        let l = s(base);
        let r = if ch == 2 { s(base + 2) } else { l };
        (l, r)
    }
}
