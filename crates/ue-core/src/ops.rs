//! High-level operations (split, delete with ripple, overwrite, move…).
//! Each op PLANS a list of primitive actions by simulating them on a
//! clone of the project (Planner); the store then executes them as a transaction.

use crate::action::{apply, Action};
use crate::error::{UeError, UeResult};
use crate::model::*;
use crate::time::{frame_duration_us, quantize_to_frame, TimeUs};

/// Simulates actions on a clone to plan consistent transactions.
pub struct Planner {
    pub proj: Project,
    pub actions: Vec<Action>,
}

impl Planner {
    pub fn new(project: &Project) -> Self {
        Planner { proj: project.clone(), actions: vec![] }
    }

    /// Applies to the clone and records it. If it fails, the whole transaction aborts.
    pub fn do_(&mut self, action: Action) -> UeResult<()> {
        apply(&mut self.proj, action.clone())?;
        self.actions.push(action);
        Ok(())
    }

    pub fn finish(self) -> Vec<Action> {
        self.actions
    }
}

/// Ids of a clip's linked group (including itself).
pub fn linked_ids(project: &Project, clip_id: Id) -> Vec<Id> {
    let Some(clip) = project.clip(clip_id) else { return vec![clip_id] };
    let Some(group) = clip.group else { return vec![clip_id] };
    let mut out = vec![];
    for seq in &project.sequences {
        for track in &seq.tracks {
            for c in &track.clips {
                if c.group == Some(group) {
                    out.push(c.id);
                }
            }
        }
    }
    if out.is_empty() {
        out.push(clip_id);
    }
    out
}

fn ensure_unlocked(track: &Track) -> UeResult<()> {
    if track.locked {
        return Err(UeError::Locked(track.name.clone()));
    }
    Ok(())
}

/// Splits a clip at timeline time `t` (quantized to frame).
/// LINKED clips that cross `t` are split too; the right halves
/// share a new group. Returns (actions, left, right) of the
/// requested clip.
pub fn split_clip(project: &Project, clip_id: Id, t: TimeUs) -> UeResult<(Vec<Action>, Id, Id)> {
    let (si, ti, ci) = project
        .locate_clip(clip_id)
        .ok_or_else(|| UeError::NotFound(format!("clip {clip_id}")))?;
    let seq = &project.sequences[si];
    ensure_unlocked(&seq.tracks[ti])?;
    let clip = &seq.tracks[ti].clips[ci];

    let t = quantize_to_frame(t, seq.fps);
    if t <= clip.start || t >= clip.end() {
        return Err(UeError::Invalid(format!(
            "the cut point {t} is outside the clip ({}..{})",
            clip.start,
            clip.end()
        )));
    }

    let group = linked_ids(project, clip_id);
    let new_right_group = if group.len() > 1 { Some(Id::new()) } else { None };
    let mut plan = Planner::new(project);
    let mut primary: Option<(Id, Id)> = None;
    for gid in group {
        let Some((gsi, gti, gci)) = plan.proj.locate_clip(gid) else { continue };
        let gclip = plan.proj.sequences[gsi].tracks[gti].clips[gci].clone();
        let gtrack_id = plan.proj.sequences[gsi].tracks[gti].id;
        if !(gclip.start < t && t < gclip.end()) {
            continue; // this linked clip doesn't cross the cut
        }
        let (left, mut right) = split_clip_data(&gclip, t - gclip.start);
        if let Some(ng) = new_right_group {
            right.group = Some(ng);
        }
        let ids = (left.id, right.id);
        plan.do_(Action::ReplaceClips {
            track_id: gtrack_id,
            remove_ids: vec![gid],
            insert: vec![left, right],
        })?;
        if gid == clip_id {
            primary = Some(ids);
        }
    }
    let (l, r) = primary.ok_or_else(|| UeError::Invalid("nothing to split".into()))?;
    Ok((plan.finish(), l, r))
}

