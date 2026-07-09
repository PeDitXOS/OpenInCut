//! Generadores: clips de contenido procedural (sólidos, degradados, …).
//! Mismo espíritu que los packs de efectos: un manifest declara los params y
//! una plantilla de FUENTE lavfi; preview y export usan la misma cadena.
//!
//! Placeholders reservados que rellena el renderer: `{d}` duración en s,
//! `{fps}` frame rate. El resto sale de los params del manifest (los tamaños
//! se redondean a entero par).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{ParamDef, ParamKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub params: Vec<ParamDef>,
    /// Plantilla de fuente lavfi con {claves}.
    pub source: String,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Generadores core embebidos.
pub fn core_generators() -> Vec<GeneratorDef> {
    const MANIFESTS: &[&str] = &[
        include_str!("../../../generators/core/solid/manifest.json"),
        include_str!("../../../generators/core/gradient/manifest.json"),
    ];
    MANIFESTS
        .iter()
        .map(|m| serde_json::from_str(m).expect("manifest core de generador válido"))
        .collect()
}

pub fn find_generator<'a>(registry: &'a [GeneratorDef], id: &str) -> Option<&'a GeneratorDef> {
    registry.iter().find(|d| d.id == id)
}

/// Renderiza la fuente lavfi de un generador: sustituye params (con clamps y
/// defaults del manifest), y los reservados {d}/{fps}.
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
                // los tamaños/coordenadas de fuentes lavfi van como enteros;
                // par para no pelear con yuv420p
                if p.key.contains("width") || p.key.contains("height") {
                    (((v.round() as i64) / 2) * 2).max(2).to_string()
                } else {
                    crate::format_float(v)
                }
            }
            ParamKind::Color { default } => {
                let hex = color_params.get(&p.key).map(String::as_str).unwrap_or(default);
                crate::format_color(hex)
                    .unwrap_or_else(|| crate::format_color(default).expect("default válido"))
            }
        };
        out = out.replace(&placeholder, &value);
    }
    let fps_s = format!("{}/{}", fps.0, fps.1);
    out = out.replace("{fps}", &fps_s);
    out = out.replace("{d}", &format!("{:.6}", duration_us as f64 / 1_000_000.0));
    out
}

/// Catálogo serializable para la UI.
pub fn generators_catalog_json(registry: &[GeneratorDef]) -> serde_json::Value {
    serde_json::to_value(registry).expect("registry serializable")
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
        params.insert("width".to_string(), 401.0.into()); // impar → par
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
        colors.insert("color".to_string(), "no-es-color".to_string());
        let src = render_generator(solid, &BTreeMap::new(), &colors, (30, 1), 1_000_000);
        assert!(src.contains("0x"), "cae al default del manifest: {src}");
    }
}
