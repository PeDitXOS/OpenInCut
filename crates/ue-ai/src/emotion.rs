//! Per-segment emotion analysis for the avatar (PLAN §7.E.1).
//! Port of Youtubers-toolkit's avatar_video_generation.py:
//! - RMS volume per segment (over the conformed WAV).
//! - Classification: offline heuristic by energy/speed, or optionally
//!   an OpenAI-compatible API (same prompt as the toolkit) via curl.

use std::collections::BTreeMap;

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

/// The toolkit's prompt (build_emotion_system_prompt).
pub fn emotion_system_prompt(labels: &[String]) -> String {
    let described: Vec<(String, String)> =
        labels.iter().map(|l| (l.clone(), String::new())).collect();
    emotion_system_prompt_described(&described)
}

/// Same prompt, enriched with each label's user-written description
/// ("angry: furious, upset, complaining"), which greatly improves matching.
pub fn emotion_system_prompt_described(labels: &[(String, String)]) -> String {
    let list = labels
        .iter()
        .map(|(name, desc)| {
            if desc.trim().is_empty() {
                name.clone()
            } else {
                format!("{name} ({})", desc.trim())
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let names =
        labels.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ");
    format!(
        "You are an emotion classifier. Given a short phrase in any language, \
         reply with exactly one of the following labels: {list}. \
         Respond with just the label name (one of: {names}), no extra text. \
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
    let described: Vec<(String, String)> =
        available.iter().map(|l| (l.clone(), String::new())).collect();
    classify_via_api_described(api_base, api_key, model, text, &described)
}

/// Classifier using per-label descriptions (avatar expressions).
pub fn classify_via_api_described(
    api_base: &str,
    api_key: &str,
    model: &str,
    text: &str,
    described: &[(String, String)],
) -> Option<String> {
    let available: Vec<String> = described.iter().map(|(n, _)| n.clone()).collect();
    let body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": emotion_system_prompt_described(described) },
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

/// API classifier config, read from the environment (like the toolkit's .env).
pub struct ApiConfig {
    pub base: String,
    pub key: String,
    pub model: String,
}

impl ApiConfig {
    pub fn from_env() -> Option<ApiConfig> {
        let key = std::env::var("OPENAI_API_KEY").ok().filter(|k| !k.is_empty())?;
        Some(ApiConfig {
            base: std::env::var("OPENAI_API_BASE")
                .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
            key,
            model: std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into()),
        })
    }
}

/// Classifies all segments of a transcript (API if available, otherwise heuristic)
/// and writes `segments[].emotion`. `avatars` = available emotion→path map.
pub fn classify_segments(
    doc: &mut TranscriptDoc,
    avatars: &BTreeMap<String, String>,
    api: Option<&ApiConfig>,
) {
    let labels: Vec<String> = avatars.keys().cloned().collect();
    if labels.is_empty() {
        return;
    }
    let avg = doc.global_avg_volume;
    for i in 0..doc.segments.len() {
        let seg = &doc.segments[i];
        let dur_s = ((seg.end_us - seg.start_us) as f64 / 1e6).max(0.2);
        let words = (seg.word_range.1 - seg.word_range.0) as f64;
        let from_api = api.and_then(|c| {
            classify_via_api(&c.base, &c.key, &c.model, &seg.text, &labels)
        });
        let emotion = from_api.unwrap_or_else(|| {
            classify_heuristic(seg.volume_rms, avg, words / dur_s, &labels)
        });
        doc.segments[i].emotion = Some(emotion);
    }
}
