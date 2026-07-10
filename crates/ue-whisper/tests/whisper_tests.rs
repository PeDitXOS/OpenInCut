//! Real transcription test: speech generated with `say` (macOS) + tiny model.
//! Skipped with a warning if `say`, `curl`, ffmpeg or the model download is missing.

use std::path::PathBuf;
use std::process::Command;

use ue_whisper::{ensure_model, transcribe, wav_to_16k_mono};

fn tool_ok(cmd: &str, arg: &str) -> bool {
    Command::new(cmd).arg(arg).output().map(|o| o.status.success()).unwrap_or(false)
}

#[test]
fn transcribes_real_speech_word_level() {
    if !tool_ok("say", "-v?") || !tool_ok(&ue_media::ffmpeg_bin(), "-version") {
        eprintln!("WARNING: no `say`/ffmpeg; whisper test skipped");
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ue-whisper-tests");
    std::fs::create_dir_all(&dir).unwrap();

    // tiny model (~75 MB, cached between runs)
    let models = dir.join("models");
    let model = match ensure_model(&models, "tiny.en") {
        Ok(m) => m,
        Err(e) => {
            eprintln!("WARNING: couldn't download the model ({e}); test skipped");
            return;
        }
    };

    // synthetic speech: "hello world this is a test"
    let aiff = dir.join("voice.aiff");
    let st = Command::new("say")
        .args(["-o"])
        .arg(&aiff)
        .args(["hello world this is a test"])
        .status()
        .unwrap();
    assert!(st.success());
    // conform to 48k stereo (the pipeline format)
    let wav = dir.join("voice48.wav");
    ue_media::conform_audio(&aiff, &wav).unwrap();

    // correct decimation: ~duration * 16000 samples
    let samples = wav_to_16k_mono(&wav).unwrap();
    assert!(samples.len() > 16_000, "at least 1 s of audio: {}", samples.len());

    let doc = transcribe(&wav, &model, Some("en"), ue_core::model::Id::new()).unwrap();
    assert!(!doc.words.is_empty(), "there are words");
    let joined = doc
        .words
        .iter()
        .map(|w| w.text.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!("transcript: {joined}");
    assert!(joined.contains("hello"), "recognizes 'hello': {joined}");
    assert!(joined.contains("test"), "recognizes 'test': {joined}");

    // increasing timestamps within the audio
    for w in doc.words.windows(2) {
        assert!(w[0].start_us <= w[1].start_us, "words ordered");
    }
    let total_us = (samples.len() as i64) * 1_000_000 / 16_000;
    assert!(doc.words.last().unwrap().end_us <= total_us + 1_500_000);
    assert!(!doc.segments.is_empty(), "there are grouped phrases");
}
