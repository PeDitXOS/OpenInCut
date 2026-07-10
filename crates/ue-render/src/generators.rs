//! Generators: procedural content clips (solids, gradients, …).
//! Same spirit as the effect packs: a manifest declares the params and a
//! lavfi SOURCE template; preview and export use the same chain.
//!
//! Reserved placeholders the renderer fills in: `{d}` duration in s,
//! `{fps}` frame rate. The rest comes from the manifest params (sizes are
//! rounded to an even integer).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{ParamDef, ParamKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub params: Vec<ParamDef>,
    /// lavfi source template with {keys}.
    pub source: String,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Embedded core generators.
pub fn core_generators() -> Vec<GeneratorDef> {
    const MANIFESTS: &[&str] = &[
        include_str!("../../../generators/core/solid/manifest.json"),
        include_str!("../../../generators/core/gradient/manifest.json"),
    ];
    MANIFESTS
        .iter()
        .map(|m| serde_json::from_str(m).expect("core generator manifest is valid"))
        .collect()
}

pub fn find_generator<'a>(registry: &'a [GeneratorDef], id: &str) -> Option<&'a GeneratorDef> {
    registry.iter().find(|d| d.id == id)
}

/// Renders a generator's lavfi source: substitutes params (with clamps and
/// manifest defaults), and the reserved {d}/{fps}.
pub fn render_generator(
    def: &GeneratorDef,
    params: &BTreeMap<String, ue_core::keyframe::Param>,
    color_params: &BTreeMap<String, String>,
    fps: (u32, u32),
    duration_us: i64,
) -> String {
    let mut out = def.source.clone();
    for p in &def.params {
        let placeholder = format!("{{{}}}", p.key);
        let value = match &p.kind {
            ParamKind::Float { default, min, max } => {
                let v = params
                    .get(&p.key)
                    .map(|param| param.eval(0))
                    .unwrap_or(*default)
                    .clamp(*min, *max);
                // sizes/coordinates of lavfi sources go as integers;
                // even to avoid fighting with yuv420p
                if p.key.contains("width") || p.key.contains("height") {
                    (((v.round() as i64) / 2) * 2).max(2).to_string()
                } else {
                    crate::format_float(v)
                }
            }
            ParamKind::Color { default } => {
                let hex = color_params.get(&p.key).map(String::as_str).unwrap_or(default);
                crate::format_color(hex)
                    .unwrap_or_else(|| crate::format_color(default).expect("valid default"))
            }
        };
        out = out.replace(&placeholder, &value);
    }
    let fps_s = format!("{}/{}", fps.0, fps.1);
    out = out.replace("{fps}", &fps_s);
    out = out.replace("{d}", &format!("{:.6}", duration_us as f64 / 1_000_000.0));
    out
}

/// Serializable catalog for the UI.
pub fn generators_catalog_json(registry: &[GeneratorDef]) -> serde_json::Value {
    serde_json::to_value(registry).expect("registry is serializable")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_generators_parse_and_render() {
        let gens = core_generators();
        assert!(gens.iter().any(|g| g.id == "core.solid"));
        assert!(gens.iter().any(|g| g.id == "core.gradient"));

        let solid = find_generator(&gens, "core.solid").unwrap();
        let mut colors = BTreeMap::new();
        colors.insert("color".to_string(), "#ff3355".to_string());
        let mut params = BTreeMap::new();
        params.insert("width".to_string(), 401.0.into()); // odd → even
        params.insert("height".to_string(), 200.0.into());
        let src = render_generator(solid, &params, &colors, (30, 1), 2_000_000);
        assert!(src.contains("color=c=0xFF3355"), "{src}");
        assert!(src.contains("s=400x200"), "{src}");
        assert!(src.contains("d=2.000000"), "{src}");
        assert!(src.contains("r=30/1"), "{src}");

        let grad = find_generator(&gens, "core.gradient").unwrap();
        let src = render_generator(grad, &BTreeMap::new(), &BTreeMap::new(), (30, 1), 1_000_000);
        assert!(src.starts_with("gradients="), "{src}");
        assert!(src.contains("duration=1.000000"), "{src}");
    }

    #[test]
    fn generator_color_invalid_falls_back_to_default() {
        let gens = core_generators();
        let solid = find_generator(&gens, "core.solid").unwrap();
        let mut colors = BTreeMap::new();
        colors.insert("color".to_string(), "not-a-color".to_string());
        let src = render_generator(solid, &BTreeMap::new(), &colors, (30, 1), 1_000_000);
        assert!(src.contains("0x"), "falls back to the manifest default: {src}");
    }
}