/// Builds the two halves of a clip split at `offset` (µs relative to the clip).
fn split_clip_data(clip: &Clip, offset: TimeUs) -> (Clip, Clip) {
    let mut left = clip.clone();
    let mut right = clip.clone();
    left.id = Id::new();
    right.id = Id::new();

    left.duration = offset;
    right.start = clip.start + offset;
    right.duration = clip.duration - offset;

    // Source range for a Media payload (mapped by speed).
    if let (ClipPayload::Media { src_in, src_out, .. }, ClipPayload::Media { src_in: r_in, .. }) =
        (&mut left.payload, &mut right.payload)
    {
        let src_off = (offset as f64 * clip.speed).round() as TimeUs;
        let boundary = (*src_in + src_off).min(*src_out - 1);
        *src_out = boundary;
        *r_in = boundary;
    }

    // Keyframe curves: split them preserving the value at the boundary.
    let (tl, tr) = split_transform(&clip.transform, offset);
    left.transform = tl;
    right.transform = tr;
    let (al, ar) = split_audio(&clip.audio, offset, clip.duration);
    left.audio = al;
    right.audio = ar;
    for (le, re) in left.effects.iter_mut().zip(right.effects.iter_mut()) {
        for (key, param) in le.params.clone() {
            let (pl, pr) = param.split(offset);
            le.params.insert(key.clone(), pl);
            re.params.insert(key, pr);
        }
    }

    // The in-transition stays on the left; the right one starts clean.
    right.transition_in = None;

    (left, right)
}

fn split_transform(t: &Transform2D, offset: TimeUs) -> (Transform2D, Transform2D) {
    let mut l = t.clone();
    let mut r = t.clone();
    macro_rules! sp {
        ($field:expr, $lf:expr, $rf:expr) => {{
            let (a, b) = $field.split(offset);
            *$lf = a;
            *$rf = b;
        }};
    }
    sp!(t.position.0, &mut l.position.0, &mut r.position.0);
    sp!(t.position.1, &mut l.position.1, &mut r.position.1);
    sp!(t.scale.0, &mut l.scale.0, &mut r.scale.0);
    sp!(t.scale.1, &mut l.scale.1, &mut r.scale.1);
    sp!(t.rotation, &mut l.rotation, &mut r.rotation);
    sp!(t.crop.0, &mut l.crop.0, &mut r.crop.0);
    sp!(t.crop.1, &mut l.crop.1, &mut r.crop.1);
    sp!(t.crop.2, &mut l.crop.2, &mut r.crop.2);
    sp!(t.crop.3, &mut l.crop.3, &mut r.crop.3);
    sp!(t.opacity, &mut l.opacity, &mut r.opacity);
    (l, r)
}

fn split_audio(a: &AudioProps, offset: TimeUs, total: TimeUs) -> (AudioProps, AudioProps) {
    let mut l = a.clone();
    let mut r = a.clone();
    let (gl, gr) = a.gain_db.split(offset);
    let (pl, pr) = a.pan.split(offset);
    l.gain_db = gl;
    r.gain_db = gr;
    l.pan = pl;
    r.pan = pr;
    // Fades: the fade-in belongs to the left, the fade-out to the right.
    l.fade_in_us = a.fade_in_us.min(offset);
    l.fade_out_us = 0;
    r.fade_in_us = 0;
    r.fade_out_us = a.fade_out_us.min(total - offset);
    (l, r)
}

/// Deletes clips. `ripple=true` closes the gaps by shifting what comes after
/// (v1: clips with ripple must be on a single track).
pub fn delete_clips(project: &Project, ids: &[Id], ripple: bool) -> UeResult<Vec<Action>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    // expand with the linked clips (video+audio go together)
    let mut ids: Vec<Id> = ids.to_vec();
    for id in ids.clone() {
        for linked in linked_ids(project, id) {
            if !ids.contains(&linked) {
                ids.push(linked);
            }
        }
    }
    let ids = &ids;
    // Group by track and validate locks.
    let mut by_track: std::collections::BTreeMap<Id, Vec<Id>> = Default::default();
    for id in ids {
        let (si, ti, _) = project
            .locate_clip(*id)
            .ok_or_else(|| UeError::NotFound(format!("clip {id}")))?;
        let track = &project.sequences[si].tracks[ti];
        ensure_unlocked(track)?;
        by_track.entry(track.id).or_default().push(*id);
    }
    // multi-track ripple: each track closes its own gaps (linked pairs have
    // identical gaps, so they stay aligned)

    let mut plan = Planner::new(project);
    for (track_id, clip_ids) in by_track {
        // Removed intervals, sorted.
        let track = project.track(track_id).unwrap();
        let mut removed: Vec<(TimeUs, TimeUs)> = clip_ids
            .iter()
            .map(|id| {
                let c = &track.clips[track.clip_index(*id).unwrap()];
                (c.start, c.end())
            })
            .collect();
        removed.sort();

        for id in &clip_ids {
            plan.do_(Action::RemoveClip { clip_id: *id })?;
        }

        if ripple {
            // Shift each remaining clip left by the sum of what was removed
            // before its start (left to right: no collisions).
            let survivors: Vec<(Id, TimeUs, TimeUs)> = plan
                .proj
                .track(track_id)
                .unwrap()
                .clips
                .iter()
                .map(|c| (c.id, c.start, c.duration))
                .collect();
            for (cid, start, duration) in survivors {
                let shift: TimeUs = removed
                    .iter()
                    .filter(|(_, e)| *e <= start)
                    .map(|(s, e)| e - s)
                    .sum();
                if shift > 0 {
                    plan.do_(Action::SetClipBounds {
                        clip_id: cid,
                        start: start - shift,
                        duration,
                        src_in: None,
                        src_out: None,
                    })?;
                }
            }
        }
    }
    Ok(plan.finish())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertMode {
    /// Fails if there is a collision.
    Strict,
    /// Trims/deletes whatever is underneath (classic NLE behavior).
    Overwrite,
}

