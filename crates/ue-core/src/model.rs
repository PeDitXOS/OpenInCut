//! Modelo de datos del proyecto (sección 4 del PLAN).

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::keyframe::Param;
use crate::time::TimeUs;

pub type Id = Ulid;
pub const SCHEMA_VERSION: u32 = 1;

fn new_id() -> Id {
    Ulid::new()
}

// ---------------------------------------------------------------------------
// Proyecto
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub id: Id,
    pub name: String,
    pub created_at: String,
    #[serde(default)]
    pub settings: ProjectSettings,
    #[serde(default)]
    pub assets: Vec<MediaAsset>,
    #[serde(default)]
    pub transcripts: Vec<TranscriptDoc>,
    pub sequences: Vec<Sequence>,
    pub active_sequence: Id,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSettings {
    #[serde(default = "default_lang")]
    pub whisper_language: String,
    #[serde(default = "default_autosave")]
    pub autosave_secs: u32,
}

fn default_lang() -> String {
    "auto".into()
}
fn default_autosave() -> u32 {
    60
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self { whisper_language: default_lang(), autosave_secs: default_autosave() }
    }
}

impl Project {
    /// Proyecto nuevo con una secuencia 1920x1080@30 y pistas V1/A1.
    pub fn new(name: &str) -> Self {
        let seq = Sequence::new("Principal", (1920, 1080), (30, 1));
        let seq_id = seq.id;
        Project {
            schema_version: SCHEMA_VERSION,
            id: new_id(),
            name: name.to_string(),
            created_at: String::new(), // la capa de app la rellena (ue-core no toca el reloj)
            settings: ProjectSettings::default(),
            assets: vec![],
            transcripts: vec![],
            sequences: vec![seq],
            active_sequence: seq_id,
        }
    }

    pub fn sequence(&self, id: Id) -> Option<&Sequence> {
        self.sequences.iter().find(|s| s.id == id)
    }
    pub fn sequence_mut(&mut self, id: Id) -> Option<&mut Sequence> {
        self.sequences.iter_mut().find(|s| s.id == id)
    }
    pub fn asset(&self, id: Id) -> Option<&MediaAsset> {
        self.assets.iter().find(|a| a.id == id)
    }

    /// Localiza un clip por id: (seq_idx, track_idx, clip_idx).
    pub fn locate_clip(&self, id: Id) -> Option<(usize, usize, usize)> {
        for (si, s) in self.sequences.iter().enumerate() {
            for (ti, t) in s.tracks.iter().enumerate() {
                if let Some(ci) = t.clips.iter().position(|c| c.id == id) {
                    return Some((si, ti, ci));
                }
            }
        }
        None
    }

    pub fn clip(&self, id: Id) -> Option<&Clip> {
        let (si, ti, ci) = self.locate_clip(id)?;
        Some(&self.sequences[si].tracks[ti].clips[ci])
    }

    pub fn track(&self, id: Id) -> Option<&Track> {
        self.sequences.iter().flat_map(|s| s.tracks.iter()).find(|t| t.id == id)
    }
    pub fn track_mut(&mut self, id: Id) -> Option<&mut Track> {
        self.sequences.iter_mut().flat_map(|s| s.tracks.iter_mut()).find(|t| t.id == id)
    }

    /// Secuencia que contiene la pista.
    pub fn sequence_of_track(&self, track_id: Id) -> Option<&Sequence> {
        self.sequences.iter().find(|s| s.tracks.iter().any(|t| t.id == track_id))
    }
}

