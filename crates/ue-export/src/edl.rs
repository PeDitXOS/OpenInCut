//! Flattens the video timeline into an EDL: a list of consecutive segments where
//! each segment is either a range of an asset (the top track wins) or black.

use std::collections::BTreeSet;

use ue_core::model::{ClipPayload, Id, Project, TrackKind};
use ue_core::TimeUs;

use crate::{ExportError, ExportResult};

#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    /// Generated lavfi source (rectangle, gradient, …), already rendered with
    /// its window duration.
    Gen {
        source: String,
        duration: TimeUs,
        vf: Option<String>,
    },
    Source {
        asset_id: Id,
        src_in: TimeUs,
        src_out: TimeUs,
        /// Clip speed (rate stretch): output = source / speed.
        speed: f64,
        /// Clip effects+transform chain (ue-render), already rendered.
        vf: Option<String>,
        /// Transition with the previous segment: (output duration µs, effect_id).
        /// The handles are already extended in src_in/src_out by the post-pass.
        transition_in: Option<(TimeUs, String)>,
    },
    Black { duration: TimeUs },
}

impl Segment {
    /// OUTPUT duration of the segment (the source divided by the speed).
    pub fn duration(&self) -> TimeUs {
        match self {
            Segment::Source { src_in, src_out, speed, .. } => {
                (((src_out - src_in) as f64) / speed).round() as TimeUs
            }
            Segment::Gen { duration, .. } => *duration,
            Segment::Black { duration } => *duration,
        }
    }
}

/// Builds the EDL with only the core packs (shortcut for tests and simple uses).
pub fn build_video_edl(project: &Project, sequence_id: Id) -> ExportResult<Vec<Segment>> {
    build_video_edl_with(project, sequence_id, &[])
}