/// Inserts a clip into a track. In Overwrite mode it trims whatever it overlaps.
pub fn insert_clip(
    project: &Project,
    track_id: Id,
    clip: Clip,
    mode: InsertMode,
) -> UeResult<Vec<Action>> {
    let track = project
        .track(track_id)
        .ok_or_else(|| UeError::NotFound(format!("track {track_id}")))?;
    ensure_unlocked(track)?;

    let mut plan = Planner::new(project);
    if mode == InsertMode::Overwrite {
        carve_range(&mut plan, track_id, clip.start, clip.end(), None)?;
    }
    plan.do_(Action::InsertClip { track_id, clip })?;
    Ok(plan.finish())
}

/// Moves a clip (possibly to another track). Overwrite trims the destination.
pub fn move_clip(
    project: &Project,
    clip_id: Id,
    to_track: Id,
    to_start: TimeUs,
    mode: InsertMode,
) -> UeResult<Vec<Action>> {
    let (si, ti, ci) = project
        .locate_clip(clip_id)
        .ok_or_else(|| UeError::NotFound(format!("clip {clip_id}")))?;
    let seq = &project.sequences[si];
    let from_track = &seq.tracks[ti];
    ensure_unlocked(from_track)?;
    let target = project
        .track(to_track)
        .ok_or_else(|| UeError::NotFound(format!("track {to_track}")))?;
    ensure_unlocked(target)?;

    let to_start = quantize_to_frame(to_start.max(0), seq.fps);
    let old_start = seq.tracks[ti].clips[ci].start;
    let duration = seq.tracks[ti].clips[ci].duration;
    let delta = to_start - old_start;

    let mut plan = Planner::new(project);
    if mode == InsertMode::Overwrite {
        carve_range(&mut plan, to_track, to_start, to_start + duration, Some(clip_id))?;
    }
    plan.do_(Action::MoveClipToTrack { clip_id, to_track, to_start })?;
    // the linked clips shift by the same delta on THEIR tracks
    if delta != 0 {
        for gid in linked_ids(project, clip_id) {
            if gid == clip_id {
                continue;
            }
            let Some(gclip) = plan.proj.clip(gid) else { continue };
            let gstart = (gclip.start + delta).max(0);
            let gtrack = plan
                .proj
                .sequences
                .iter()
                .flat_map(|s| s.tracks.iter())
                .find(|t| t.clip_index(gid).is_some())
                .map(|t| t.id)
                .unwrap();
            if mode == InsertMode::Overwrite {
                let gdur = gclip.duration;
                carve_range(&mut plan, gtrack, gstart, gstart + gdur, Some(gid))?;
            }
            plan.do_(Action::MoveClipToTrack { clip_id: gid, to_track: gtrack, to_start: gstart })?;
        }
    }
    Ok(plan.finish())
}

