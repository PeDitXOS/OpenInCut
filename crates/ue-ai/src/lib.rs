//! ue-ai: análisis de contenido. v0: detección de silencios (PLAN §7.C).
//! Port mejorado del `trim_by_silence` del Youtubers-toolkit: ventanas RMS
//! finas, umbral dual con histéresis, duraciones mínimas y padding.

pub mod emotion;
pub mod silence;
