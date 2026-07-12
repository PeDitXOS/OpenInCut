//! Single-frame preview compositor.
//!
//! The paused preview MUST match the export 1:1 (the golden rule). The export
//! is a linear whole-timeline render, so sampling one frame at time `t`
//! through it would decode everything up to `t` — far too slow for a 20-minute
//! timeline. Instead this composites the SAME instant directly:
//!
//! - every video clip active at `t`, bottom-to-top, its source frame reached
//!   with an input-side `-ss` (fast even for a deep source offset),
//! - each transform/effect chain SAMPLED at `t` (a baked constant, so a single
//!   still renders exactly as the moving export frame would),
//! - the titles and subtitles from the shared `text_overlays_at`.
//!
//! The base clip uses the export's opaque `norm` (fit + pad to canvas); upper
//! layers use its transparent PiP fit and centre overlay — the very strings
//! `build_ffmpeg_args` emits — so the composite is identical by construction.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ue_core::model::{ClipPayload, Id, Project, TrackKind};
use ue_core::TimeUs;
use ue_render::EffectDef;

use crate::graph::{resolve_path_pub, text_overlays_at};
use crate::{ExportError, ExportResult};

/// A single preview frame may never take longer than this.
const FRAME_TIMEOUT: Duration = Duration::from_secs(20);

/// Runs ffmpeg with a hard deadline and KILLS it if it overruns.
///
/// `Command::output()` waits forever. A preview graph that stalls therefore
/// wedged the calling thread AND left the ffmpeg process alive — and because an
/// agent retries a tool that never answers, one bad frame could pile up a dozen
/// ffmpeg processes and peg the machine (observed in the field: ~17 of them).
/// A frame we cannot produce in `timeout` is a frame we do not want.
pub fn run_bounded(args: &[String], timeout: Duration) -> ExportResult<std::process::Output> {
    let mut child = Command::new(ue_media::ffmpeg_bin())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ExportError::Spawn(e.to_string()))?;

    // read both pipes on threads: a full pipe would deadlock the child before
    // the deadline ever fired
    let mut stdout = child.stdout.take().expect("stdout piped");
    let mut stderr = child.stderr.take().expect("stderr piped");
    let out_t = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = stdout.read_to_end(&mut b);
        b
    });
    let err_t = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = stderr.read_to_end(&mut b);
        b
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().map_err(|e| ExportError::Spawn(e.to_string()))? {
            Some(s) => break s,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ExportError::Ffmpeg(format!(
                    "the preview frame took longer than {}s and was killed",
                    timeout.as_secs()
                )));
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    Ok(std::process::Output {
        status,
        stdout: out_t.join().unwrap_or_default(),
        stderr: err_t.join().unwrap_or_default(),
    })
}

fn secs(us: TimeUs) -> String {
    format!("{:.6}", us as f64 / 1_000_000.0)
}

/// The export's xfade parameters when `t_us` falls inside the transition
/// window between two adjacent media clips on the base track — the same
/// handle math as edl.rs `apply_transition_handles`. Returns
/// `(prev, clip, xfade kind, effective duration µs, position µs into it)`.
fn base_xfade_at<'a>(
    project: &Project,
    track: &'a ue_core::model::Track,
    t_us: TimeUs,
) -> Option<(&'a ue_core::model::Clip, &'a ue_core::model::Clip, &'static str, TimeUs, TimeUs)> {
    const MIN_TRANSITION: TimeUs = 40_000;
    for i in 1..track.clips.len() {
        let clip = &track.clips[i];
        let Some(tr) = &clip.transition_in else { continue };
        let prev = &track.clips[i - 1];
        if (prev.end() - clip.start).abs() > 1_000 {
            continue; // not adjacent: the export drops the transition too
        }
        let (
            ClipPayload::Media { asset_id: prev_asset, src_out: prev_out, .. },
            ClipPayload::Media { src_in: cur_in, .. },
        ) = (&prev.payload, &clip.payload)
        else {
            continue;
        };
        let Some(passet) = project.asset(*prev_asset) else { continue };
        let avail_left =
            (((passet.probe.duration_us - prev_out).max(0)) as f64 / prev.speed) as TimeUs;
        let avail_right = (*cur_in as f64 / clip.speed) as TimeUs;
        let half = (tr.duration / 2).min(avail_left).min(avail_right);
        if half * 2 < MIN_TRANSITION {
            continue;
        }
        let cut = clip.start;
        if t_us < cut - half || t_us >= cut + half {
            continue;
        }
        return Some((
            prev,
            clip,
            crate::graph::xfade_kind(&tr.effect_id),
            half * 2,
            t_us - (cut - half),
        ));
    }
    None
}