/// Trims one edge of a clip. `left=true` moves the left edge.
/// `new_edge` is the new timeline time of the edge (quantized).
pub fn trim_clip(project: &Project, clip_id: Id, left: bool, new_edge: TimeUs) -> UeResult<Vec<Action>> {
    let (si, ti, ci) = project
        .locate_clip(clip_id)
        .ok_or_else(|| UeError::NotFound(format!("clip {clip_id}")))?;
    let seq = &project.sequences[si];
    let track = &seq.tracks[ti];
    ensure_unlocked(track)?;
    let clip = &track.clips[ci];
    let min_dur = frame_duration_us(seq.fps);
    let new_edge = quantize_to_frame(new_edge, seq.fps);

    let mut start = clip.start;
    let duration; // set in both branches
    let (mut src_in, mut src_out) = match &clip.payload {
        ClipPayload::Media { src_in, src_out, .. } => (Some(*src_in), Some(*src_out)),
        _ => (None, None),
    };

    if left {
        let max_edge = clip.end() - min_dur;
        // limit by available source material (not before the start of the file)
        let min_edge = match src_in {
            Some(si_v) => clip.start - (si_v as f64 / clip.speed).round() as TimeUs,
            None => TimeUs::MIN / 4,
        };
        let edge = new_edge.clamp(min_edge.max(0), max_edge);
        let delta = edge - clip.start;
        start = edge;
        duration = clip.duration - delta;
        if let Some(v) = src_in.as_mut() {
            *v += (delta as f64 * clip.speed).round() as TimeUs;
        }
    } else {
        let min_edge = clip.start + min_dur;
        // limit by source material (asset duration)
        let max_by_src = match (src_in, src_out) {
            (Some(si_v), Some(_)) => {
                let asset_dur = asset_duration(project, clip);
                match asset_dur {
                    Some(d) => clip.start + ((d - si_v) as f64 / clip.speed).round() as TimeUs,
                    None => TimeUs::MAX / 4,
                }
            }
            _ => TimeUs::MAX / 4,
        };
        let edge = new_edge.clamp(min_edge, max_by_src);
        duration = edge - clip.start;
        if let Some(v) = src_out.as_mut() {
            *v = src_in.unwrap() + (duration as f64 * clip.speed).round() as TimeUs;
        }
    }

    let mut plan = Planner::new(project);
    plan.do_(Action::SetClipBounds { clip_id, start, duration, src_in, src_out })?;
    Ok(plan.finish())
}

fn asset_duration(project: &Project, clip: &Clip) -> Option<TimeUs> {
    if let ClipPayload::Media { asset_id, .. } = &clip.payload {
        project.asset(*asset_id).map(|a| a.probe.duration_us)
    } else {
        None
    }
}

/// Frees the range [from, to) of a track by trimming/deleting/splitting the
/// clips that overlap it (optionally excluding one clip).
fn carve_range(
    plan: &mut Planner,
    track_id: Id,
    from: TimeUs,
    to: TimeUs,
    exclude: Option<Id>,
) -> UeResult<()> {
    let victims: Vec<Clip> = plan
        .proj
        .track(track_id)
        .ok_or_else(|| UeError::NotFound(format!("track {track_id}")))?
        .clips
        .iter()
        .filter(|c| Some(c.id) != exclude && c.start < to && from < c.end())
        .cloned()
        .collect();

    for c in victims {
        let covered_left = c.start >= from;
        let covered_right = c.end() <= to;
        match (covered_left, covered_right) {
            // fully covered → gone
            (true, true) => plan.do_(Action::RemoveClip { clip_id: c.id })?,
            // overlapped on the left → trim its start up to `to`
            (true, false) => {
                let delta = to - c.start;
                let (si, so) = media_src(&c);
                plan.do_(Action::SetClipBounds {
                    clip_id: c.id,
                    start: to,
                    duration: c.duration - delta,
                    src_in: si.map(|v| v + (delta as f64 * c.speed).round() as TimeUs),
                    src_out: so,
                })?;
            }
            // overlapped on the right → trim its end back to `from`
            (false, true) => {
                let new_dur = from - c.start;
                let (si, so) = media_src(&c);
                plan.do_(Action::SetClipBounds {
                    clip_id: c.id,
                    start: c.start,
                    duration: new_dur,
                    src_in: si,
                    src_out: so.map(|_| {
                        media_src(&c).0.unwrap() + (new_dur as f64 * c.speed).round() as TimeUs
                    }),
                })?;
            }
            // the clip wraps the whole range → split in two and trim the middle
            (false, false) => {
                let (left, right) = split_clip_data(&c, from - c.start);
                // right starts at `from`; trim its start up to `to`
                let delta = to - from;
                let mut right2 = right.clone();
                right2.start = to;
                right2.duration = right.duration - delta;
                if let ClipPayload::Media { src_in, .. } = &mut right2.payload {
                    *src_in += (delta as f64 * c.speed).round() as TimeUs;
                }
                plan.do_(Action::ReplaceClips {
                    track_id,
                    remove_ids: vec![c.id],
                    insert: vec![left, right2],
                })?;
            }
        }
    }
    Ok(())
}

