//! Construcción de la línea de comandos de ffmpeg (inputs + filter_complex).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ue_core::model::{ClipPayload, Id, Project};
use ue_core::TimeUs;

use crate::edl::{build_video_edl, edl_duration, Segment};
use crate::{ExportError, ExportResult, ExportSettings};

pub struct FfmpegPlan {
    pub args: Vec<String>,
    pub duration_us: TimeUs,
}

fn secs(us: TimeUs) -> String {
    format!("{:.6}", us as f64 / 1_000_000.0)
}

fn resolve_path(base: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() { path.to_path_buf() } else { base.join(path) }
}

/// Clip audible: cualquier clip media (en pista de audio o video) cuyo asset
/// tenga audio, sin mute de clip ni de pista.
struct AudioItem {
    asset_id: Id,
    src_in: TimeUs,
    src_out: TimeUs,
    start: TimeUs,
    gain_db: f64,
    fade_in_us: TimeUs,
    fade_out_us: TimeUs,
}

fn collect_audio(project: &Project, sequence_id: Id) -> Vec<AudioItem> {
    let Some(seq) = project.sequence(sequence_id) else { return vec![] };
    let mut items = vec![];
    let any_solo = seq.tracks.iter().any(|t| t.solo);
    for track in &seq.tracks {
        if track.muted || (any_solo && !track.solo) {
            continue;
        }
        for clip in &track.clips {
            if clip.audio.muted {
                continue;
            }
            if let ClipPayload::Media { asset_id, src_in, src_out } = &clip.payload {
                let Some(asset) = project.asset(*asset_id) else { continue };
                if asset.probe.audio_channels == 0 {
                    continue;
                }
                items.push(AudioItem {
                    asset_id: *asset_id,
                    src_in: *src_in,
                    src_out: *src_out,
                    start: clip.start,
                    gain_db: clip.audio.gain_db.eval(0) + track.volume_db as f64,
                    fade_in_us: clip.audio.fade_in_us,
                    fade_out_us: clip.audio.fade_out_us,
                });
            }
        }
    }
    items
}

pub fn build_ffmpeg_args(
    project: &Project,
    sequence_id: Id,
    base_dir: &Path,
    output: &Path,
    settings: &ExportSettings,
) -> ExportResult<FfmpegPlan> {
    let seq = project
        .sequence(sequence_id)
        .ok_or(ExportError::NoSequence(sequence_id))?;
    let edl = build_video_edl(project, sequence_id)?;
    let total_us = edl_duration(&edl);
    let audio_items = collect_audio(project, sequence_id);

    let (mut out_w, mut out_h) = seq.resolution;
    if let Some(mh) = settings.max_height {
        if out_h > mh {
            out_w = (out_w as u64 * mh as u64 / out_h as u64) as u32 & !1;
            out_h = mh & !1;
        }
    }
    let fps = format!("{}/{}", seq.fps.0, seq.fps.1);

    // inputs únicos por asset
    let mut input_index: BTreeMap<Id, usize> = BTreeMap::new();
    let mut inputs: Vec<PathBuf> = vec![];
    let mut input_of = |asset_id: Id, project: &Project| -> usize {
        *input_index.entry(asset_id).or_insert_with(|| {
            let asset = project.asset(asset_id).expect("validado en la EDL");
            inputs.push(resolve_path(base_dir, &asset.path));
            inputs.len() - 1
        })
    };

    // ---- cadenas de video ----
    let mut fc: Vec<String> = vec![];
    let norm = format!(
        "fps={fps},scale={out_w}:{out_h}:force_original_aspect_ratio=decrease,\
         pad={out_w}:{out_h}:(ow-iw)/2:(oh-ih)/2,setsar=1,format=yuv420p"
    );
    let mut vlabels: Vec<String> = vec![];
    for (k, seg) in edl.iter().enumerate() {
        let label = format!("v{k}");
        match seg {
            Segment::Source { asset_id, src_in, src_out, vf } => {
                let idx = input_of(*asset_id, project);
                let effects = match vf {
                    Some(chain) => format!("{chain},"),
                    None => String::new(),
                };
                fc.push(format!(
                    "[{idx}:v]trim=start={}:end={},setpts=PTS-STARTPTS,{effects}{norm}[{label}]",
                    secs(*src_in),
                    secs(*src_out),
                ));
            }
            Segment::Black { duration } => {
                fc.push(format!(
                    "color=black:size={out_w}x{out_h}:rate={fps}:duration={}[{label}]",
                    secs(*duration),
                ));
            }
        }
        vlabels.push(format!("[{label}]"));
    }
    fc.push(format!(
        "{}concat=n={}:v=1:a=0[vout]",
        vlabels.join(""),
        vlabels.len()
    ));

    // ---- cadenas de audio ----
    let mut alabels: Vec<String> = vec![];
    for (k, item) in audio_items.iter().enumerate() {
        let idx = input_of(item.asset_id, project);
        let label = format!("a{k}");
        let dur_us = item.src_out - item.src_in;
        let mut chain = format!(
            "[{idx}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,\
             aresample=48000,aformat=channel_layouts=stereo",
            secs(item.src_in),
            secs(item.src_out),
        );
        if item.gain_db.abs() > 1e-9 {
            chain.push_str(&format!(",volume={:.2}dB", item.gain_db));
        }
        if item.fade_in_us > 0 {
            chain.push_str(&format!(",afade=t=in:st=0:d={}", secs(item.fade_in_us)));
        }
        if item.fade_out_us > 0 {
            chain.push_str(&format!(
                ",afade=t=out:st={}:d={}",
                secs(dur_us - item.fade_out_us),
                secs(item.fade_out_us),
            ));
        }
        if item.start > 0 {
            chain.push_str(&format!(",adelay={}:all=1", item.start / 1000)); // ms
        }
        chain.push_str(&format!("[{label}]"));
        fc.push(chain);
        alabels.push(format!("[{label}]"));
    }
    let has_audio = !alabels.is_empty();
    if has_audio {
        fc.push(format!(
            "{}amix=inputs={}:duration=longest:normalize=0,atrim=0:{}[aout]",
            alabels.join(""),
            alabels.len(),
            secs(total_us),
        ));
    }

    // ---- línea de comandos ----
    let mut args: Vec<String> = vec!["-y".into(), "-v".into(), "error".into()];
    for input in &inputs {
        args.push("-i".into());
        args.push(input.to_string_lossy().into_owned());
    }
    args.push("-filter_complex".into());
    args.push(fc.join(";"));
    args.extend(["-map".into(), "[vout]".into()]);
    if has_audio {
        args.extend(["-map".into(), "[aout]".into()]);
        args.extend([
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            format!("{}k", settings.audio_bitrate_k),
        ]);
    } else {
        args.push("-an".into());
    }
    args.extend([
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        settings.preset.clone(),
        "-crf".into(),
        settings.crf.to_string(),
        "-movflags".into(),
        "+faststart".into(),
    ]);
    args.push(output.to_string_lossy().into_owned());

    Ok(FfmpegPlan { args, duration_us: total_us })
}
