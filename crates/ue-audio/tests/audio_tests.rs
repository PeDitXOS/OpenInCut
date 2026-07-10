//! ue-audio tests: WAV parser, mixer with exact synthetic signals,
//! audible-clip collection, real conforming (ffmpeg) and cpal output
//! (with a graceful skip if there's no device).

use std::path::{Path, PathBuf};

use ue_audio::items::{collect_specs, load_items};
use ue_audio::mixer::{db_to_linear, fill, mix_frame, MixItem};
use ue_audio::wav::WavMap;
use ue_audio::{us_to_frames, RATE};
use ue_core::model::*;
use ue_core::ops::InsertMode;
use ue_core::ProjectStore;

const SEC: i64 = 1_000_000;

fn tmp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ue-audio-tests");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// 48k stereo WAV with a per-frame generator.
fn write_wav(name: &str, frames: i64, gen: impl Fn(i64) -> (i16, i16)) -> PathBuf {
    let path = tmp(name);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).unwrap();
    for i in 0..frames {
        let (l, r) = gen(i);
        w.write_sample(l).unwrap();
        w.write_sample(r).unwrap();
    }
    w.finalize().unwrap();
    path
}

fn dc_item(path: &PathBuf, timeline_start: i64, len: i64) -> MixItem {
    MixItem {
        wav: WavMap::open(path).unwrap(),
        timeline_start,
        src_in: 0,
        len,
        speed: 1.0,
        gain: 1.0,
        gain_curve: None,
        pan: 0.0,
        stretcher: None,
        fade_in: 0,
        fade_out: 0,
    }
}

// ---------------------------------------------------------------------------
// WAV parser
// ---------------------------------------------------------------------------

#[test]
fn wav_roundtrip_and_bounds() {
    let path = write_wav("ramp.wav", 1000, |i| ((i * 16) as i16, (-i * 16) as i16));
    let wav = WavMap::open(&path).unwrap();
    assert_eq!(wav.frames(), 1000);
    assert_eq!(wav.sample_rate, RATE);
    let (l, r) = wav.frame(10);
    assert!((l - 160.0 / 32768.0).abs() < 1e-6);
    assert!((r + 160.0 / 32768.0).abs() < 1e-6);
    assert_eq!(wav.frame(-1), (0.0, 0.0));
    assert_eq!(wav.frame(1000), (0.0, 0.0), "out of range → silence");
}

#[test]
fn compute_peaks_reflects_signal_shape() {
    // 1 s of silence + 1 s at full scale → half the bins ~0, half ~1
    let frames = 2 * RATE as i64;
    let path = write_wav("peaks.wav", frames, |i| {
        if i < RATE as i64 { (0, 0) } else { (32767, 32767) }
    });
    let wav = WavMap::open(&path).unwrap();
    let peaks = ue_audio::wav::compute_peaks(&wav, 25);
    assert_eq!(peaks.len(), 50, "2 s × 25 bins/s");
    assert!(peaks[..25].iter().all(|p| *p < 0.01), "first half silence");
    assert!(peaks[25..].iter().all(|p| *p > 0.9), "second half full scale");
}

// ---------------------------------------------------------------------------
// Mezclador
// ---------------------------------------------------------------------------

#[test]
fn gain_in_db_is_applied() {
    let path = write_wav("dc_half.wav", 100, |_| (16384, 16384)); // 0.5
    let mut item = dc_item(&path, 0, 100);
    item.gain = db_to_linear(-6.0206); // ≈ 0.5×
    let (l, _) = mix_frame(&[item], 50);
    assert!((l - 0.25).abs() < 1e-3, "0.5 × -6dB ≈ 0.25, was {l}");
}

#[test]
fn pan_balance_law_attenuates_opposite_channel() {
    let path = write_wav("dc_pan.wav", 100, |_| (16384, 16384)); // 0.5 both
    // pan 1.0 = full right: left dies, right intact
    let mut item = dc_item(&path, 0, 100);
    item.pan = 1.0;
    let (l, r) = mix_frame(&[item], 50);
    assert!(l.abs() < 1e-6, "left silenced, was {l}");
    assert!((r - 0.5).abs() < 1e-3, "right untouched, was {r}");
    // pan -0.5: right halved, left intact
    let mut item = dc_item(&path, 0, 100);
    item.pan = -0.5;
    let (l, r) = mix_frame(&[item], 50);
    assert!((l - 0.5).abs() < 1e-3, "left unity, was {l}");
    assert!((r - 0.25).abs() < 1e-3, "right at 0.5×, was {r}");
}