/// Builds the sequence's video EDL (core packs + `extra_packs`).
/// Errors if there is speed != 1 on a visible clip or if the timeline is empty.
pub fn build_video_edl_with(
    project: &Project,
    sequence_id: Id,
    extra_packs: &[ue_render::EffectDef],
) -> ExportResult<Vec<Segment>> {
    let seq = project
        .sequence(sequence_id)
        .ok_or(ExportError::NoSequence(sequence_id))?;

    // boundaries: starts and ends of all media clips on visible video tracks
    let mut cuts: BTreeSet<TimeUs> = BTreeSet::new();
    cuts.insert(0);
    for track in seq.tracks.iter().filter(|t| t.kind == TrackKind::Video && !t.muted) {
        for clip in &track.clips {
            if matches!(clip.payload, ClipPayload::Media { .. } | ClipPayload::Generator { .. }) {
                cuts.insert(clip.start);
                cuts.insert(clip.end());
            }
        }
    }
    let cuts: Vec<TimeUs> = cuts.into_iter().collect();
    if cuts.len() < 2 {
        return Err(ExportError::EmptyTimeline);
    }

    // per segment, resolve the visible clip (top track wins)
    let registry = ue_render::merge_registries(ue_render::core_registry(), extra_packs.to_vec());
    let generators = ue_render::core_generators();
    let mut segments: Vec<Segment> = vec![];
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let mid = a + (b - a) / 2;
        let mut found: Option<Segment> = None;
        for track in seq.tracks.iter().rev().filter(|t| t.kind == TrackKind::Video && !t.muted) {
            for clip in &track.clips {
                if clip.start <= mid && mid < clip.end() {
                    if let ClipPayload::Generator { generator_id, params, color_params } =
                        &clip.payload
                    {
                        if let Some(def) =
                            ue_render::find_generator(&generators, generator_id)
                        {
                            found = Some(Segment::Gen {
                                source: ue_render::render_generator(
                                    def,
                                    params,
                                    color_params,
                                    seq.fps,
                                    b - a,
                                ),
                                duration: b - a,
                                vf: ue_render::clip_vf(
                                    &registry,
                                    &clip.effects,
                                    &clip.transform,
                                    Some(seq.resolution),
                                ),
                            });
                        }
                        break;
                    }
                    if let ClipPayload::Media { asset_id, src_in, src_out } = &clip.payload {
                        if project.asset(*asset_id).is_none() {
                            return Err(ExportError::MissingAsset(*asset_id));
                        }
                        let s_in =
                            *src_in + ((a - clip.start) as f64 * clip.speed).round() as TimeUs;
                        let s_out =
                            *src_in + ((b - clip.start) as f64 * clip.speed).round() as TimeUs;
                        // the transition belongs to the FIRST segment of the clip
                        let transition_in = if a == clip.start {
                            clip.transition_in
                                .as_ref()
                                .map(|t| (t.duration, t.effect_id.clone()))
                        } else {
                            None
                        };
                        found = Some(Segment::Source {
                            asset_id: *asset_id,
                            src_in: s_in,
                            src_out: s_out.min(*src_out),
                            speed: clip.speed,
                            vf: ue_render::clip_vf(&registry, &clip.effects, &clip.transform, Some(seq.resolution)),
                            transition_in,
                        });
                    }
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        segments.push(found.unwrap_or(Segment::Black { duration: b - a }));
    }

    // trim trailing black (after the last segment with content)
    while matches!(segments.last(), Some(Segment::Black { .. })) {
        segments.pop();
    }
    if segments.is_empty() {
        return Err(ExportError::EmptyTimeline);
    }

    // merge contiguous segments of the same asset with a continuous source, the same
    // effects and no transition in between
    let mut merged: Vec<Segment> = vec![];
    for seg in segments {
        match (merged.last_mut(), &seg) {
            (
                Some(Segment::Source { asset_id: a1, src_out: o1, vf: v1, speed: s1, .. }),
                Segment::Source {
                    asset_id: a2,
                    src_in: i2,
                    src_out: o2,
                    vf: v2,
                    speed: s2,
                    transition_in: None,
                },
            ) if a1 == a2 && o1 == i2 && v1 == v2 && (s1.to_bits() == s2.to_bits()) => {
                *o1 = *o2
            }
            (Some(Segment::Black { duration: d1 }), Segment::Black { duration: d2 }) => {
                *d1 += *d2;
            }
            _ => merged.push(seg),
        }
    }

    apply_transition_handles(project, &mut merged);
    Ok(merged)
}

/// Total OUTPUT duration of the EDL in µs (crossfades overlap material,
/// so they subtract their duration).
pub fn edl_duration(segments: &[Segment]) -> TimeUs {
    let raw: TimeUs = segments.iter().map(|s| s.duration()).sum();
    let overlapped: TimeUs = segments
        .iter()
        .filter_map(|s| match s {
            Segment::Source { transition_in: Some((d, _)), .. } => Some(*d),
            _ => None,
        })
        .sum();
    raw - overlapped
}

/// Transition post-pass: validates that there is a contiguous Source segment before,
/// extends the handles (half on each side, limited by the file's
/// material) and reduces or removes the transition if there is not enough material.
fn apply_transition_handles(project: &Project, segments: &mut [Segment]) {
    const MIN_TRANSITION: TimeUs = 40_000; // below ~1 frame it's not worth it
    for i in 0..segments.len() {
        let Segment::Source {
            transition_in: Some((want, _)),
            src_in: cur_in,
            speed: cur_speed,
            ..
        } = &segments[i]
        else {
            continue;
        };
        let (want, cur_in, cur_speed) = (*want, *cur_in, *cur_speed);
        // no Source segment right before → no transition
        if i == 0 {
            if let Segment::Source { transition_in, .. } = &mut segments[i] {
                *transition_in = None;
            }
            continue;
        }
        // availability in OUTPUT TIME (asset / speed on each side)
        let prev = match &segments[i - 1] {
            Segment::Source { asset_id, src_out, speed, .. } => {
                let dur = project.asset(*asset_id).map(|a| a.probe.duration_us).unwrap_or(0);
                Some((((dur - src_out).max(0) as f64 / speed) as TimeUs, *speed))
            }
            _ => None,
        };
        let Some((avail_left_out, prev_speed)) = prev else {
            if let Segment::Source { transition_in, .. } = &mut segments[i] {
                *transition_in = None;
            }
            continue;
        };
        let avail_right_out = (cur_in as f64 / cur_speed) as TimeUs;
        let half = (want / 2).min(avail_left_out).min(avail_right_out);
        let effective = half * 2;
        if effective < MIN_TRANSITION {
            if let Segment::Source { transition_in, .. } = &mut segments[i] {
                *transition_in = None;
            }
            continue;
        }
        if let Segment::Source { src_out, .. } = &mut segments[i - 1] {
            *src_out += (half as f64 * prev_speed).round() as TimeUs;
        }
        if let Segment::Source { src_in, transition_in, .. } = &mut segments[i] {
            *src_in -= (half as f64 * cur_speed).round() as TimeUs;
            if let Some((d, _)) = transition_in {
                *d = effective;
            }
        }
    }
}