fn media_src(c: &Clip) -> (Option<TimeUs>, Option<TimeUs>) {
    match &c.payload {
        ClipPayload::Media { src_in, src_out, .. } => (Some(*src_in), Some(*src_out)),
        _ => (None, None),
    }
}

/// Changes the speed of a media clip (rate stretch). The new duration is
/// quantized to frame; error if it would collide with the next clip.
pub fn set_clip_speed(project: &Project, clip_id: Id, speed: f64) -> UeResult<Vec<Action>> {
    if !(0.05..=20.0).contains(&speed) {
        return Err(UeError::Invalid("speed out of range (0.05–20)".into()));
    }
    let (si, ti, ci) = project
        .locate_clip(clip_id)
        .ok_or_else(|| UeError::NotFound(format!("clip {clip_id}")))?;
    let seq = &project.sequences[si];
    ensure_unlocked(&seq.tracks[ti])?;
    let clip = &seq.tracks[ti].clips[ci];
    let ClipPayload::Media { src_in, src_out, .. } = &clip.payload else {
        return Err(UeError::Invalid("only media clips have speed".into()));
    };
    let src_len = src_out - src_in;
    let duration = quantize_to_frame(((src_len as f64) / speed).round() as TimeUs, seq.fps)
        .max(frame_duration_us(seq.fps));
    let mut plan = Planner::new(project);
    plan.do_(Action::SetClipSpeed { clip_id, speed, duration })?;
    Ok(plan.finish())
}

/// Speeds up the given ranges (e.g. silences): cuts at the boundaries, applies
/// `factor` to the interior clips (which shrink), and closes the gaps with
/// ripple. Multi-track, one transaction.
pub fn speedup_ranges(
    project: &Project,
    sequence_id: Id,
    ranges: &[(TimeUs, TimeUs)],
    factor: f64,
) -> UeResult<Vec<Action>> {
    if factor <= 1.0 {
        return Err(UeError::Invalid("the speedup factor must be > 1".into()));
    }
    let seq = project
        .sequence(sequence_id)
        .ok_or_else(|| UeError::NotFound(format!("sequence {sequence_id}")))?;
    let fps = seq.fps;
    let track_ids: Vec<Id> = seq.tracks.iter().filter(|t| !t.locked).map(|t| t.id).collect();

    // normalize and sort RIGHT to LEFT: shrinking a range shifts the ones
    // after it, so by processing back to front the earlier ranges are not
    // invalidated.
    let mut rs: Vec<(TimeUs, TimeUs)> = ranges
        .iter()
        .map(|(a, b)| (quantize_to_frame((*a).max(0), fps), quantize_to_frame(*b, fps)))
        .filter(|(a, b)| b > a)
        .collect();
    rs.sort();
    rs.reverse();

    let mut plan = Planner::new(project);
    for (from, to) in rs {
        split_all_at(&mut plan, &track_ids, from)?;
        split_all_at(&mut plan, &track_ids, to)?;
        let len = to - from;
        // shrink the interior clips
        let mut shrunk_by: TimeUs = 0;
        for tid in &track_ids {
            let inside: Vec<Clip> = plan
                .proj
                .track(*tid)
                .unwrap()
                .clips
                .iter()
                .filter(|c| c.start >= from && c.end() <= to)
                .cloned()
                .collect();
            for c in inside {
                if let ClipPayload::Media { src_in, src_out, .. } = &c.payload {
                    let src_len = src_out - src_in;
                    let new_speed = c.speed * factor;
                    let new_dur =
                        quantize_to_frame(((src_len as f64) / new_speed).round() as TimeUs, fps)
                            .max(frame_duration_us(fps));
                    // reposition proportionally within the shrunken range
                    let rel = c.start - from;
                    let new_start = from + quantize_to_frame(
                        ((rel as f64) / factor).round() as TimeUs,
                        fps,
                    );
                    plan.do_(Action::SetClipSpeed {
                        clip_id: c.id,
                        speed: new_speed,
                        duration: new_dur,
                    })?;
                    // moving left doesn't collide: we process ranges right to
                    // left and, within a range, in natural order
                    plan.do_(Action::SetClipBounds {
                        clip_id: c.id,
                        start: new_start,
                        duration: new_dur,
                        src_in: None,
                        src_out: None,
                    })?;
                    shrunk_by = shrunk_by.max(len - quantize_to_frame(
                        ((len as f64) / factor).round() as TimeUs,
                        fps,
                    ));
                }
            }
        }
        // close the gap: everything starting at >= to shifts left
        let shift = shrunk_by;
        if shift > 0 {
            for tid in &track_ids {
                let after: Vec<(Id, TimeUs, TimeUs)> = plan
                    .proj
                    .track(*tid)
                    .unwrap()
                    .clips
                    .iter()
                    .filter(|c| c.start >= to)
                    .map(|c| (c.id, c.start, c.duration))
                    .collect();
                for (cid, start, duration) in after {
                    plan.do_(Action::SetClipBounds {
                        clip_id: cid,
                        start: start - shift,
                        duration,
                        src_in: None,
                        src_out: None,
                    })?;
                }
            }
        }
    }
    Ok(plan.finish())
}

