//! Progress and cancellation, against a real model and real audio.
//!
//! Run with:
//!   UE_WAV=<48k wav> UE_MODEL=<ggml model> \
//!   cargo test -p ue-whisper --test progress_and_cancel -- --ignored --nocapture

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

fn fixtures() -> Option<(PathBuf, PathBuf)> {
    Some((
        PathBuf::from(std::env::var("UE_WAV").ok()?),
        PathBuf::from(std::env::var("UE_MODEL").ok()?),
    ))
}

/// A job used to sit at 0.0 until it finished, so "slow" and "hung" looked
/// identical from the outside. Whisper reports its own percentage; we must see
/// values BETWEEN the ends, not just 0 and 1.
#[test]
#[ignore = "needs UE_WAV and UE_MODEL"]
fn progress_reports_intermediate_values() {
    let Some((wav, model)) = fixtures() else { return };
    let seen = Arc::new(Mutex::new(Vec::<f64>::new()));
    let sink = seen.clone();
    let doc = ue_whisper::transcribe_with(
        &wav,
        &model,
        Some("es"),
        ue_core::model::Id::new(),
        move |p| sink.lock().unwrap().push(p),
        &AtomicBool::new(false),
    )
    .expect("transcription must still succeed with the callbacks attached");

    let seen = seen.lock().unwrap();
    let middle: Vec<f64> = seen.iter().copied().filter(|p| *p > 0.0 && *p < 1.0).collect();
    eprintln!(
        "{} progress reports, {} of them intermediate (first few: {:?}), {} words",
        seen.len(),
        middle.len(),
        &middle.iter().take(6).collect::<Vec<_>>(),
        doc.words.len()
    );
    assert!(!doc.words.is_empty(), "it actually transcribed something");
    assert!(
        middle.len() >= 3,
        "progress moved through the middle, not just 0 → 1 ({} reports)",
        seen.len()
    );
    assert!(
        seen.windows(2).all(|w| w[1] >= w[0]),
        "progress never goes backwards"
    );
}

/// The only way out of a long transcription used to be killing the whole app.
#[test]
#[ignore = "needs UE_WAV and UE_MODEL"]
fn a_transcription_can_be_cancelled() {
    let Some((wav, model)) = fixtures() else { return };
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    // let it get going, then pull the plug
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        flag.store(true, Ordering::SeqCst);
    });

    let t0 = std::time::Instant::now();
    let r = ue_whisper::transcribe_with(
        &wav,
        &model,
        Some("es"),
        ue_core::model::Id::new(),
        |_| {},
        &cancel,
    );
    let secs = t0.elapsed().as_secs_f64();
    eprintln!("cancelled after {secs:.1}s → {r:?}");
    assert!(
        matches!(r, Err(ue_whisper::WhisperError::Cancelled)),
        "cancelling reports Cancelled, not a bogus success or a fake crash"
    );
    // it really cut the work short (the full run of this fixture takes ~25 s)
    assert!(secs < 22.0, "it stopped early, not after finishing anyway ({secs:.1}s)");
}
