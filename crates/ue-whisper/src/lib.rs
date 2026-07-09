//! ue-whisper: transcripción word-level con whisper.cpp (PLAN §7.B).
//!
//! Receta word-level de whisper.cpp: `token_timestamps + split_on_word +
//! max_len=1` → un segmento por palabra. La entrada es el WAV conformado
//! (48 kHz estéreo): se hace downmix y decimación exacta ×3 a 16 kHz mono.

use std::path::{Path, PathBuf};

use thiserror::Error;
use ue_core::model::{Segment, TranscriptDoc, Word};
use ue_core::TimeUs;
use ulid::Ulid;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[derive(Debug, Error)]
pub enum WhisperError {
    #[error("audio: {0}")]
    Audio(String),
    #[error("whisper: {0}")]
    Whisper(String),
    #[error("modelo no encontrado en {0}; descárgalo primero (ensure_model)")]
    NoModel(PathBuf),
    #[error("descarga del modelo falló: {0}")]
    Download(String),
}

pub type WhisperResult<T> = Result<T, WhisperError>;

/// URL oficial de los modelos ggml de whisper.cpp.
pub fn model_url(name: &str) -> String {
    format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{name}.bin")
}

pub fn model_path(models_dir: &Path, name: &str) -> PathBuf {
    models_dir.join(format!("ggml-{name}.bin"))
}

/// Descarga el modelo si no existe (curl del sistema; atómico via .part).
pub fn ensure_model(models_dir: &Path, name: &str) -> WhisperResult<PathBuf> {
    let target = model_path(models_dir, name);
    if target.exists() {
        return Ok(target);
    }
    std::fs::create_dir_all(models_dir).map_err(|e| WhisperError::Download(e.to_string()))?;
    let part = target.with_extension("bin.part");
    let status = std::process::Command::new("curl")
        .args(["-L", "--fail", "--silent", "--show-error", "-o"])
        .arg(&part)
        .arg(model_url(name))
        .status()
        .map_err(|e| WhisperError::Download(e.to_string()))?;
    if !status.success() {
        let _ = std::fs::remove_file(&part);
        return Err(WhisperError::Download(format!("curl terminó con {status}")));
    }
    std::fs::rename(&part, &target).map_err(|e| WhisperError::Download(e.to_string()))?;
    Ok(target)
}

/// WAV conformado (48 kHz) → muestras f32 16 kHz mono (decimación ×3 con promedio).
pub fn wav_to_16k_mono(wav_path: &Path) -> WhisperResult<Vec<f32>> {
    let wav = ue_audio::wav::WavMap::open(wav_path).map_err(|e| WhisperError::Audio(e.to_string()))?;
    if wav.sample_rate != 48_000 {
        return Err(WhisperError::Audio(format!(
            "se espera WAV conformado a 48 kHz, fue {}",
            wav.sample_rate
        )));
    }
    let frames = wav.frames();
    let mut out = Vec::with_capacity((frames / 3) as usize);
    let mut i = 0i64;
    while i + 2 < frames {
        let mut acc = 0.0f32;
        for k in 0..3 {
            let (l, r) = wav.frame(i + k);
            acc += 0.5 * (l + r);
        }
        out.push(acc / 3.0);
        i += 3;
    }
    Ok(out)
}

/// Transcribe y devuelve un TranscriptDoc word-level (tiempos en µs del asset).
pub fn transcribe(
    conform_wav: &Path,
    model: &Path,
    language: Option<&str>,
    asset_id: ue_core::model::Id,
) -> WhisperResult<TranscriptDoc> {
    if !model.exists() {
        return Err(WhisperError::NoModel(model.to_path_buf()));
    }
    let samples = wav_to_16k_mono(conform_wav)?;

    let ctx = WhisperContext::new_with_params(
        &model.to_string_lossy(),
        WhisperContextParameters::default(),
    )
    .map_err(|e| WhisperError::Whisper(e.to_string()))?;
    let mut state = ctx.create_state().map_err(|e| WhisperError::Whisper(e.to_string()))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(language.or(Some("auto")));
    params.set_token_timestamps(true);
    params.set_split_on_word(true);
    params.set_max_len(1); // un segmento ≈ una palabra
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_realtime(false);
    params.set_suppress_blank(true);

    state
        .full(params, &samples)
        .map_err(|e| WhisperError::Whisper(e.to_string()))?;

    let n = state.full_n_segments().map_err(|e| WhisperError::Whisper(e.to_string()))?;
    let mut words: Vec<Word> = vec![];
    for i in 0..n {
        let text = state
            .full_get_segment_text(i)
            .map_err(|e| WhisperError::Whisper(e.to_string()))?;
        let text = text.trim().to_string();
        if text.is_empty() || text.starts_with('[') {
            continue; // [_BEG_], [BLANK_AUDIO], etc.
        }
        // t0/t1 en centisegundos
        let t0 = state.full_get_segment_t0(i).map_err(|e| WhisperError::Whisper(e.to_string()))?;
        let t1 = state.full_get_segment_t1(i).map_err(|e| WhisperError::Whisper(e.to_string()))?;
        words.push(Word {
            text,
            start_us: t0 as TimeUs * 10_000,
            end_us: t1 as TimeUs * 10_000,
            confidence: 0.0,
            rejected: false,
        });
    }

    // Agrupar palabras en frases por pausas > 1 s (port de process_transcript
    // del toolkit) para los segmentos.
    let mut segments: Vec<Segment> = vec![];
    let mut seg_start_idx = 0usize;
    for i in 0..words.len() {
        let is_last = i + 1 == words.len();
        let pause_after = if is_last {
            i64::MAX
        } else {
            words[i + 1].start_us - words[i].end_us
        };
        if pause_after > 1_000_000 || is_last {
            let text = words[seg_start_idx..=i]
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            segments.push(Segment {
                text,
                start_us: words[seg_start_idx].start_us,
                end_us: words[i].end_us,
                word_range: (seg_start_idx, i + 1),
                emotion: None,
                volume_rms: 0.0,
            });
            seg_start_idx = i + 1;
        }
    }

    Ok(TranscriptDoc {
        id: Ulid::new(),
        asset_id,
        language: language.unwrap_or("auto").to_string(),
        model: model
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        words,
        segments,
        global_avg_volume: 0.0,
    })
}
