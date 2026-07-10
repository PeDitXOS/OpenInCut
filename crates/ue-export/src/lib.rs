//! ue-export v0: real export of the timeline to MP4 via a single ffmpeg
//! process with filter_complex.
//!
//! v0 scope (documented in PLAN §5.6; the wgpu graph will replace it):
//! - Video: flat EDL "the clip on the top track wins" per segment; gaps → black.
//!   Text/effects/transform are not burned in yet (Phase 2).
//! - Audio: all clips with audio (audio and video tracks) with gain,
//!   fades and track volume; mixed with amix.
//! - speed != 1.0 not supported yet (explicit error).

pub mod edl;
pub mod avatar_gen;
pub mod graph;

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use thiserror::Error;
use ue_core::model::{Id, Project};

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("sequence {0} does not exist")]
    NoSequence(Id),
    #[error("nothing to export (empty timeline)")]
    EmptyTimeline,
    #[error("asset {0} does not exist in the pool")]
    MissingAsset(Id),
    #[error("could not run ffmpeg: {0}")]
    Spawn(String),
    #[error("ffmpeg failed:\n{0}")]
    Ffmpeg(String),
    #[error("export cancelled")]
    Cancelled,
}

pub type ExportResult<T> = Result<T, ExportError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExportFormat {
    #[default]
    Mp4,
    /// AAC audio only in an .m4a container.
    M4a,
    /// Animated GIF (optimized palette, no audio).
    Gif,
}

#[derive(Debug, Clone)]
pub struct ExportSettings {
    pub format: ExportFormat,
    /// Maximum output height (None = sequence resolution).
    pub max_height: Option<u32>,
    pub crf: u8,
    pub preset: String,
    pub audio_bitrate_k: u32,
    /// R128 loudness normalization (loudnorm -14 LUFS) at the end of the master.
    pub loudnorm: bool,
    /// Export only [in, out) of the timeline (µs). None = everything.
    /// Shorthand for a single-entry `ranges`.
    pub range: Option<(ue_core::TimeUs, ue_core::TimeUs)>,
    /// Export SEVERAL pieces concatenated in order (µs). Overrides `range`.
    /// Empty = unused. Each piece is trimmed from the finished master, so
    /// transitions, overlays and audio stay exactly as previewed.
    pub ranges: Vec<(ue_core::TimeUs, ue_core::TimeUs)>,
    /// User effect packs (merged on top of the core ones).
    pub extra_packs: Vec<ue_render::EffectDef>,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            format: ExportFormat::default(),
            max_height: None,
            crf: 18,
            preset: "veryfast".into(),
            audio_bitrate_k: 256,
            loudnorm: false,
            range: None,
            ranges: vec![],
            extra_packs: vec![],
        }
    }
}

/// Exports the active sequence to `output` (mp4). Blocking.
pub fn export_sequence(
    project: &Project,
    sequence_id: Id,
    base_dir: &Path,
    output: &Path,
    settings: &ExportSettings,
) -> ExportResult<()> {
    let never = AtomicBool::new(false);
    export_sequence_with_progress(project, sequence_id, base_dir, output, settings, |_| {}, &never)
}

/// Export with progress (0..1) and cooperative cancellation.
/// On cancel, ffmpeg is killed and the partial file is removed.
pub fn export_sequence_with_progress(
    project: &Project,
    sequence_id: Id,
    base_dir: &Path,
    output: &Path,
    settings: &ExportSettings,
    mut on_progress: impl FnMut(f32),
    cancel: &AtomicBool,
) -> ExportResult<()> {
    let plan = graph::build_ffmpeg_args(project, sequence_id, base_dir, output, settings)?;
    let total_us = plan.duration_us.max(1);

    // -progress pipe:1 emits key=value lines on stdout
    let mut args: Vec<String> =
        vec!["-progress".into(), "pipe:1".into(), "-nostats".into()];
    args.extend(plan.args.iter().cloned());

    let mut child = Command::new(ue_media::ffmpeg_bin())
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| ExportError::Spawn(e.to_string()))?;

    // drain stderr in parallel (avoids pipe deadlock and keeps the error)
    let mut stderr = child.stderr.take().expect("stderr piped");
    let stderr_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    let stdout = child.stdout.take().expect("stdout piped");
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(output);
            return Err(ExportError::Cancelled);
        }
        let Ok(line) = line else { break };
        if let Some(v) = line.strip_prefix("out_time_us=") {
            if let Ok(us) = v.trim().parse::<i64>() {
                on_progress((us as f32 / total_us as f32).clamp(0.0, 1.0));
            }
        }
    }

    let status = child.wait().map_err(|e| ExportError::Spawn(e.to_string()))?;
    let err_text = stderr_thread.join().unwrap_or_default();
    if cancel.load(Ordering::SeqCst) {
        let _ = std::fs::remove_file(output);
        return Err(ExportError::Cancelled);
    }
    if !status.success() {
        return Err(ExportError::Ffmpeg(err_text.trim().to_string()));
    }
    on_progress(1.0);
    Ok(())
}
