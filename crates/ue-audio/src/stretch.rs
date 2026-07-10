//! WSOLA time-stretcher: plays a source at a different speed WITHOUT changing
//! the pitch (the live counterpart of the export's `atempo`).
//!
//! Classic waveform-similarity overlap-add: Hann windows at 50% overlap laid
//! down at the output rate while the source read position advances at
//! `speed ×` rate; each window is nudged (±SEARCH frames) to the offset that
//! best continues the previous one, which removes the phasing/garbling of a
//! naive overlap-add. Good speech quality for the 0.25×–4× range we expose.
//!
//! The stretcher is sequential by design; the mixer feeds it monotonically
//! increasing output positions and it resets itself on any jump (seek).

use std::collections::VecDeque;

use crate::wav::WavMap;

/// ~21 ms at 48 kHz.
const WIN: usize = 1024;
/// 50% overlap: periodic Hann windows sum to 1 (COLA).
const HOP: usize = WIN / 2;
/// Alignment search range in frames (±6 ms).
const SEARCH: i64 = 288;
/// Correlation template length.
const CMP: usize = 256;

pub struct Wsola {
    speed: f64,
    /// Next output frame we expect to be asked for (clip-relative).
    next_out: i64,
    /// Nominal source position of the next window (clip-relative frames).
    nominal: f64,
    /// Actual source position of the previous window (continuity template).
    prev_src: i64,
    /// Second half of the previous Hann window, pending overlap-add.
    tail: Vec<(f32, f32)>,
    ready: VecDeque<(f32, f32)>,
    window: Vec<f32>,
}

impl Wsola {
    pub fn new(speed: f64) -> Self {
        let window = (0..WIN)
            .map(|i| {
                let x = std::f32::consts::PI * i as f32 / WIN as f32;
                x.sin() * x.sin() // periodic Hann
            })
            .collect();
        let mut w = Wsola {
            speed: speed.clamp(0.05, 20.0),
            next_out: -1,
            nominal: 0.0,
            prev_src: 0,
            tail: vec![(0.0, 0.0); HOP],
            ready: VecDeque::with_capacity(HOP * 2),
            window,
        };
        w.reset(0);
        w
    }

    fn reset(&mut self, out_frame: i64) {
        self.next_out = out_frame;
        self.nominal = out_frame as f64 * self.speed;
        self.prev_src = self.nominal as i64 - HOP as i64;
        // zeroed tail = a 10 ms fade-in right after a seek; inaudible
        self.tail.iter_mut().for_each(|t| *t = (0.0, 0.0));
        self.ready.clear();
    }

    #[inline]
    fn read(wav: &WavMap, src_in: i64, pos: i64) -> (f32, f32) {
        wav.frame(src_in + pos)
    }

    /// Offset (±SEARCH) whose window best continues the previous one.
    fn best_offset(&self, wav: &WavMap, src_in: i64) -> i64 {
        let base = self.prev_src + HOP as i64;
        let nominal = self.nominal as i64;
        let mut tmpl = [0f32; CMP];
        for (k, t) in tmpl.iter_mut().enumerate() {
            let (l, r) = Self::read(wav, src_in, base + k as i64);
            *t = l + r;
        }
        let (mut best, mut best_score) = (0i64, f32::MIN);
        let mut d = -SEARCH;
        while d <= SEARCH {
            let mut score = 0f32;
            let mut k = 0;
            while k < CMP {
                let (l, r) = Self::read(wav, src_in, nominal + d + k as i64);
                score += tmpl[k] * (l + r);
                k += 4;
            }
            if score > best_score {
                best_score = score;
                best = d;
            }
            d += 4;
        }
        best
    }

    /// Produce the next HOP output frames into `ready`.
    fn generate(&mut self, wav: &WavMap, src_in: i64) {
        let d = if (self.speed - 1.0).abs() < 1e-6 { 0 } else { self.best_offset(wav, src_in) };
        let s = self.nominal as i64 + d;
        for k in 0..HOP {
            let (l, r) = Self::read(wav, src_in, s + k as i64);
            let w = self.window[k];
            let (tl, tr) = self.tail[k];
            self.ready.push_back((tl + l * w, tr + r * w));
        }
        for k in 0..HOP {
            let (l, r) = Self::read(wav, src_in, s + (HOP + k) as i64);
            let w = self.window[HOP + k];
            self.tail[k] = (l * w, r * w);
        }
        self.prev_src = s;
        self.nominal += HOP as f64 * self.speed;
    }

    /// Pitch-preserved sample for clip-relative OUTPUT frame `rel`.
    /// Sequential: any jump in `rel` resets the stretcher (seek).
    pub fn frame_at(&mut self, wav: &WavMap, src_in: i64, rel: i64) -> (f32, f32) {
        if rel != self.next_out {
            self.reset(rel);
        }
        while self.ready.is_empty() {
            self.generate(wav, src_in);
        }
        self.next_out += 1;
        self.ready.pop_front().unwrap_or((0.0, 0.0))
    }
}