// ---------------------------------------------------------------------------
// Media
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeInfo {
    pub duration_us: TimeUs,
    #[serde(default)]
    pub fps: Option<(u32, u32)>,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub rotation: i32,
    #[serde(default)]
    pub vcodec: Option<String>,
    #[serde(default)]
    pub acodec: Option<String>,
    #[serde(default)]
    pub audio_channels: u32,
    #[serde(default)]
    pub vfr: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaAsset {
    pub id: Id,
    pub kind: MediaKind,
    /// Ruta relativa al archivo .uep (o absoluta mientras no se guarde).
    pub path: String,
    pub content_hash: String,
    pub probe: ProbeInfo,
    #[serde(default)]
    pub proxy: Option<String>,
    #[serde(default)]
    pub audio_conform: Option<String>,
    #[serde(default)]
    pub peaks: Option<String>,
    #[serde(default)]
    pub thumbnails: Option<String>,
    #[serde(default)]
    pub transcript: Option<Id>,
    #[serde(default)]
    pub offline: bool,
}

// ---------------------------------------------------------------------------
// Secuencia / pistas / clips
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sequence {
    pub id: Id,
    pub name: String,
    pub resolution: (u32, u32),
    pub fps: (u32, u32),
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub markers: Vec<Marker>,
}

fn default_sample_rate() -> u32 {
    48000
}

impl Sequence {
    pub fn new(name: &str, resolution: (u32, u32), fps: (u32, u32)) -> Self {
        Sequence {
            id: new_id(),
            name: name.to_string(),
            resolution,
            fps,
            sample_rate: 48000,
            tracks: vec![
                Track::new(TrackKind::Audio, "A1"),
                Track::new(TrackKind::Video, "V1"),
            ],
            markers: vec![],
        }
    }

    /// Duración = fin del último clip entre todas las pistas.
    pub fn duration_us(&self) -> TimeUs {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.start + c.duration)
            .max()
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Video,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: Id,
    pub kind: TrackKind,
    pub name: String,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub solo: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub volume_db: f32,
    /// SIEMPRE ordenados por `start` y sin solaparse (invariante validado).
    #[serde(default)]
    pub clips: Vec<Clip>,
}

impl Track {
    pub fn new(kind: TrackKind, name: &str) -> Self {
        Track {
            id: new_id(),
            kind,
            name: name.to_string(),
            muted: false,
            solo: false,
            locked: false,
            volume_db: 0.0,
            clips: vec![],
        }
    }

    pub fn clip_index(&self, id: Id) -> Option<usize> {
        self.clips.iter().position(|c| c.id == id)
    }

    /// Índice de inserción manteniendo orden por start.
    pub fn insertion_index(&self, start: TimeUs) -> usize {
        self.clips.partition_point(|c| c.start < start)
    }

    /// ¿Un clip [start, start+dur) colisiona con los existentes (excluyendo `exclude`)?
    pub fn collides(&self, start: TimeUs, duration: TimeUs, exclude: Option<Id>) -> bool {
        let end = start + duration;
        self.clips.iter().any(|c| {
            Some(c.id) != exclude && c.start < end && start < c.start + c.duration
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub id: Id,
    pub t: TimeUs,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
}

// ---------------------------------------------------------------------------
// Clip
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: Id,
    pub payload: ClipPayload,
    pub start: TimeUs,
    pub duration: TimeUs,
    #[serde(default = "default_speed")]
    pub speed: f64,
    #[serde(default)]
    pub effects: Vec<EffectInstance>,
    #[serde(default)]
    pub transform: Transform2D,
    #[serde(default)]
    pub audio: AudioProps,
    #[serde(default)]
    pub transition_in: Option<TransitionRef>,
    #[serde(default)]
    pub label_color: Option<String>,
}

fn default_speed() -> f64 {
    1.0
}

impl Clip {
    pub fn new_media(asset_id: Id, src_in: TimeUs, src_out: TimeUs, start: TimeUs) -> Self {
        Clip {
            id: new_id(),
            payload: ClipPayload::Media { asset_id, src_in, src_out },
            start,
            duration: src_out - src_in,
            speed: 1.0,
            effects: vec![],
            transform: Transform2D::default(),
            audio: AudioProps::default(),
            transition_in: None,
            label_color: None,
        }
    }

    pub fn new_text(content: &str, start: TimeUs, duration: TimeUs) -> Self {
        Clip {
            id: new_id(),
            payload: ClipPayload::Text { content: content.to_string(), style: TextStyle::default() },
            start,
            duration,
            speed: 1.0,
            effects: vec![],
            transform: Transform2D::default(),
            audio: AudioProps::default(),
            transition_in: None,
            label_color: None,
        }
    }

    pub fn end(&self) -> TimeUs {
        self.start + self.duration
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ClipPayload {
    Media {
        asset_id: Id,
        /// Rango del archivo fuente en µs.
        src_in: TimeUs,
        src_out: TimeUs,
    },
    Text {
        content: String,
        style: TextStyle,
    },
    Subtitles {
        transcript_id: Id,
        style: TextStyle,
        mode: SubtitleMode,
    },
    Solid {
        color: [f32; 4],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleMode {
    Phrase,
    Word,
    Karaoke,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    #[serde(default = "default_font")]
    pub font: String,
    #[serde(default = "default_font_size")]
    pub size: f32,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub bg: Option<String>,
    #[serde(default)]
    pub stroke_color: Option<String>,
    #[serde(default)]
    pub stroke_width: f32,
    #[serde(default)]
    pub highlight_color: Option<String>,
    #[serde(default)]
    pub y_offset: f32,
    #[serde(default)]
    pub align: TextAlign,
}

fn default_font() -> String {
    "sans-serif".into()
}
fn default_font_size() -> f32 {
    60.0
}
fn default_color() -> String {
    "#ffffff".into()
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font: default_font(),
            size: default_font_size(),
            color: default_color(),
            bg: None,
            stroke_color: None,
            stroke_width: 0.0,
            highlight_color: None,
            y_offset: 0.0,
            align: TextAlign::Center,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transform2D {
    pub position: (Param, Param),
    pub scale: (Param, Param),
    /// Grados.
    pub rotation: Param,
    /// Crop left/top/right/bottom en fracción [0, 1].
    pub crop: (Param, Param, Param, Param),
    pub opacity: Param,
    #[serde(default)]
    pub flip_h: bool,
    #[serde(default)]
    pub flip_v: bool,
}

impl Default for Transform2D {
    fn default() -> Self {
        Self {
            position: (0.0.into(), 0.0.into()),
            scale: (1.0.into(), 1.0.into()),
            rotation: 0.0.into(),
            crop: (0.0.into(), 0.0.into(), 0.0.into(), 0.0.into()),
            opacity: 1.0.into(),
            flip_h: false,
            flip_v: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioProps {
    pub gain_db: Param,
    pub pan: Param,
    #[serde(default)]
    pub fade_in_us: TimeUs,
    #[serde(default)]
    pub fade_out_us: TimeUs,
    #[serde(default)]
    pub muted: bool,
}

impl Default for AudioProps {
    fn default() -> Self {
        Self {
            gain_db: 0.0.into(),
            pan: 0.0.into(),
            fade_in_us: 0,
            fade_out_us: 0,
            muted: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectInstance {
    pub effect_id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub params: std::collections::BTreeMap<String, Param>,
    /// Parámetros de color ("#rrggbb"), separados de los numéricos/curvas.
    #[serde(default)]
    pub color_params: std::collections::BTreeMap<String, String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransitionRef {
    pub effect_id: String,
    pub duration: TimeUs,
    #[serde(default)]
    pub params: std::collections::BTreeMap<String, Param>,
}

// ---------------------------------------------------------------------------
// Transcripción (word-level)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptDoc {
    pub id: Id,
    pub asset_id: Id,
    pub language: String,
    pub model: String,
    #[serde(default)]
    pub words: Vec<Word>,
    #[serde(default)]
    pub segments: Vec<Segment>,
    #[serde(default)]
    pub global_avg_volume: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    pub start_us: TimeUs,
    pub end_us: TimeUs,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub rejected: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub text: String,
    pub start_us: TimeUs,
    pub end_us: TimeUs,
    /// Rango [desde, hasta) en `words`.
    pub word_range: (usize, usize),
    #[serde(default)]
    pub emotion: Option<String>,
    #[serde(default)]
    pub volume_rms: f64,
}

// ---------------------------------------------------------------------------
// (De)serialización del proyecto
// ---------------------------------------------------------------------------

impl Project {
    pub fn to_json(&self) -> Result<String, crate::UeError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(s: &str) -> Result<Project, crate::UeError> {
        #[derive(Deserialize)]
        struct VersionProbe {
            schema_version: u32,
        }
        let v: VersionProbe = serde_json::from_str(s)?;
        if v.schema_version > SCHEMA_VERSION {
            return Err(crate::UeError::SchemaVersion(v.schema_version, SCHEMA_VERSION));
        }
        // Aquí irán las migraciones v1→v2… cuando existan.
        Ok(serde_json::from_str(s)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_json_roundtrip() {
        let mut p = Project::new("Demo");
        let seq_id = p.active_sequence;
        let track_id = p.sequence(seq_id).unwrap().tracks[1].id;
        let asset = MediaAsset {
            id: Ulid::new(),
            kind: MediaKind::Video,
            path: "media/a.mp4".into(),
            content_hash: "xxh3:abc".into(),
            probe: ProbeInfo {
                duration_us: 10_000_000,
                fps: Some((30, 1)),
                width: 1920,
                height: 1080,
                rotation: 0,
                vcodec: Some("h264".into()),
                acodec: Some("aac".into()),
                audio_channels: 2,
                vfr: false,
            },
            proxy: None,
            audio_conform: None,
            peaks: None,
            thumbnails: None,
            transcript: None,
            offline: false,
        };
        let clip = Clip::new_media(asset.id, 0, 5_000_000, 0);
        p.assets.push(asset);
        p.sequence_mut(seq_id).unwrap();
        p.track_mut(track_id).unwrap().clips.push(clip);

        let json = p.to_json().unwrap();
        let back = Project::from_json(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn future_schema_rejected() {
        let p = Project::new("X");
        let mut val: serde_json::Value = serde_json::from_str(&p.to_json().unwrap()).unwrap();
        val["schema_version"] = serde_json::json!(999);
        let err = Project::from_json(&val.to_string()).unwrap_err();
        assert!(matches!(err, crate::UeError::SchemaVersion(999, _)));
    }

    #[test]
    fn collision_detection() {
        let mut t = Track::new(TrackKind::Video, "V1");
        let a = Ulid::new();
        t.clips.push(Clip::new_media(a, 0, 1_000_000, 0)); // [0, 1s)
        assert!(t.collides(500_000, 1_000_000, None));
        assert!(!t.collides(1_000_000, 1_000_000, None)); // adyacente exacto no colisiona
        let id = t.clips[0].id;
        assert!(!t.collides(0, 1_000_000, Some(id))); // excluyéndose a sí mismo
    }
}
