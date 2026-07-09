//! ue-render: sistema modular de efectos (PLAN §6.5).
//!
//! v0: cada efecto es un pack con manifest.json que declara parámetros y una
//! plantilla de filtro ffmpeg — el mismo efecto se aplica en preview (sesión
//! MJPEG / render_frame) y en export (filter_complex), así que preview==export.
//! El motor wgpu (WGSL) sustituirá el backend de render manteniendo manifests
//! y parámetros; los packs de usuario en disco llegan con el hot-reload.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ue_core::model::EffectInstance;

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("manifest inválido: {0}")]
    Manifest(String),
}

// ---------------------------------------------------------------------------
// Definición de efectos (manifest)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ParamKind {
    Float {
        default: f64,
        min: f64,
        max: f64,
    },
    Color {
        default: String, // "#rrggbb"
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamDef {
    pub key: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(flatten)]
    pub kind: ParamKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub params: Vec<ParamDef>,
    /// Plantilla de cadena -vf con {claves} a sustituir.
    pub ffmpeg: String,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Packs core embebidos en el binario. Los packs de usuario (carpeta en disco
/// con hot-reload) se añadirán sobre esta base.
pub fn core_registry() -> Vec<EffectDef> {
    const MANIFESTS: &[&str] = &[
        include_str!("../../../effects/core/color_correct/manifest.json"),
        include_str!("../../../effects/core/chroma_key/manifest.json"),
        include_str!("../../../effects/core/gaussian_blur/manifest.json"),
    ];
    MANIFESTS
        .iter()
        .map(|m| serde_json::from_str(m).expect("manifest core válido (verificado por tests)"))
        .collect()
}

pub fn find_effect<'a>(registry: &'a [EffectDef], id: &str) -> Option<&'a EffectDef> {
    registry.iter().find(|d| d.id == id)
}

// ---------------------------------------------------------------------------
// Renderizado de la cadena ffmpeg (backend v0)
// ---------------------------------------------------------------------------

fn format_float(v: f64) -> String {
    // sin notación científica y sin ceros infinitos
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// "#rrggbb" → "0xRRGGBB" (sintaxis de color de ffmpeg). Valores raros caen al default.
fn format_color(hex: &str) -> Option<String> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("0x{}", h.to_uppercase()))
}

/// Sustituye los parámetros de una instancia en la plantilla de su definición.
/// Parámetros ausentes usan el default del manifest; los float se clampean al rango.
pub fn render_effect(def: &EffectDef, inst: &EffectInstance) -> String {
    let mut out = def.ffmpeg.clone();
    for p in &def.params {
        let placeholder = format!("{{{}}}", p.key);
        let value = match &p.kind {
            ParamKind::Float { default, min, max } => {
                let v = inst
                    .params
                    .get(&p.key)
                    .map(|param| param.eval(0))
                    .unwrap_or(*default)
                    .clamp(*min, *max);
                format_float(v)
            }
            ParamKind::Color { default } => {
                let hex = inst.color_params.get(&p.key).map(String::as_str).unwrap_or(default);
                format_color(hex).unwrap_or_else(|| format_color(default).expect("default válido"))
            }
        };
        out = out.replace(&placeholder, &value);
    }
    out
}

/// Cadena -vf completa para los efectos habilitados de un clip (None si no hay).
pub fn render_chain(registry: &[EffectDef], effects: &[EffectInstance]) -> Option<String> {
    let parts: Vec<String> = effects
        .iter()
        .filter(|e| e.enabled)
        .filter_map(|e| find_effect(registry, &e.effect_id).map(|d| render_effect(d, e)))
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(","))
    }
}

/// Catálogo serializable para la UI / MCP.
pub fn catalog_json(registry: &[EffectDef]) -> serde_json::Value {
    serde_json::to_value(registry).expect("registry serializable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ue_core::model::EffectInstance;

    fn inst(id: &str, params: &[(&str, f64)], colors: &[(&str, &str)]) -> EffectInstance {
        EffectInstance {
            effect_id: id.into(),
            enabled: true,
            params: params.iter().map(|(k, v)| (k.to_string(), (*v).into())).collect(),
            color_params: colors
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn core_manifests_parse() {
        let reg = core_registry();
        assert!(reg.len() >= 3);
        assert!(find_effect(&reg, "core.chroma_key").is_some());
        assert!(find_effect(&reg, "core.color_correct").is_some());
    }

    #[test]
    fn render_with_defaults_and_overrides() {
        let reg = core_registry();
        let def = find_effect(&reg, "core.color_correct").unwrap();
        // solo brightness cambiado; el resto usa defaults
        let e = inst("core.color_correct", &[("brightness", 0.25)], &[]);
        let s = render_effect(def, &e);
        assert_eq!(s, "eq=brightness=0.25:contrast=1:saturation=1:gamma=1");
    }

    #[test]
    fn float_params_are_clamped() {
        let reg = core_registry();
        let def = find_effect(&reg, "core.color_correct").unwrap();
        let e = inst("core.color_correct", &[("brightness", 99.0)], &[]);
        assert!(render_effect(def, &e).contains("brightness=1"));
    }

    #[test]
    fn color_param_formats_and_falls_back() {
        let reg = core_registry();
        let def = find_effect(&reg, "core.chroma_key").unwrap();
        let e = inst("core.chroma_key", &[], &[("key_color", "#12abEF")]);
        assert!(render_effect(def, &e).contains("color=0x12ABEF"));
        // color corrupto → default del manifest
        let bad = inst("core.chroma_key", &[], &[("key_color", "verde;rm -rf")]);
        assert!(render_effect(def, &bad).contains("color=0x00FF00"));
    }

    #[test]
    fn chain_joins_enabled_only() {
        let reg = core_registry();
        let mut off = inst("core.gaussian_blur", &[("sigma", 5.0)], &[]);
        off.enabled = false;
        let on = inst("core.color_correct", &[("saturation", 1.5)], &[]);
        let chain = render_chain(&reg, &[off, on]).unwrap();
        assert!(!chain.contains("gblur"));
        assert!(chain.contains("saturation=1.5"));
        assert!(render_chain(&reg, &[]).is_none());
        // efecto desconocido se ignora sin romper
        let unknown = inst("user.no_existe", &[], &[]);
        assert!(render_chain(&reg, &[unknown]).is_none());
    }
}
