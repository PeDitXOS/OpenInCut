//! Aplana el timeline de video a una EDL: lista de tramos consecutivos donde
//! cada tramo es o bien un rango de un asset (gana la pista superior) o negro.

use std::collections::BTreeSet;

use ue_core::model::{ClipPayload, Id, Project, TrackKind};
use ue_core::TimeUs;

use crate::{ExportError, ExportResult};

#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    /// Fuente lavfi generada (rectángulo, degradado, …), ya renderizada con
    /// su duración de ventana.
    Gen {
        source: String,
        duration: TimeUs,
        vf: Option<String>,
    },
    Source {
        asset_id: Id,
        src_in: TimeUs,
        src_out: TimeUs,
        /// Velocidad del clip (rate stretch): salida = fuente / speed.
        speed: f64,
        /// Cadena de efectos+transform del clip (ue-render), ya renderizada.
        vf: Option<String>,
        /// Transición con el tramo anterior: (duración de salida µs, effect_id).
        /// Los handles ya están extendidos en src_in/src_out por el post-pass.
        transition_in: Option<(TimeUs, String)>,
    },
    Black { duration: TimeUs },
}

impl Segment {
    /// Duración de SALIDA del tramo (la fuente dividida por la velocidad).
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

/// Construye la EDL con solo los packs core (atajo para tests y usos simples).
pub fn build_video_edl(project: &Project, sequence_id: Id) -> ExportResult<Vec<Segment>> {
    build_video_edl_with(project, sequence_id, &[])
}

/// Construye la EDL de video de la secuencia (packs core + `extra_packs`).
/// Error si hay speed != 1 en un clip visible o si el timeline está vacío.
pub fn build_video_edl_with(
    project: &Project,
    sequence_id: Id,
    extra_packs: &[ue_render::EffectDef],
) -> ExportResult<Vec<Segment>> {
    let seq = project
        .sequence(sequence_id)
        .ok_or(ExportError::NoSequence(sequence_id))?;

    // fronteras: inicios y finales de todos los clips media de pistas de video visibles
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

    // por tramo, resolver el clip visible (pista superior gana)
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
                        // la transición pertenece al PRIMER tramo del clip
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

    // recortar negro final (después del último tramo con contenido)
    while matches!(segments.last(), Some(Segment::Black { .. })) {
        segments.pop();
    }
    if segments.is_empty() {
        return Err(ExportError::EmptyTimeline);
    }

    // fusionar tramos contiguos del mismo asset con fuente continua, mismos
    // efectos y sin transición de por medio
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

/// Duración total de SALIDA de la EDL en µs (los crossfades solapan material,
/// así que restan su duración).
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

/// Post-pass de transiciones: valida que haya un tramo Source contiguo antes,
/// extiende los handles (mitad a cada lado, limitado por el material del
/// archivo) y reduce o elimina la transición si no hay material suficiente.
fn apply_transition_handles(project: &Project, segments: &mut [Segment]) {
    const MIN_TRANSITION: TimeUs = 40_000; // por debajo de ~1 frame no vale la pena
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
        // sin tramo Source justo antes → sin transición
        if i == 0 {
            if let Segment::Source { transition_in, .. } = &mut segments[i] {
                *transition_in = None;
            }
            continue;
        }
        // disponibilidad en TIEMPO DE SALIDA (asset / speed de cada lado)
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
