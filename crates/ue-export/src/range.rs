//! Restricting a sequence to the ranges actually being exported.
//!
//! The export used to build the WHOLE timeline and trim the wanted range off
//! the end of the filtergraph (`[vout]trim=start=X`). ffmpeg therefore rendered
//! everything from t=0 and threw it away, so a 46-second clip starting at minute
//! 22 spent ~12 minutes writing nothing at all — and, because `-progress`
//! reports OUTPUT time, the progress bar sat at 0 the whole while and then
//! jumped straight to 1. Cost was proportional to where the range STARTED, not
//! to how long it was.
//!
//! Instead we rewrite the sequence up front so it contains only the material
//! inside the ranges, spliced back to back from t=0. The graph downstream then
//! has nothing to throw away: no trailing trim, honest progress, and (with the
//! per-segment input seek in `graph.rs`) no decoding of the parts of the source
//! file nobody asked for.

use ue_core::keyframe::Param;
use ue_core::model::{AudioProps, Clip, ClipPayload, EffectInstance, Id, Project, Transform2D};
use ue_core::TimeUs;

/// Keyframe times live relative to the clip's start, so a clip trimmed on its
/// left has to have its curves rebased by the same amount.
fn rebase(p: &Param, delta: TimeUs) -> Param {
    if delta <= 0 {
        return p.clone();
    }
    p.split(delta).1.sanitized()
}

fn rebase_transform(t: &Transform2D, delta: TimeUs) -> Transform2D {
    Transform2D {
        position: (rebase(&t.position.0, delta), rebase(&t.position.1, delta)),
        scale: (rebase(&t.scale.0, delta), rebase(&t.scale.1, delta)),
        rotation: rebase(&t.rotation, delta),
        crop: (
            rebase(&t.crop.0, delta),
            rebase(&t.crop.1, delta),
            rebase(&t.crop.2, delta),
            rebase(&t.crop.3, delta),
        ),
        opacity: rebase(&t.opacity, delta),
        flip_h: t.flip_h,
        flip_v: t.flip_v,
    }
}

fn rebase_audio(a: &AudioProps, delta: TimeUs, keep_fade_in: bool, keep_fade_out: bool) -> AudioProps {
    AudioProps {
        gain_db: rebase(&a.gain_db, delta),
        pan: rebase(&a.pan, delta),
        // a fade only survives if the edge it hangs off is still the clip's edge
        fade_in_us: if keep_fade_in { a.fade_in_us } else { 0 },
        fade_out_us: if keep_fade_out { a.fade_out_us } else { 0 },
        muted: a.muted,
        denoise: a.denoise,
    }
}

fn rebase_effects(effects: &[EffectInstance], delta: TimeUs) -> Vec<EffectInstance> {
    effects
        .iter()
        .map(|e| EffectInstance {
            effect_id: e.effect_id.clone(),
            enabled: e.enabled,
            params: e.params.iter().map(|(k, v)| (k.clone(), rebase(v, delta))).collect(),
            color_params: e.color_params.clone(),
        })
        .collect()
}

/// The piece of `clip` that lives inside `[from, to)`, moved to `dest` on the
/// new timeline. `None` when the clip does not reach into the range at all.
fn clip_slice(clip: &Clip, from: TimeUs, to: TimeUs, dest: TimeUs) -> Option<Clip> {
    let start = clip.start.max(from);
    let end = clip.end().min(to);
    if end <= start {
        return None;
    }
    let head = start - clip.start; // how much was cut off the left
    let tail = clip.end() - end; // how much was cut off the right
    let duration = end - start;

    let mut out = clip.clone();
    out.id = Id::new(); // a slice is a new clip: ids stay unique
    out.start = dest + (start - from);
    out.duration = duration;
    out.transform = rebase_transform(&clip.transform, head);
    out.audio = rebase_audio(&clip.audio, head, head == 0, tail == 0);
    out.effects = rebase_effects(&clip.effects, head);
    // a transition needs the neighbouring clip's material; once we cut into the
    // clip's left edge that neighbour may not be in the export at all
    if head != 0 {
        out.transition_in = None;
    }
    if let ClipPayload::Media { src_in, src_out, .. } = &mut out.payload {
        let src_head = (head as f64 * clip.speed).round() as TimeUs;
        let src_len = (duration as f64 * clip.speed).round() as TimeUs;
        let new_in = *src_in + src_head;
        *src_out = (new_in + src_len).min(*src_out);
        *src_in = new_in;
    }
    Some(out)
}

/// A copy of `project` whose sequence holds ONLY the material inside `ranges`,
/// spliced contiguously from t=0. Empty `ranges` returns the project untouched.
///
/// Ranges are normalised (clamped, sorted, overlaps merged) so the output is
/// always a clean, monotonically increasing timeline — which is exactly what
/// the caller would have got from the old trailing `trim`+`concat`, minus the
/// cost of rendering everything in between.
pub fn restrict_to_ranges(
    project: &Project,
    sequence_id: Id,
    ranges: &[(TimeUs, TimeUs)],
) -> Project {
    if ranges.is_empty() {
        return project.clone();
    }
    let Some(seq) = project.sequence(sequence_id) else { return project.clone() };
    let end = seq
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .map(|c| c.end())
        .max()
        .unwrap_or(0);

    // normalise: clamp to the timeline, drop empties, sort, merge overlaps
    let mut wanted: Vec<(TimeUs, TimeUs)> = ranges
        .iter()
        .map(|&(a, b)| (a.clamp(0, end), b.clamp(0, end)))
        .filter(|(a, b)| b > a)
        .collect();
    if wanted.is_empty() {
        return project.clone();
    }
    wanted.sort_by_key(|r| r.0);
    let mut merged: Vec<(TimeUs, TimeUs)> = vec![];
    for r in wanted {
        match merged.last_mut() {
            Some(last) if r.0 <= last.1 => last.1 = last.1.max(r.1),
            _ => merged.push(r),
        }
    }

    let mut out = project.clone();
    let Some(seq_mut) = out.sequence_mut(sequence_id) else { return out };
    for track in &mut seq_mut.tracks {
        let source = std::mem::take(&mut track.clips);
        let mut dest = 0;
        for &(from, to) in &merged {
            for clip in &source {
                if let Some(slice) = clip_slice(clip, from, to, dest) {
                    track.clips.push(slice);
                }
            }
            dest += to - from;
        }
        track.clips.sort_by_key(|c| c.start);
    }
    out
}

/// Total output duration of the restricted timeline.
pub fn ranges_duration(ranges: &[(TimeUs, TimeUs)]) -> TimeUs {
    ranges.iter().filter(|(a, b)| b > a).map(|(a, b)| b - a).sum()
}
