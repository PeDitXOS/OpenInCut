//! ue-whisper: word-level transcription with whisper.cpp (PLAN §7.B).
//!
//! whisper.cpp word-level recipe: `token_timestamps + split_on_word +
//! max_len=1` → one segment per word. The input is the conformed WAV
//! (48 kHz stereo): downmix and exact ×3 decimation to 16 kHz mono.

use std::path::{Path, PathBuf};

use thiserror::Error;
use ue_core::model::{Segment, TranscriptDoc, Word};
use ue_core::TimeUs;
use ulid::Ulid;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[derive(Debug, Error)]
pub enum WhisperError {
    #[error("audio: {0}")]
    Audio(String),
    #[error("whisper: {0}")]
    Whisper(String),
    #[error("model not found at {0}; download it first (ensure_model)")]
    NoModel(PathBuf),
    #[error("model download failed: {0}")]
    Download(String),
    #[error("transcription cancelled")]
    Cancelled,
}

pub type WhisperResult<T> = Result<T, WhisperError>;

/// Official URL for the whisper.cpp ggml models.
pub fn model_url(name: &str) -> String {
    format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{name}.bin")
}

pub fn model_path(models_dir: &Path, name: &str) -> PathBuf {
    models_dir.join(format!("ggml-{name}.bin"))
}

/// Downloads the model if it doesn't exist (system curl; atomic via .part).
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
        return Err(WhisperError::Download(format!("curl exited with {status}")));
    }
    std::fs::rename(&part, &target).map_err(|e| WhisperError::Download(e.to_string()))?;
    Ok(target)
}

/// Conformed WAV (48 kHz) → f32 16 kHz mono samples (×3 decimation with averaging).
pub fn wav_to_16k_mono(wav_path: &Path) -> WhisperResult<Vec<f32>> {
    let wav = ue_audio::wav::WavMap::open(wav_path).map_err(|e| WhisperError::Audio(e.to_string()))?;
    if wav.sample_rate != 48_000 {
        return Err(WhisperError::Audio(format!(
            "expected a WAV conformed to 48 kHz, was {}",
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

/// Transcribes and returns a word-level TranscriptDoc (times in µs of the asset).
pub fn transcribe(
    conform_wav: &Path,
    model: &Path,
    language: Option<&str>,
    asset_id: ue_core::model::Id,
) -> WhisperResult<TranscriptDoc> {
    transcribe_with(conform_wav, model, language, asset_id, |_| {}, &AtomicBool::new(false))
}

/// Same, but reporting progress (0..1) and honouring a cancel flag.
///
/// Both were missing, and both hurt: a job sat at 0.0 until it finished, so
/// "slow" and "hung" looked identical from the outside, and the only way to
/// stop a 28-minute transcription was to kill the whole app. whisper.cpp has
/// had the hooks all along — we just never passed them.
pub fn transcribe_with(
    conform_wav: &Path,
    model: &Path,
    language: Option<&str>,
    asset_id: ue_core::model::Id,
    mut on_progress: impl FnMut(f64) + Send,
    cancel: &AtomicBool,
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
    // whisper.cpp reports whole percents; the channel keeps the closure (which
    // must be 'static) away from the borrowed `on_progress`
    let (tx, rx) = std::sync::mpsc::channel::<i32>();
    params.set_progress_callback_safe(move |pct: i32| {
        let _ = tx.send(pct);
    });
    // The abort flag is read by whisper.cpp between compute steps.
    //
    // NOT via `set_abort_callback_safe`: that helper in whisper-rs 0.14.4 boxes
    // the closure into a `Box<Box<dyn FnMut() -> bool>>` but instantiates its
    // trampoline as `trampoline::<F>` with the CONCRETE closure type — so the
    // trampoline casts a pointer-to-fat-pointer as a pointer-to-closure and
    // reads garbage. Whisper then dies with a generic error -6. (Its progress
    // sibling, two functions up, gets this right; the abort one does not.)
    //
    // So we hand whisper a plain thin pointer to the flag itself.
    let flag = Arc::new(AtomicBool::new(false));
    unsafe extern "C" fn abort_trampoline(user_data: *mut std::ffi::c_void) -> bool {
        if user_data.is_null() {
            return false;
        }
        unsafe { (*(user_data as *const AtomicBool)).load(Ordering::SeqCst) }
    }
    let flag_raw = Arc::into_raw(flag.clone());
    unsafe {
        params.set_abort_callback(Some(abort_trampoline));
        params.set_abort_callback_user_data(flag_raw as *mut std::ffi::c_void);
    }
    // reclaims the leaked Arc once whisper can no longer read it
    struct FlagGuard(*const AtomicBool);
    impl Drop for FlagGuard {
        fn drop(&mut self) {
            unsafe { drop(Arc::from_raw(self.0)) };
        }
    }
    let _flag_guard = FlagGuard(flag_raw);
    params.set_language(language.or(Some("auto")));
    params.set_token_timestamps(true);
    params.set_split_on_word(true);
    params.set_max_len(1); // one segment ≈ one word
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_realtime(false);
    params.set_suppress_blank(true);

    // pump progress (and mirror the caller's cancel into the abort flag) while
    // whisper runs on this thread: the callbacks fire from inside `full`
    let result = std::thread::scope(|scope| -> WhisperResult<()> {
        let done = Arc::new(AtomicBool::new(false));
        let done_w = done.clone();
        let flag_w = flag.clone();
        scope.spawn(move || {
            while !done_w.load(Ordering::SeqCst) {
                if cancel.load(Ordering::SeqCst) {
                    flag_w.store(true, Ordering::SeqCst);
                }
                match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(pct) => on_progress((pct as f64 / 100.0).clamp(0.0, 1.0)),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => break,
                }
            }
        });
        let r = state
            .full(params, &samples)
            .map(|_| ())
            .map_err(|e| WhisperError::Whisper(e.to_string()));
        done.store(true, Ordering::SeqCst);
        r
    });
    // An aborted graph makes whisper.cpp return a generic failure (-6), so the
    // cancel flag is what tells the difference between "the user stopped it"
    // and "it broke". Check it BEFORE surfacing the error, or a cancellation
    // reads as a crash.
    if cancel.load(Ordering::SeqCst) {
        return Err(WhisperError::Cancelled);
    }
    result?;

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
        // t0/t1 in centiseconds
        let t0 = state.full_get_segment_t0(i).map_err(|e| WhisperError::Whisper(e.to_string()))?;
        let t1 = state.full_get_segment_t1(i).map_err(|e| WhisperError::Whisper(e.to_string()))?;
        words.push(Word {
            text,
            start_us: t0 as TimeUs * 10_000,
            end_us: t1 as TimeUs * 10_000,
            confidence: 0.0,
            rejected: false,
            display: None,
        });
    }

    // Group words into phrases by pauses > 1 s (port of the toolkit's
    // process_transcript) for the segments.
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