#[test]
fn gain_curve_animates_during_playback() {
    use ue_core::keyframe::{Interp, Keyframe, KeyframeCurve};
    // 1 s of DC 0.5 with a linear curve 0 dB → -20 dB
    let frames = RATE as i64;
    let path = write_wav("dc_curve.wav", frames, |_| (16384, 16384));
    let mut item = dc_item(&path, 0, frames);
    item.gain_curve = Some(KeyframeCurve::new(vec![
        Keyframe { t: 0, value: 0.0, interp: Interp::Linear },
        Keyframe { t: SEC, value: -20.0, interp: Interp::Linear },
    ]));
    let items = [item];
    let (start, _) = mix_frame(&items, 0);
    assert!((start - 0.5).abs() < 1e-3, "start at 0 dB, was {start}");
    let (mid, _) = mix_frame(&items, frames / 2);
    let expected = 0.5 * db_to_linear(-10.0);
    assert!((mid - expected).abs() < 2e-3, "center ≈ -10 dB ({expected}), was {mid}");
}

#[test]
fn overlapping_items_sum_and_clamp() {
    let path = write_wav("dc_04.wav", 100, |_| (13107, 13107)); // 0.4
    let a = dc_item(&path, 0, 100);
    let b = dc_item(&path, 0, 100);
    let (l, _) = mix_frame(&[a, b], 10);
    assert!((l - 0.8).abs() < 1e-3, "sum 0.4+0.4");

    let path_hot = write_wav("dc_09.wav", 100, |_| (29491, 29491)); // 0.9
    let a = dc_item(&path_hot, 0, 100);
    let b = dc_item(&path_hot, 0, 100);
    let (l, _) = mix_frame(&[a, b], 10);
    assert_eq!(l, 1.0, "clamp to 1.0");
}

#[test]
fn timeline_offset_and_src_in_mapping() {
    // exact ramp signal: frame i equals i*16
    let path = write_wav("ramp2.wav", 2000, |i| ((i * 16) as i16, (i * 16) as i16));
    let item = MixItem {
        wav: WavMap::open(&path).unwrap(),
        timeline_start: 500,
        src_in: 100,
        len: 300,
        speed: 1.0,
        gain: 1.0,
        gain_curve: None,
        pan: 0.0,
        stretcher: None,
        fade_in: 0,
        fade_out: 0,
    };
    // before the clip → silence
    assert_eq!(mix_frame(&[item], 499).0, 0.0);
    // the clip re-opens the wav for more asserts
    let item = MixItem {
        wav: WavMap::open(&path).unwrap(),
        timeline_start: 500,
        src_in: 100,
        len: 300,
        speed: 1.0,
        gain: 1.0,
        gain_curve: None,
        pan: 0.0,
        stretcher: None,
        fade_in: 0,
        fade_out: 0,
    };
    // timeline frame 500 = source frame 100 = 1600/32768
    let expect = |src: i64| (src * 16) as f32 / 32768.0;
    assert!((mix_frame(&[item], 500).0 - expect(100)).abs() < 1e-6);
    let item = MixItem {
        wav: WavMap::open(&path).unwrap(),
        timeline_start: 500,
        src_in: 100,
        len: 300,
        speed: 1.0,
        gain: 1.0,
        gain_curve: None,
        pan: 0.0,
        stretcher: None,
        fade_in: 0,
        fade_out: 0,
    };
    // last frame of the clip: timeline 799 → source 399; and 800 is already silence
    assert!((mix_frame(&[item], 799).0 - expect(399)).abs() < 1e-6);
}

#[test]
fn fades_ramp_linearly() {
    let path = write_wav("dc_full.wav", 1000, |_| (32767, 32767)); // ≈1.0
    let item = MixItem {
        wav: WavMap::open(&path).unwrap(),
        timeline_start: 0,
        src_in: 0,
        len: 1000,
        speed: 1.0,
        gain: 1.0,
        gain_curve: None,
        pan: 0.0,
        stretcher: None,
        fade_in: 200,
        fade_out: 200,
    };
    let items = [item];
    assert_eq!(mix_frame(&items, 0).0, 0.0, "start of the fade-in");
    let mid_in = mix_frame(&items, 100).0;
    assert!((mid_in - 0.5).abs() < 0.01, "middle of the fade-in ≈ 0.5, was {mid_in}");
    let plateau = mix_frame(&items, 500).0;
    assert!(plateau > 0.99, "plateau at full gain");
    let mid_out = mix_frame(&items, 900).0;
    assert!((mid_out - 0.5).abs() < 0.01, "middle of the fade-out ≈ 0.5, was {mid_out}");
}

