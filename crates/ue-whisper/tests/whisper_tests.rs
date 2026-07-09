//! Test real de transcripción: voz generada con `say` (macOS) + modelo tiny.
//! Se salta con aviso si falta `say`, `curl`, ffmpeg o la descarga del modelo.

use std::path::PathBuf;
use std::process::Command;

use ue_whisper::{ensure_model, transcribe, wav_to_16k_mono};

fn tool_ok(cmd: &str, arg: &str) -> bool {
    Command::new(cmd).arg(arg).output().map(|o| o.status.success()).unwrap_or(false)
}

#[test]
fn transcribes_real_speech_word_level() {
    if !tool_ok("say", "-v?") || !tool_ok(&ue_media::ffmpeg_bin(), "-version") {
        eprintln!("AVISO: sin `say`/ffmpeg; test de whisper saltado");
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ue-whisper-tests");
    std::fs::create_dir_all(&dir).unwrap();

    // modelo tiny (~75 MB, cacheado entre ejecuciones)
    let models = dir.join("models");
    let model = match ensure_model(&models, "tiny.en") {
        Ok(m) => m,
        Err(e) => {
            eprintln!("AVISO: no se pudo descargar el modelo ({e}); test saltado");
            return;
        }
    };

    // voz sintética: "hello world this is a test"
    let aiff = dir.join("voz.aiff");
    let st = Command::new("say")
        .args(["-o"])
        .arg(&aiff)
        .args(["hello world this is a test"])
        .status()
        .unwrap();
    assert!(st.success());
    // conformar a 48k estéreo (el formato del pipeline)
    let wav = dir.join("voz48.wav");
    ue_media::conform_audio(&aiff, &wav).unwrap();

    // decimación correcta: ~duración * 16000 muestras
    let samples = wav_to_16k_mono(&wav).unwrap();
    assert!(samples.len() > 16_000, "al menos 1 s de audio: {}", samples.len());

    let doc = transcribe(&wav, &model, Some("en"), ue_core::model::Id::new()).unwrap();
    assert!(!doc.words.is_empty(), "hay palabras");
    let joined = doc
        .words
        .iter()
        .map(|w| w.text.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!("transcripción: {joined}");
    assert!(joined.contains("hello"), "reconoce 'hello': {joined}");
    assert!(joined.contains("test"), "reconoce 'test': {joined}");

    // timestamps crecientes y dentro del audio
    for w in doc.words.windows(2) {
        assert!(w[0].start_us <= w[1].start_us, "palabras ordenadas");
    }
    let total_us = (samples.len() as i64) * 1_000_000 / 16_000;
    assert!(doc.words.last().unwrap().end_us <= total_us + 1_500_000);
    assert!(!doc.segments.is_empty(), "hay frases agrupadas");
}