/// Splits at `t` any clip that crosses it, across all the given tracks.
fn split_all_at(plan: &mut Planner, track_ids: &[Id], t: TimeUs) -> UeResult<()> {
    for tid in track_ids {
        let victim = plan
            .proj
            .track(*tid)
            .and_then(|tr| {
                tr.clips
                    .iter()
                    .find(|c| c.start < t && t < c.end())
                    .cloned()
            });
        if let Some(c) = victim {
            let (left, right) = split_clip_data(&c, t - c.start);
            plan.do_(Action::ReplaceClips {
                track_id: *tid,
                remove_ids: vec![c.id],
                insert: vec![left, right],
            })?;
        }
    }
    Ok(())
}

/// Moves the range [from, to) of the sequence to `dest` (outside the range),
/// across all unlocked tracks. Cuts at the boundaries, closes the source gap,
/// opens a gap at the destination, and places it back. The basis of "move
/// phrases" in text editing. One transaction.
pub fn move_range(
    project: &Project,
    sequence_id: Id,
    from: TimeUs,
    to: TimeUs,
    dest: TimeUs,
) -> UeResult<Vec<Action>> {
    let seq = project
        .sequence(sequence_id)
        .ok_or_else(|| UeError::NotFound(format!("sequence {sequence_id}")))?;
    let from = quantize_to_frame(from.max(0), seq.fps);
    let to = quantize_to_frame(to, seq.fps);
    let dest = quantize_to_frame(dest.max(0), seq.fps);
    if to <= from {
        return Err(UeError::Invalid("empty range".into()));
    }
    if dest > from && dest < to {
        return Err(UeError::Invalid("the destination cannot fall inside the range".into()));
    }
    let len = to - from;
    let track_ids: Vec<Id> = seq.tracks.iter().filter(|t| !t.locked).map(|t| t.id).collect();

    let mut plan = Planner::new(project);
    // 1. exact boundaries
    split_all_at(&mut plan, &track_ids, from)?;
    split_all_at(&mut plan, &track_ids, to)?;
    split_all_at(&mut plan, &track_ids, dest)?;

    // 2. extract the clips in the range (clones) and remove them
    let mut moved: Vec<(Id, Clip)> = vec![]; // (track, clip)
    for tid in &track_ids {
        let inside: Vec<Clip> = plan
            .proj
            .track(*tid)
            .unwrap()
            .clips
            .iter()
            .filter(|c| c.start >= from && c.end() <= to)
            .cloned()
            .collect();
        for c in inside {
            plan.do_(Action::RemoveClip { clip_id: c.id })?;
            moved.push((*tid, c));
        }
    }

    // 3. close the source gap (left, ascending)
    for tid in &track_ids {
        let after: Vec<(Id, TimeUs, TimeUs)> = plan
            .proj
            .track(*tid)
            .unwrap()
            .clips
            .iter()
            .filter(|c| c.start >= to)
            .map(|c| (c.id, c.start, c.duration))
            .collect();
        for (cid, start, duration) in after {
            plan.do_(Action::SetClipBounds {
                clip_id: cid,
                start: start - len,
                duration,
                src_in: None,
                src_out: None,
            })?;
        }
    }

    // 4. destination adjusted after closing the gap
    let dest_adj = if dest >= to { dest - len } else { dest };

    // 5. open a gap at the destination (right, DESCENDING to avoid collisions)
    for tid in &track_ids {
        let mut after: Vec<(Id, TimeUs, TimeUs)> = plan
            .proj
            .track(*tid)
            .unwrap()
            .clips
            .iter()
            .filter(|c| c.start >= dest_adj)
            .map(|c| (c.id, c.start, c.duration))
            .collect();
        after.sort_by_key(|(_, start, _)| std::cmp::Reverse(*start));
        for (cid, start, duration) in after {
            plan.do_(Action::SetClipBounds {
                clip_id: cid,
                start: start + len,
                duration,
                src_in: None,
                src_out: None,
            })?;
        }
    }

    // 6. place the extracted clips back
    for (tid, mut clip) in moved {
        clip.start = dest_adj + (clip.start - from);
        plan.do_(Action::InsertClip { track_id: tid, clip })?;
    }
    Ok(plan.finish())
}