/// One video clip visible at the sampled instant.
struct ActiveLayer {
    /// Media input path + the source time to seek to, or a generator source.
    source: LayerSource,
    /// Sampled effect+transform chain (`None` = no chain).
    chain: Option<String>,
    /// The clip sits on the export's base track (fills the canvas).
    base: bool,
}

enum LayerSource {
    Media { path: PathBuf, src_time: TimeUs },
    Gen { source: String },
}

/// Renders the composited frame at timeline time `t_us`, scaled to at most
/// `max_width`, as a JPEG. `None` when no video clip is active at `t_us`.
///
/// Mirrors `build_ffmpeg_args`: base track (bottom) fills the canvas, upper
/// tracks overlay centred, then titles/subtitles are burned on top.
pub fn render_preview_frame(
    project: &Project,
    sequence_id: Id,
    base_dir: &Path,
    t_us: TimeUs,
    max_width: u32,
    extra_packs: &[EffectDef],
) -> ExportResult<Option<Vec<u8>>> {
    let seq = project
        .sequence(sequence_id)
        .ok_or(ExportError::NoSequence(sequence_id))?;
    let (cw, ch) = seq.resolution;
    let fps = format!("{}/{}", seq.fps.0, seq.fps.1);
    let registry =
        ue_render::merge_registries(ue_render::core_registry(), extra_packs.to_vec());

    // Base = the first unmuted video track with media/generator clips — the
    // same track the export's EDL uses. An upper-track clip stays PiP-sized
    // even when the base has a gap at t (the export shows it over black,
    // never stretched to fill the canvas).
    let base_track_id = seq
        .tracks
        .iter()
        .find(|t| {
            t.kind == TrackKind::Video
                && !t.muted
                && t.clips.iter().any(|c| {
                    matches!(c.payload, ClipPayload::Media { .. } | ClipPayload::Generator { .. })
                })
        })
        .map(|t| t.id);

    // active video clips, bottom-to-top (track order). One per track: clips on
    // a track never overlap, so at most one covers t.
    let mut layers: Vec<ActiveLayer> = vec![];
    // base-track transition at t: the first two layers become a REAL xfade
    // (kind, effective duration s, position s) — identical to the export's.
    let mut base_xfade: Option<(&'static str, f64, f64)> = None;
    for track in seq.tracks.iter().filter(|t| t.kind == TrackKind::Video && !t.muted) {
        if Some(track.id) == base_track_id {
            if let Some((prev, cur, kind, d_us, pos_us)) = base_xfade_at(project, track, t_us) {
                for c in [prev, cur] {
                    let rel = t_us - c.start; // beyond the clip edges = handle material
                    let chain = ue_render::clip_vf_sampled_ex(
                        &registry,
                        &c.effects,
                        &c.transform,
                        Some((cw, ch)),
                        rel.clamp(0, c.duration),
                        false,
                    );
                    let ClipPayload::Media { asset_id, src_in, .. } = &c.payload else {
                        unreachable!("base_xfade_at only matches media clips");
                    };
                    let Some(asset) = project.asset(*asset_id) else {
                        return Err(ExportError::MissingAsset(*asset_id));
                    };
                    let src_time =
                        (*src_in + (rel as f64 * c.speed).round() as TimeUs).max(0);
                    layers.push(ActiveLayer {
                        source: LayerSource::Media {
                            path: resolve_path_pub(base_dir, &asset.path),
                            src_time,
                        },
                        chain,
                        base: true,
                    });
                }
                base_xfade = Some((kind, d_us as f64 / 1e6, pos_us as f64 / 1e6));
                continue;
            }
        }
        let Some(clip) = track.clips.iter().find(|c| c.start <= t_us && t_us < c.end()) else {
            continue;
        };
        let rel = t_us - clip.start;
        // the base-track clip is opaque; every other layer composites transparently
        let base = Some(track.id) == base_track_id;
        let transparent = !base;
        let chain = ue_render::clip_vf_sampled_ex(
            &registry,
            &clip.effects,
            &clip.transform,
            Some((cw, ch)),
            rel,
            transparent,
        );
        match &clip.payload {
            ClipPayload::Media { asset_id, src_in, .. } => {
                let Some(asset) = project.asset(*asset_id) else {
                    return Err(ExportError::MissingAsset(*asset_id));
                };
                // a still image holds one frame: never seek into it
                let src_time = if asset.kind == ue_core::model::MediaKind::Image {
                    0
                } else {
                    *src_in + (rel as f64 * clip.speed).round() as TimeUs
                };
                layers.push(ActiveLayer {
                    source: LayerSource::Media {
                        path: resolve_path_pub(base_dir, &asset.path),
                        src_time,
                    },
                    chain,
                    base,
                });
            }
            ClipPayload::Generator { generator_id, params, color_params } => {
                let generators = ue_render::core_generators();
                let Some(def) = ue_render::find_generator(&generators, generator_id) else {
                    continue;
                };
                let source =
                    ue_render::render_generator(def, params, color_params, seq.fps, clip.duration);
                layers.push(ActiveLayer { source: LayerSource::Gen { source }, chain, base });
            }
            _ => {} // Text/Subtitles/Avatar are not video layers here
        }
    }

    let text = text_overlays_at(project, seq, ch, cw, t_us);
    let any_styled_text = seq
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video && !t.muted)
        .flat_map(|t| &t.clips)
        .any(|c| {
            c.start <= t_us
                && t_us < c.end()
                && matches!(c.payload, ClipPayload::Text { .. } | ClipPayload::Subtitles { .. })
                && crate::graph::text_is_styled_pub(c)
        });
    if layers.is_empty() && text.is_none() && !any_styled_text {
        return Ok(None); // genuinely nothing on screen (export would be black too)
    }

    // ---- assemble the ffmpeg command ----
    let mut args: Vec<String> = vec!["-v".into(), "error".into()];
    // media inputs, in layer order (generators are filter sources, no -i)
    let mut input_idx: Vec<Option<usize>> = vec![];
    let mut next_input = 0usize;
    for layer in &layers {
        match &layer.source {
            LayerSource::Media { path, src_time } => {
                // a still image is one frame: loop it so the `fps` filter in
                // the base norm has frames to resample (otherwise a base image
                // renders black). Also never seek into it.
                if ue_media::is_image_path(path) {
                    args.extend(["-loop".into(), "1".into()]);
                }
                // input-side seek: fast, and frame-close (keyframe) is fine for
                // a paused preview
                args.push("-ss".into());
                args.push(secs(*src_time));
                args.push("-i".into());
                args.push(path.to_string_lossy().into_owned());
                input_idx.push(Some(next_input));
                next_input += 1;
            }
            LayerSource::Gen { .. } => input_idx.push(None),
        }
    }

    // opaque canvas base for the norm (matches the export's base normalisation)
    let norm = format!(
        "fps={fps},scale={cw}:{ch}:force_original_aspect_ratio=decrease,\
         pad={cw}:{ch}:(ow-iw)/2:(oh-ih)/2,setsar=1,format=yuv420p"
    );
    // transparent PiP fit for upper layers (native size, no pad)
    let layer_fit =
        format!("scale='min({cw},iw)':'min({ch},ih)':force_original_aspect_ratio=decrease");

    let mut fc: Vec<String> = vec![];
    let mut current = String::new();
    let mut skip_layers = 0usize;
    // Base-track transition: xfade the two normalised stills with the REAL
    // transition pattern (loop each into a short stream, xfade over the
    // effective duration, take the frame at the exact progress). This is the
    // very filter the export runs, so wipes/slides/etc match 1:1 — not a
    // crossfade approximation.
    if let Some((kind, d, pos)) = &base_xfade {
        let fr = seq.fps.0 as f64 / seq.fps.1.max(1) as f64;
        let n = (d * fr).ceil() as i64 + 2;
        let pos = pos.min(d - (0.5 / fr).min(*d)).max(0.0);
        for (j, layer) in layers.iter().take(2).enumerate() {
            let Some(i) = input_idx[j] else { continue };
            let chain = layer.chain.as_deref().map(|c| format!("{c},")).unwrap_or_default();
            fc.push(format!("[{i}:v]{chain}{norm}[xs{j}]"));
            // settb: xfade requires both inputs on one timebase (see graph.rs)
            fc.push(format!(
                "[xs{j}]settb=AVTB,loop=loop={n}:size=1:start=0,setpts=N/({fps})/TB[xl{j}]"
            ));
        }
        fc.push(format!(
            "[xl0][xl1]xfade=transition={kind}:duration={d:.6}:offset=0[xfo]"
        ));
        fc.push(format!("[xfo]trim=start={pos:.6},setpts=PTS-STARTPTS[c0]"));
        current = "c0".into();
        skip_layers = 2;
    }
    // base track with a gap at t (or no base at all): layers go over black,
    // exactly like the export's Black EDL segment
    if current.is_empty() && !layers.first().map(|l| l.base).unwrap_or(true) {
        fc.push(format!("color=c=black:s={cw}x{ch}:d=1,format=yuv420p[c0]"));
        current = "c0".into();
    }
    for (k, layer) in layers.iter().enumerate().skip(skip_layers) {
        let src_label = match (&layer.source, input_idx[k]) {
            (LayerSource::Media { .. }, Some(i)) => format!("[{i}:v]"),
            (LayerSource::Gen { source }, _) => format!("{source},"),
            _ => continue,
        };
        let chain = layer.chain.as_deref().map(|c| format!("{c},")).unwrap_or_default();
        if layer.base && current.is_empty() {
            // base: chain (opaque, fits to canvas) then the export's norm
            fc.push(format!("{src_label}{chain}{norm}[c0]"));
            current = "c0".into();
        } else {
            // layer: chain (transparent) + PiP fit + rgba, then centre overlay
            fc.push(format!("{src_label}{chain}{layer_fit},format=rgba[l{k}]"));
            let out = format!("c{}", k + 1);
            fc.push(format!(
                "[{current}][l{k}]overlay=x=(W-w)/2:y=(H-h)/2:eof_action=pass[{out}]"
            ));
            current = out;
        }
    }
    // no video layer but we have text → over black
    if current.is_empty() {
        fc.push(format!("color=c=black:s={cw}x{ch}:d=1,format=yuv420p[c0]"));
        current = "c0".into();
    }
    // TITLES: rasterised exactly as the export does it (ue-text, colour fonts and
    // all) and overlaid as an image layer. Doing anything else here would put
    // pause and export out of step — the one thing this compositor exists to
    // prevent — and would also lose the emoji the export now renders.
    let mut tk = 0usize;
    let mut temp_files: Vec<PathBuf> = vec![];
    let tmp_dir = std::env::temp_dir();
    for track in seq.tracks.iter().filter(|t| t.kind == TrackKind::Video && !t.muted) {
        for clip in track.clips.iter().filter(|c| c.start <= t_us && t_us < c.end()) {
            let ClipPayload::Text { content, style } = &clip.payload else { continue };
            if content.trim().is_empty() {
                continue;
            }
            let Ok(img) = ue_text::render_to_png(
                &ue_text::TextSpec {
                    content,
                    style,
                    out_w: cw,
                    out_h: ch,
                    width_fraction: ue_text::DEFAULT_WIDTH_FRACTION,
                },
                &tmp_dir,
            ) else {
                continue;
            };
            let vf = ue_render::clip_vf_sampled_ex(
                &registry,
                &clip.effects,
                &clip.transform,
                Some((cw, ch)),
                t_us - clip.start,
                true,
            )
            .map(|c| format!("{c},"))
            .unwrap_or_default();
            args.extend(["-loop".into(), "1".into(), "-i".into(), img.to_string_lossy().into_owned()]);
            let idx = next_input;
            next_input += 1;
            temp_files.push(img);
            fc.push(format!("[{idx}:v]{vf}format=rgba[tl{tk}]"));
            let out = format!("tc{tk}");
            fc.push(format!(
                "[{current}][tl{tk}]overlay=x=(W-w)/2:y=(H-h)/2:eof_action=pass[{out}]"
            ));
            current = out;
            tk += 1;
        }
    }

    // styled SUBTITLES clips still ride the drawtext layer path
    for track in seq.tracks.iter().filter(|t| t.kind == TrackKind::Video && !t.muted) {
        for clip in track.clips.iter().filter(|c| c.start <= t_us && t_us < c.end()) {
            if !matches!(clip.payload, ClipPayload::Subtitles { .. }) {
                continue;
            }
            if !crate::graph::text_is_styled_pub(clip) {
                continue; // burned in by libass with the plain ones below
            }
            let Some(chain) =
                crate::graph::text_clip_chain(project, seq, clip.id, ch, cw, Some(t_us), &[])
            else {
                continue;
            };
            let vf = ue_render::clip_vf_sampled_ex(
                &registry,
                &clip.effects,
                &clip.transform,
                Some((cw, ch)),
                t_us - clip.start,
                true,
            )
            .map(|c| format!("{c},"))
            .unwrap_or_default();
            fc.push(format!(
                "color=c=black@0:s={cw}x{ch}:d=1,format=rgba,{chain},{vf}format=rgba[tl{tk}]"
            ));
            let out = format!("tc{tk}");
            fc.push(format!(
                "[{current}][tl{tk}]overlay=x=(W-w)/2:y=(H-h)/2:eof_action=pass[{out}]"
            ));
            current = out;
            tk += 1;
        }
    }

    // titles + subtitles: the SAME libass script the export burns in. The frame
    // is stamped with the timeline PTS first so libass picks exactly the events
    // on screen at `t_us` — using drawtext here while the export used ASS would
    // put pause and export back out of step, which is the one thing this whole
    // compositor exists to prevent.
    let mut subs_file: Option<PathBuf> = None;
    if let Some(script) = crate::ass::build_script(project, seq, cw, ch, None) {
        let dir = std::env::temp_dir();
        let path = crate::ass::write_script(&script, &dir)
            .map_err(|e| ExportError::Ffmpeg(format!("could not write the subtitles: {e}")))?;
        fc.push(format!(
            "[{current}]setpts=PTS+{}/TB,{},setpts=PTS-STARTPTS[txt]",
            secs(t_us),
            crate::ass::ass_filter(&path),
        ));
        current = "txt".into();
        subs_file = Some(path);
    }
    // final downscale to the preview width
    fc.push(format!("[{current}]scale='min({max_width},iw)':-2[out]"));

    args.push("-filter_complex".into());
    args.push(fc.join(";"));
    args.extend(["-map".into(), "[out]".into()]);
    args.extend([
        "-frames:v".into(), "1".into(),
        "-f".into(), "image2".into(),
        "-c:v".into(), "mjpeg".into(),
        "-q:v".into(), "4".into(),
        "pipe:1".into(),
    ]);

    let out = run_bounded(&args, FRAME_TIMEOUT);
    if let Some(p) = &subs_file {
        let _ = std::fs::remove_file(p);
    }
    for p in &temp_files {
        let _ = std::fs::remove_file(p);
    }
    let out = out?;
    if !out.status.success() {
        return Err(ExportError::Ffmpeg(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    if out.stdout.is_empty() {
        return Ok(None);
    }
    Ok(Some(out.stdout))
}

/// Test hook for the deadline: same runner the preview uses.
pub fn run_bounded_for_test(
    args: &[String],
    timeout: Duration,
) -> ExportResult<std::process::Output> {
    run_bounded(args, timeout)
}