#[test]
fn fill_is_contiguous() {
    let path = write_wav("ramp3.wav", 1000, |i| ((i * 16) as i16, (i * 16) as i16));
    let items = [dc_item(&path, 0, 1000)];
    let mut buf = vec![0f32; 20]; // 10 stereo frames
    fill(&items, 100, &mut buf);
    for k in 0..10i64 {
        let expect = ((100 + k) * 16) as f32 / 32768.0;
        assert!((buf[(k * 2) as usize] - expect).abs() < 1e-6);
    }
}

// ---------------------------------------------------------------------------
// Collection from the project
// ---------------------------------------------------------------------------

fn asset(kind: MediaKind, channels: u32, dur_s: i64) -> MediaAsset {
    MediaAsset {
        id: Id::new(),
        kind,
        path: format!("{kind:?}.dat"),
        content_hash: format!("h{channels}{dur_s}"),
        probe: ProbeInfo {
            duration_us: dur_s * SEC,
            fps: None,
            width: 0,
            height: 0,
            rotation: 0,
            vcodec: None,
            acodec: Some("aac".into()),
            audio_channels: channels,
            vfr: false,
        },
        proxy: None,
        audio_conform: None,
        peaks: None,
        thumbnails: None,
        transcript: None,
        offline: false,
    }
}

#[test]
fn collect_respects_mute_solo_and_video_audio() {
    let mut p = Project::new("t");
    let seq_id = p.active_sequence;
    let music = asset(MediaKind::Audio, 2, 60);
    let video_with_audio = asset(MediaKind::Video, 2, 60);
    let video_silent = asset(MediaKind::Video, 0, 60);
    let (m, va, vs) = (music.id, video_with_audio.id, video_silent.id);
    p.assets.extend([music, video_with_audio, video_silent]);
    let atrack = p.sequence(seq_id).unwrap().tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap().id;
    let vtrack = p.sequence(seq_id).unwrap().tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap().id;
    let mut store = ProjectStore::new(p);
    store.insert_clip(atrack, Clip::new_media(m, 0, 5 * SEC, 0), InsertMode::Strict).unwrap();
    store.insert_clip(vtrack, Clip::new_media(va, 0, 5 * SEC, 0), InsertMode::Strict).unwrap();
    store.insert_clip(vtrack, Clip::new_media(vs, 0, 5 * SEC, 6 * SEC), InsertMode::Strict).unwrap();

    // both with audio come in; the video without audio doesn't
    let specs = collect_specs(&store.project, seq_id);
    assert_eq!(specs.len(), 2);

    // muting the audio track → only the video's remains
    store
        .dispatch("mute", vec![ue_core::Action::SetTrackProp {
            track_id: atrack,
            prop: ue_core::action::TrackProp::Muted(true),
        }])
        .unwrap();
    let specs = collect_specs(&store.project, seq_id);
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].asset_id, va);

    // solo on the audio track (unmuted) → only the music
    store
        .dispatch("unmute+solo", vec![
            ue_core::Action::SetTrackProp { track_id: atrack, prop: ue_core::action::TrackProp::Muted(false) },
            ue_core::Action::SetTrackProp { track_id: atrack, prop: ue_core::action::TrackProp::Solo(true) },
        ])
        .unwrap();
    let specs = collect_specs(&store.project, seq_id);
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].asset_id, m);
}

#[test]
fn load_items_skips_missing_conform() {
    let mut p = Project::new("t");
    let seq_id = p.active_sequence;
    let music = asset(MediaKind::Audio, 2, 60);
    let mid = music.id;
    p.assets.push(music);
    let atrack = p.sequence(seq_id).unwrap().tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap().id;
    let mut store = ProjectStore::new(p);
    store.insert_clip(atrack, Clip::new_media(mid, 0, 5 * SEC, 0), InsertMode::Strict).unwrap();
    let specs = collect_specs(&store.project, seq_id);
    let (items, skipped) = load_items(&store.project, &specs, |_| None);
    assert!(items.is_empty());
    assert_eq!(skipped, vec![mid]);
}

// ---------------------------------------------------------------------------
// Real conforming (ffmpeg) and playback (cpal) — with a graceful skip
// ---------------------------------------------------------------------------

