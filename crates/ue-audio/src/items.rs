//! Building the MixItems from the project: which clips play, with
//! what gain and from which conformed WAV. (Mirror of the export logic.)

use std::path::{Path, PathBuf};

use ue_core::keyframe::{KeyframeCurve, Param};
use ue_core::model::{ClipPayload, Id, MediaAsset, Project};

use crate::mixer::{db_to_linear, MixItem};
use crate::us_to_frames;
use crate::wav::WavMap;

/// Spec computed before opening files (pure, testable without IO).
#[derive(Debug, PartialEq)]
pub struct ItemSpec {
    pub asset_id: Id,
    pub timeline_start_us: i64,
    pub src_in_us: i64,
    /// Duration in TIMELINE TIME (source already divided by speed).
    pub len_us: i64,
    pub speed: f64,
    /// Static part of the gain (clip const + track volume).
    pub gain_db: f64,
    /// Gain curve in dB (if the clip's gain is animated).
    pub gain_curve: Option<KeyframeCurve>,
    /// Pan -1..1 (static; evaluated at t=0).
    pub pan: f64,
    pub fade_in_us: i64,
    pub fade_out_us: i64,
}

/// Collects the audible clips (audio and video tracks; respects mute/solo).
pub fn collect_specs(project: &Project, sequence_id: Id) -> Vec<ItemSpec> {
    let Some(seq) = project.sequence(sequence_id) else { return vec![] };
    let any_solo = seq.tracks.iter().any(|t| t.solo);
    let mut specs = vec![];
    for track in &seq.tracks {
        if track.muted || (any_solo && !track.solo) {
            continue;
        }
        for clip in &track.clips {
            if clip.audio.muted {
                continue;
            }
            let ClipPayload::Media { asset_id, src_in, src_out } = &clip.payload else {
                continue;
            };
            let Some(asset) = project.asset(*asset_id) else { continue };
            if asset.probe.audio_channels == 0 {
                continue;
            }
            let src_len_tl = (((*src_out - *src_in) as f64) / clip.speed).round() as i64;
            let (gain_const, gain_curve) = match &clip.audio.gain_db {
                Param::Const(v) => (*v, None),
                Param::Curve(c) => (0.0, Some(c.clone())),
            };
            specs.push(ItemSpec {
                asset_id: *asset_id,
                timeline_start_us: clip.start,
                src_in_us: *src_in,
                len_us: src_len_tl.min(clip.duration),
                speed: clip.speed,
                gain_db: gain_const + track.volume_db as f64,
                gain_curve,
                pan: clip.audio.pan.eval(0).clamp(-1.0, 1.0),
                fade_in_us: clip.audio.fade_in_us,
                fade_out_us: clip.audio.fade_out_us,
            });
        }
    }
    specs
}

/// Opens the conformed WAVs and produces the MixItems ready for the mixer.
/// Assets without an available conform are skipped (and reported).
pub fn load_items(
    project: &Project,
    specs: &[ItemSpec],
    conform_path: impl Fn(&MediaAsset) -> Option<PathBuf>,
) -> (Vec<MixItem>, Vec<Id>) {
    let mut items = vec![];
    let mut skipped = vec![];
    for spec in specs {
        let Some(asset) = project.asset(spec.asset_id) else {
            skipped.push(spec.asset_id);
            continue;
        };
        let Some(path) = conform_path(asset) else {
            skipped.push(spec.asset_id);
            continue;
        };
        match WavMap::open(Path::new(&path)) {
            Ok(wav) => items.push(MixItem {
                wav,
                timeline_start: us_to_frames(spec.timeline_start_us),
                src_in: us_to_frames(spec.src_in_us),
                len: us_to_frames(spec.len_us),
                speed: spec.speed,
                gain: db_to_linear(spec.gain_db),
                gain_curve: spec.gain_curve.clone(),
                pan: spec.pan as f32,
                fade_in: us_to_frames(spec.fade_in_us),
                fade_out: us_to_frames(spec.fade_out_us),
            }),
            Err(_) => skipped.push(spec.asset_id),
        }
    }
    (items, skipped)
}
