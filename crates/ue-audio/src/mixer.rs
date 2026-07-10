//! Pure mixer: given a set of items and a timeline frame, produces
//! the mixed stereo sample. No IO, no devices: 100% testable.

use ue_core::keyframe::KeyframeCurve;

use crate::frames_to_us;
use crate::wav::WavMap;

pub struct MixItem {
    pub wav: WavMap,
    /// The clip's position on the timeline, in 48 kHz frames.
    pub timeline_start: i64,
    /// Offset within the WAV where the clip begins (frames).
    pub src_in: i64,
    /// Clip duration in frames (TIMELINE, already divided by speed).
    pub len: i64,
    /// Clip speed: the source is read at this rate. With a `stretcher` the
    /// pitch is preserved live (WSOLA), matching the export's atempo.
    pub speed: f64,
    /// Static linear gain (clip's const gain_db + track volume).
    pub gain: f32,
    /// Gain curve in dB, times relative to the clip start (timeline µs).
    /// If present, it multiplies over `gain` frame by frame.
    pub gain_curve: Option<KeyframeCurve>,
    /// Pan -1 (full left) .. 1 (full right): attenuates the opposite
    /// channel without touching its own (center = unity).
    pub pan: f32,
    /// WSOLA stretcher when speed ≠ 1: live playback keeps the voice pitch.
    /// Only the audio thread touches it (uncontended Mutex).
    pub stretcher: Option<std::sync::Mutex<crate::stretch::Wsola>>,
    pub fade_in: i64,
    pub fade_out: i64,
}

pub fn db_to_linear(db: f64) -> f32 {
    10f64.powf(db / 20.0) as f32
}

/// Pan law: (left_gain, right_gain) for pan in [-1, 1].
#[inline]
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    ((1.0 - p).min(1.0), (1.0 + p).min(1.0))
}

impl MixItem {
    #[inline]
    fn factor_at(&self, rel: i64) -> f32 {
        let mut g = self.gain;
        if let Some(curve) = &self.gain_curve {
            g *= db_to_linear(curve.eval(frames_to_us(rel)));
        }
        if self.fade_in > 0 && rel < self.fade_in {
            g *= rel as f32 / self.fade_in as f32;
        }
        if self.fade_out > 0 {
            let from_end = self.len - rel;
            if from_end < self.fade_out {
                g *= (from_end.max(0)) as f32 / self.fade_out as f32;
            }
        }
        g
    }
}

/// Mixes the sample at timeline frame `pos`. Hard clamp to [-1, 1]
/// (soft limiter: backlog).
#[inline]
pub fn mix_frame(items: &[MixItem], pos: i64) -> (f32, f32) {
    let mut acc = (0.0f32, 0.0f32);
    for item in items {
        let rel = pos - item.timeline_start;
        if rel < 0 || rel >= item.len {
            continue;
        }
        let (l, r) = if let Some(st) = &item.stretcher {
            // pitch-preserved time stretch (same idea as atempo on export)
            st.lock().unwrap().frame_at(&item.wav, item.src_in, rel)
        } else if (item.speed - 1.0).abs() > 1e-9 {
            // fallback: plain resample (pitch shifts)
            item.wav.frame(item.src_in + (rel as f64 * item.speed).round() as i64)
        } else {
            item.wav.frame(item.src_in + rel)
        };
        let g = item.factor_at(rel);
        let (pl, pr) = pan_gains(item.pan);
        acc.0 += l * g * pl;
        acc.1 += r * g * pr;
    }
    (acc.0.clamp(-1.0, 1.0), acc.1.clamp(-1.0, 1.0))
}

/// Fills a contiguous interleaved stereo buffer starting at `pos`.
pub fn fill(items: &[MixItem], pos: i64, out: &mut [f32]) {
    for (i, chunk) in out.chunks_exact_mut(2).enumerate() {
        let (l, r) = mix_frame(items, pos + i as i64);
        chunk[0] = l;
        chunk[1] = r;
    }
}