/// Removes time ranges from the WHOLE sequence (all unlocked tracks),
/// with optional ripple. The basis of "remove silences" and text editing.
/// Split every unlocked track at both edges of each range, removing nothing.
/// Silence workflow: the timeline ends up segmented into silence/speech clips
/// and the user decides what to delete, speed up or keep.
pub fn split_ranges(
    project: &Project,
    sequence_id: Id,
    ranges: &[(TimeUs, TimeUs)],
) -> UeResult<Vec<Action>> {
    let seq = project
        .sequence(sequence_id)
        .ok_or_else(|| UeError::NotFound(format!("sequence {sequence_id}")))?;
    let fps = seq.fps;
    let track_ids: Vec<Id> = seq.tracks.iter().filter(|t| !t.locked).map(|t| t.id).collect();
    let mut rs: Vec<(TimeUs, TimeUs)> = ranges
        .iter()
        .map(|(a, b)| (quantize_to_frame((*a).max(0), fps), quantize_to_frame(*b, fps)))
        .filter(|(a, b)| b > a)
        .collect();
    rs.sort();
    let mut plan = Planner::new(project);
    for (from, to) in rs {
        split_all_at(&mut plan, &track_ids, from)?;
        split_all_at(&mut plan, &track_ids, to)?;
    }
    Ok(plan.actions)
}

pub fn cut_ranges(
    project: &Project,
    sequence_id: Id,
    ranges: &[(TimeUs, TimeUs)],
    ripple: bool,
) -> UeResult<Vec<Action>> {
    let seq = project
        .sequence(sequence_id)
        .ok_or_else(|| UeError::NotFound(format!("sequence {sequence_id}")))?;

    // Normalize: sort and merge overlaps.
    let mut rs: Vec<(TimeUs, TimeUs)> = ranges
        .iter()
        .copied()
        .filter(|(a, b)| b > a)
        .map(|(a, b)| (quantize_to_frame(a.max(0), seq.fps), quantize_to_frame(b, seq.fps)))
        .filter(|(a, b)| b > a)
        .collect();
    rs.sort();
    let mut merged: Vec<(TimeUs, TimeUs)> = vec![];
    for r in rs {
        match merged.last_mut() {
            Some(last) if r.0 <= last.1 => last.1 = last.1.max(r.1),
            _ => merged.push(r),
        }
    }
    if merged.is_empty() {
        return Ok(vec![]);
    }

    let track_ids: Vec<Id> =
        seq.tracks.iter().filter(|t| !t.locked).map(|t| t.id).collect();

    let mut plan = Planner::new(project);
    for &(from, to) in &merged {
        for tid in &track_ids {
            carve_range(&mut plan, *tid, from, to, None)?;
        }
    }
    if ripple {
        for tid in &track_ids {
            let survivors: Vec<(Id, TimeUs, TimeUs)> = plan
                .proj
                .track(*tid)
                .unwrap()
                .clips
                .iter()
                .map(|c| (c.id, c.start, c.duration))
                .collect();
            for (cid, start, duration) in survivors {
                let shift: TimeUs = merged
                    .iter()
                    .filter(|(_, e)| *e <= start)
                    .map(|(s, e)| e - s)
                    .sum();
                if shift > 0 {
                    plan.do_(Action::SetClipBounds {
                        clip_id: cid,
                        start: start - shift,
                        duration,
                        src_in: None,
                        src_out: None,
                    })?;
                }
            }
        }
    }
    Ok(plan.finish())
}