#[test]
fn conform_produces_valid_48k_stereo_wav() {
    let ff_ok = std::process::Command::new(ue_media::ffmpeg_bin())
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ff_ok {
        eprintln!("WARNING: no ffmpeg; conform test skipped");
        return;
    }
    // deliberately different source: 22.05 kHz mono
    let src = tmp("tone_22k.wav");
    let st = std::process::Command::new(ue_media::ffmpeg_bin())
        .args(["-y", "-v", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=2:sample_rate=22050"])
        .arg(&src)
        .status()
        .unwrap();
    assert!(st.success());

    let out = tmp("conformed/audio.wav");
    ue_media::conform_audio(&src, &out).unwrap();
    let wav = WavMap::open(&out).unwrap();
    assert_eq!(wav.sample_rate, RATE);
    let dur_frames = wav.frames();
    assert!((dur_frames - 2 * RATE as i64).abs() < RATE as i64 / 10, "≈2 s, was {dur_frames}");
    // idempotent: doesn't re-conform if it exists
    ue_media::conform_audio(&src, &out).unwrap();
    // there's real signal (ffmpeg's `sine` source is low-amplitude,
    // ~0.05: the threshold distinguishes signal from silence, not levels)
    let mean_sq: f32 = (0..dur_frames.min(48000))
        .map(|i| wav.frame(i).0.powi(2))
        .sum::<f32>()
        / 48000.0;
    assert!(mean_sq > 1e-4, "the tone has energy (mean²={mean_sq})");
}

#[test]
fn player_clock_advances_if_device_available() {
    match ue_audio::player::Player::new() {
        Err(e) => eprintln!("WARNING: no audio device ({e}); player test skipped"),
        Ok(player) => {
            player.set_items(vec![], 1);
            player.play(1 * SEC);
            std::thread::sleep(std::time::Duration::from_millis(300));
            let pos = player.pause();
            let advanced = pos - 1 * SEC;
            assert!(
                (100_000..=900_000).contains(&advanced),
                "the audio clock advanced ~300 ms, was {advanced} µs"
            );
            // seek repositions
            player.seek(10 * SEC);
            assert_eq!(player.position_us(), 10 * SEC);
            let _ = us_to_frames(0); // silences unused in builds without asserts
        }
    }
}

/// THE pitch test: a 220 Hz sine played at 2× through the WSOLA stretcher
/// must still be ~220 Hz (the naive resample would give 440 Hz).
#[test]
fn wsola_preserves_pitch_at_double_speed() {
    use ue_audio::stretch::Wsola;
    let sr = RATE as f64;
    let hz = 220.0;
    let frames = 4 * RATE as i64;
    let path = write_wav("sine220.wav", frames, |i| {
        let v = (2.0 * std::f64::consts::PI * hz * i as f64 / sr).sin();
        (((v * 20000.0) as i16), ((v * 20000.0) as i16))
    });
    let wav = WavMap::open(&path).unwrap();

    let mut st = Wsola::new(2.0);
    // skip the fade-in, then collect half a second of stretched output
    let n = RATE as i64 / 2;
    let mut out = Vec::with_capacity(n as usize);
    for rel in 0..(n + 2048) {
        let (l, _r) = st.frame_at(&wav, 0, rel);
        if rel >= 2048 {
            out.push(l);
        }
    }
    // zero crossings → frequency
    let mut crossings = 0u32;
    for w in out.windows(2) {
        if (w[0] >= 0.0) != (w[1] >= 0.0) {
            crossings += 1;
        }
    }
    let freq = crossings as f64 / 2.0 / (out.len() as f64 / sr);
    assert!(
        (195.0..=245.0).contains(&freq),
        "pitch preserved at 2x: expected ~220 Hz, got {freq:.1} Hz"
    );
    // and the naive resample really does shift it (sanity check of the test)
    let mut crossings2 = 0u32;
    let mut prev = 0.0f32;
    for rel in 0..n {
        let (l, _r) = wav.frame((rel as f64 * 2.0) as i64);
        if rel > 0 && (prev >= 0.0) != (l >= 0.0) {
            crossings2 += 1;
        }
        prev = l;
    }
    let freq2 = crossings2 as f64 / 2.0 / (n as f64 / sr);
    assert!(freq2 > 400.0, "naive resample doubles the pitch ({freq2:.1} Hz)");
}

/// Stretched output has no gross discontinuities (no clicks): the max jump
/// between consecutive samples stays in the same ballpark as the source's.
#[test]
fn wsola_output_is_continuous() {
    use ue_audio::stretch::Wsola;
    let sr = RATE as f64;
    let frames = 2 * RATE as i64;
    let path = write_wav("sine330.wav", frames, |i| {
        let v = (2.0 * std::f64::consts::PI * 330.0 * i as f64 / sr).sin();
        (((v * 20000.0) as i16), ((v * 20000.0) as i16))
    });
    let wav = WavMap::open(&path).unwrap();
    let mut st = Wsola::new(1.5);
    let mut prev = 0.0f32;
    let mut max_jump = 0.0f32;
    for rel in 0..RATE as i64 {
        let (l, _r) = st.frame_at(&wav, 0, rel);
        if rel > 2048 {
            max_jump = max_jump.max((l - prev).abs());
        }
        prev = l;
    }
    // a 330 Hz sine at full scale moves ≤ ~0.045/sample; allow overlap slack
    assert!(max_jump < 0.09, "no clicks: max jump {max_jump}");
}
