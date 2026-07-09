//! Aplana el timeline de video a una EDL: lista de tramos consecutivos donde
//! cada tramo es o bien un rango de un asset (gana la pista superior) o negro.

use std::collections::BTreeSet;

use ue_core::model::{ClipPayload, Id, Project, TrackKind};
use ue_core::TimeUs;

use crate::{ExportError, ExportResult};

#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    Source {
        asset_id: Id,
        src_in: TimeUs,
        src_out: TimeUs,
        /// Cadena de efectos del clip (ue-render), ya renderizada.
        vf: Option<String>,
    },
    Black { duration: TimeUs },
}

impl Segment {
    pub fn duration(&self) -> TimeUs {
        match self {
            Segment::Source { src_in, src_out, .. } => src_out - src_in,
            Segment::Black { duration } => *duration,
        }
    }
}

/// Registro de efectos usado por la EDL (core embebido).
fn effects_registry() -> Vec<ue_render::EffectDef> {
    ue_render::core_registry()
}

/// Construye la EDL de video de la secuencia. Error si hay speed != 1 en un
/// clip visible o si el timeline está vacío.
pub fn build_video_edl(project: &Project, sequence_id: Id) -> ExportResult<Vec<Segment>> {
    let seq = project
        .sequence(sequence_id)
        .ok_or(ExportError::NoSequence(sequence_id))?;

    // fronteras: inicios y finales de todos los clips media de pistas de video visibles
    let mut cuts: BTreeSet<TimeUs> = BTreeSet::new();
    cuts.insert(0);
    for track in seq.tracks.iter().filter(|t| t.kind == TrackKind::Video && !t.muted) {
        for clip in &track.clips {
            if let ClipPayload::Media { .. } = clip.payload {
                if (clip.speed - 1.0).abs() > 1e-9 {
                    return Err(ExportError::SpeedUnsupported(clip.speed));
                }
                cuts.insert(clip.start);
                cuts.insert(clip.end());
            }
        }
    }
    let cuts: Vec<TimeUs> = cuts.into_iter().collect();
    if cuts.len() < 2 {
        return Err(ExportError::EmptyTimeline);
    }

    // por tramo, resolver el clip visible (pista superior gana)
    let registry = effects_registry();
    let mut segments: Vec<Segment> = vec![];
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let mid = a + (b - a) / 2;
        let mut found: Option<Segment> = None;
        for track in seq.tracks.iter().rev().filter(|t| t.kind == TrackKind::Video && !t.muted) {
            for clip in &track.clips {
                if clip.start <= mid && mid < clip.end() {
                    if let ClipPayload::Media { asset_id, src_in, .. } = &clip.payload {
                        if project.asset(*asset_id).is_none() {
                            return Err(ExportError::MissingAsset(*asset_id));
                        }
                        let s_in = *src_in + (a - clip.start);
                        found = Some(Segment::Source {
                            asset_id: *asset_id,
                            src_in: s_in,
                            src_out: s_in + (b - a),
                            vf: ue_render::render_chain(&registry, &clip.effects),
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

    // recortar negro final (después del último tramo con contenido)
    while matches!(segments.last(), Some(Segment::Black { .. })) {
        segments.pop();
    }
    if segments.is_empty() {
        return Err(ExportError::EmptyTimeline);
    }

    // fusionar tramos contiguos del mismo asset con fuente continua y mismos efectos
    let mut merged: Vec<Segment> = vec![];
    for seg in segments {
        match (merged.last_mut(), &seg) {
            (
                Some(Segment::Source { asset_id: a1, src_out: o1, vf: v1, .. }),
                Segment::Source { asset_id: a2, src_in: i2, src_out: o2, vf: v2 },
            ) if a1 == a2 && o1 == i2 && v1 == v2 => *o1 = *o2,
            (Some(Segment::Black { duration: d1 }), Segment::Black { duration: d2 }) => {
                *d1 += *d2;
            }
            _ => merged.push(seg),
        }
    }
    Ok(merged)
}

/// Duración total de la EDL en µs.
pub fn edl_duration(segments: &[Segment]) -> TimeUs {
    segments.iter().map(|s| s.duration()).sum()
}
