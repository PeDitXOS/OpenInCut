//! Per-segment emotion analysis for the avatar (PLAN §7.E.1).
//! Port of Youtubers-toolkit's avatar_video_generation.py:
//! - RMS volume per segment (over the conformed WAV).
//! - Classification: offline heuristic by energy/speed, or optionally
//!   an OpenAI-compatible API (same prompt as the toolkit) via curl.

use ue_audio::wav::WavMap;
use ue_core::model::TranscriptDoc;

/// Linear RMS of a WAV range (mixed to mono).
fn range_rms(wav: &WavMap, from_us: i64, to_us: i64) -> f64 {
    let rate = wav.sample_rate as i64;
    let from = from_us * rate / 1_000_000;
    let to = (to_us * rate / 1_000_000).min(wav.frames());
    if to <= from {
        return 0.0;
    }
    // stepped sampling for long segments (enough for an average)
    let step = (((to - from) / 4800).max(1)) as usize;
    let mut acc = 0.0f64;
    let mut n = 0u64;
    let mut i = from;
    while i < to {
        let (l, r) = wav.frame(i);
        let mono = 0.5 * (l + r) as f64;
        acc += mono * mono;
        n += 1;
        i += step as i64;
    }
    if n == 0 { 0.0 } else { (acc / n as f64).sqrt() }
}

/// Fills in a transcript's volume_rms and global_avg_volume.
pub fn measure_volumes(doc: &mut TranscriptDoc, wav: &WavMap) {
    let mut sum = 0.0;
    for seg in &mut doc.segments {
        seg.volume_rms = range_rms(wav, seg.start_us, seg.end_us);
        sum += seg.volume_rms;
    }
    doc.global_avg_volume = if doc.segments.is_empty() {
        0.0
    } else {
        sum / doc.segments.len() as f64
    };
}

/// The toolkit's prompt (build_emotion_system_prompt). The expression NAMES
/// are the whole contract, exactly like the toolkit's `{ emotion: path }`.
pub fn emotion_system_prompt(labels: &[String]) -> String {
    let names = labels.join(", ");
    format!(
        "You are an emotion classifier. Given a short phrase in any language, \
         reply with exactly one of the following labels: {names}. \
         Respond with just the label name, no extra text. \
         Try to be expressive."
    )
}

/// Offline heuristic: relative energy + speech rate → label.
/// Only uses the emotions AVAILABLE in the map (loose matching like the toolkit).
pub fn classify_heuristic(
    seg_volume: f64,
    avg_volume: f64,
    words_per_sec: f64,
    available: &[String],
) -> String {
    let pick = |wanted: &[&str]| -> Option<String> {
        for w in wanted {
            if let Some(hit) = available
                .iter()
                .find(|a| a.to_lowercase().contains(&w.to_lowercase()))
            {
                return Some(hit.clone());
            }
        }
        None
    };
    let ratio = if avg_volume > 1e-9 { seg_volume / avg_volume } else { 1.0 };
    let choice = if ratio > 1.4 {
        pick(&["angry", "amazed", "wow", "excited"])
    } else if ratio < 0.6 {
        pick(&["sad", "calm", "suspicious", "sus"])
    } else if words_per_sec > 3.5 {
        pick(&["amazed", "smug", "excited", "wow"])
    } else {
        pick(&["calm", "smug"])
    };
    choice.unwrap_or_else(|| available.first().cloned().unwrap_or_default())
}

/// Classifies via an OpenAI-compatible API (curl). Returns None on failure
/// (no network, no key, weird response) — the caller falls back to the heuristic.
pub fn classify_via_api(
    api_base: &str,
    api_key: &str,
    model: &str,
    text: &str,
    available: &[String],
) -> Option<String> {
    let body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": emotion_system_prompt(available) },
            { "role": "user", "content": text },
        ],
    });
    let out = std::process::Command::new("curl")
        .args(["-s", "--max-time", "20", "-X", "POST"])
        .arg(format!("{}/chat/completions", api_base.trim_end_matches('/')))
        .args(["-H", "Content-Type: application/json"])
        .arg("-H")
        .arg(format!("Authorization: Bearer {api_key}"))
        .args(["-d", &body.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let raw = v
        .pointer("/choices/0/message/content")?
        .as_str()?
        .trim()
        .to_lowercase();
    // loose substring matching, like the toolkit's classify_emotion()
    available.iter().find(|k| raw.contains(&k.to_lowercase())).cloned()
}

