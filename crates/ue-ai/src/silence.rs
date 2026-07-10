//! Silence detection over the conformed WAV (48 kHz stereo).
//!
//! Algorithm (PLAN §7.C):
//! 1. RMS over `window_us` windows with `hop_us` step → dBFS.
//! 2. State machine with a dual threshold (hysteresis): speech if it exceeds
//!    `threshold_db`; silence only if it falls below `threshold_db - hysteresis_db`.
//! 3. Silences shorter than `min_silence_us` are absorbed (breaths);
//!    speech islands shorter than `min_speech_us` are dropped (clicks).
//! 4. Speech is expanded by `pad_pre/post_us` and overlaps are merged.

use serde::{Deserialize, Serialize};
use ue_audio::wav::WavMap;
use ue_core::TimeUs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SilenceParams {
    /// Speech threshold in dBFS (the toolkit's linear 0.01 ≈ -40 dBFS).
    pub threshold_db: f64,
    pub hysteresis_db: f64,
    pub min_silence_us: TimeUs,
    pub min_speech_us: TimeUs,
    pub pad_pre_us: TimeUs,
    pub pad_post_us: TimeUs,
}

impl Default for SilenceParams {
    fn default() -> Self {
        Self {
            threshold_db: -38.0,
            hysteresis_db: 6.0,
            min_silence_us: 400_000,
            min_speech_us: 150_000,
            pad_pre_us: 150_000,
            pad_post_us: 200_000,
        }
    }
}

const WINDOW_US: TimeUs = 50_000;
const HOP_US: TimeUs = 10_000;

/// SPEECH intervals `[start, end)` in µs within the WAV's `[from, to)` range.
pub fn detect_speech(
    wav: &WavMap,
    from_us: TimeUs,
    to_us: TimeUs,
    params: &SilenceParams,
) -> Vec<(TimeUs, TimeUs)> {
    let rate = wav.sample_rate as i64;
    let to_us = to_us.min(wav.frames() * 1_000_000 / rate);
    if to_us <= from_us {
        return vec![];
    }
    let win_frames = (WINDOW_US * rate / 1_000_000) as usize;

    // 1. RMS in dB per window
    let mut windows: Vec<(TimeUs, f64)> = vec![]; // (center, dBFS)
    let mut t = from_us;
    while t + WINDOW_US <= to_us {
        let start_frame = t * rate / 1_000_000;
        let mut acc = 0.0f64;
        for k in 0..win_frames {
            let (l, r) = wav.frame(start_frame + k as i64);
            let mono = 0.5 * (l + r) as f64;
            acc += mono * mono;
        }
        let rms = (acc / win_frames as f64).sqrt();
        let db = if rms > 1e-9 { 20.0 * rms.log10() } else { -120.0 };
        windows.push((t + WINDOW_US / 2, db));
        t += HOP_US;
    }
    if windows.is_empty() {
        return vec![];
    }

    // 2. hysteresis
    let t_on = params.threshold_db;
    let t_off = params.threshold_db - params.hysteresis_db;
    let mut speech_flags: Vec<bool> = Vec::with_capacity(windows.len());
    let mut talking = windows[0].1 > t_on;
    for &(_, db) in &windows {
        if talking {
            if db < t_off {
                talking = false;
            }
        } else if db > t_on {
            talking = true;
        }
        speech_flags.push(talking);
    }

    // 3. raw spans
    let mut spans: Vec<(TimeUs, TimeUs, bool)> = vec![];
    for (i, &flag) in speech_flags.iter().enumerate() {
        let time = windows[i].0;
        match spans.last_mut() {
            Some((_, end, f)) if *f == flag => *end = time,
            _ => spans.push((time, time, flag)),
        }
    }

    // short silences → speech
    let merged: Vec<(TimeUs, TimeUs, bool)> = {
        let mut out: Vec<(TimeUs, TimeUs, bool)> = vec![];
        for (s, e, f) in spans {
            let keep_as_speech = !f && (e - s) < params.min_silence_us;
            let f = f || keep_as_speech;
            match out.last_mut() {
                Some((_, oe, of)) if *of == f => *oe = e,
                _ => out.push((s, e, f)),
            }
        }
        out
    };

    // short speech islands → out; padding + clamp + merge
    let mut speech: Vec<(TimeUs, TimeUs)> = vec![];
    for (s, e, f) in merged {
        if !f || (e - s) < params.min_speech_us {
            continue;
        }
        let s = (s - params.pad_pre_us).max(from_us);
        let e = (e + params.pad_post_us).min(to_us);
        match speech.last_mut() {
            Some((_, pe)) if s <= *pe => *pe = (*pe).max(e),
            _ => speech.push((s, e)),
        }
    }
    speech
}

/// Complement: SILENCE intervals within `[from, to)`.
pub fn detect_silences(
    wav: &WavMap,
    from_us: TimeUs,
    to_us: TimeUs,
    params: &SilenceParams,
) -> Vec<(TimeUs, TimeUs)> {
    let rate = wav.sample_rate as i64;
    let to_us = to_us.min(wav.frames() * 1_000_000 / rate);
    let speech = detect_speech(wav, from_us, to_us, params);
    let mut out = vec![];
    let mut cursor = from_us;
    for (s, e) in &speech {
        if *s > cursor {
            out.push((cursor, *s));
        }
        cursor = *e;
    }
    if cursor < to_us {
        out.push((cursor, to_us));
    }
    out
}

/// A clip's silences mapped to TIMELINE TIME, ready for cut_ranges.
pub fn clip_silences_on_timeline(
    wav: &WavMap,
    clip_start_us: TimeUs,
    src_in_us: TimeUs,
    src_out_us: TimeUs,
    params: &SilenceParams,
) -> Vec<(TimeUs, TimeUs)> {
    detect_silences(wav, src_in_us, src_out_us, params)
        .into_iter()
        .map(|(s, e)| (clip_start_us + (s - src_in_us), clip_start_us + (e - src_in_us)))
        .collect()
}
