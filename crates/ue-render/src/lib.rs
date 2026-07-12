//! ue-render: modular effects system (PLAN §6.5).
//!
//! v0: each effect is a pack with a manifest.json that declares parameters and
//! an ffmpeg filter template — the same effect is applied in preview (MJPEG
//! session / render_frame) and in export (filter_complex), so preview==export.
//! The wgpu (WGSL) engine will replace the render backend while keeping
//! manifests and parameters; user packs on disk arrive with hot-reload.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ue_core::model::EffectInstance;

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("invalid manifest: {0}")]
    Manifest(String),
}

// ---------------------------------------------------------------------------
// Effect definition (manifest)
// ---------------------------------------------------------------------------

pub mod generators;
pub use generators::{
    core_generators, find_generator, generators_catalog_json, render_generator, GeneratorDef,
};

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
    /// Template for the -vf chain with {keys} to substitute.
    pub ffmpeg: String,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Core packs embedded in the binary. User packs (on-disk folder with
/// hot-reload) are added on top of this base.
pub fn core_registry() -> Vec<EffectDef> {
    const MANIFESTS: &[&str] = &[
        include_str!("../../../effects/core/color_correct/manifest.json"),
        include_str!("../../../effects/core/chroma_key/manifest.json"),
        include_str!("../../../effects/core/gaussian_blur/manifest.json"),
        include_str!("../../../effects/core/vertical_fill/manifest.json"),
        include_str!("../../../effects/core/drop_shadow/manifest.json"),
    ];
    MANIFESTS
        .iter()
        .map(|m| serde_json::from_str(m).expect("core manifest is valid (verified by tests)"))
        .collect()
}

pub fn find_effect<'a>(registry: &'a [EffectDef], id: &str) -> Option<&'a EffectDef> {
    registry.iter().find(|d| d.id == id)
}

/// Loads user packs: each subfolder of `dir` with a manifest.json.
/// Invalid manifests break nothing: they are reported as readable errors.
pub fn load_packs_from_dir(dir: &std::path::Path) -> (Vec<EffectDef>, Vec<String>) {
    let mut defs = vec![];
    let mut errors = vec![];
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (defs, errors);
    };
    for entry in entries.flatten() {
        let manifest = entry.path().join("manifest.json");
        if !manifest.is_file() {
            continue;
        }
        match std::fs::read_to_string(&manifest) {
            Ok(s) => match serde_json::from_str::<EffectDef>(&s) {
                Ok(d) => defs.push(d),
                Err(e) => errors.push(format!("{}: {e}", manifest.display())),
            },
            Err(e) => errors.push(format!("{}: {e}", manifest.display())),
        }
    }
    defs.sort_by(|a, b| a.id.cmp(&b.id));
    (defs, errors)
}

/// core + user; on an id conflict the user pack wins.
pub fn merge_registries(core: Vec<EffectDef>, user: Vec<EffectDef>) -> Vec<EffectDef> {
    let mut out = core;
    for u in user {
        if let Some(slot) = out.iter_mut().find(|d| d.id == u.id) {
            *slot = u;
        } else {
            out.push(u);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Rendering the ffmpeg chain (v0 backend)
// ---------------------------------------------------------------------------

pub(crate) fn format_float(v: f64) -> String {
    // no scientific notation and no trailing zeros
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// "#rrggbb" → "0xRRGGBB" (ffmpeg color syntax). Odd values fall back to the default.
pub(crate) fn format_color(hex: &str) -> Option<String> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("0x{}", h.to_uppercase()))
}

/// Substitutes an instance's parameters into its definition's template.
/// Missing parameters use the manifest default; floats are clamped to the range.
/// `{u}` is replaced by a unique counter (for templates with split/overlay
/// mini-graphs: labels don't collide between uses in the same ffmpeg).
pub fn render_effect(def: &EffectDef, inst: &EffectInstance) -> String {
    render_effect_at(def, inst, 0)
}

/// Like `render_effect`, evaluating animated params at `at_us` (clip-relative).
/// Full time-expression animation for effects (sendcmd) is future work; this
/// gives exact scrub/paused-preview parity for keyframed effect params.
pub fn render_effect_at(def: &EffectDef, inst: &EffectInstance, at_us: i64) -> String {
    static UNIQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut out = def.ffmpeg.clone();
    if out.contains("{u}") {
        let n = UNIQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        out = out.replace("{u}", &format!("u{n}"));
    }
    for p in &def.params {
        let placeholder = format!("{{{}}}", p.key);
        let value = match &p.kind {
            ParamKind::Float { default, min, max } => {
                let v = inst
                    .params
                    .get(&p.key)
                    .map(|param| param.eval(at_us))
                    .unwrap_or(*default)
                    .clamp(*min, *max);
                format_float(v)
            }
            ParamKind::Color { default } => {
                let hex = inst.color_params.get(&p.key).map(String::as_str).unwrap_or(default);
                // Also expose the components as 0..1 floats ({key_r}/{key_g}/
                // {key_b}). Filters that TINT something (a shadow colouring the
                // source's alpha with colorchannelmixer's ra/ga/ba) need the
                // channels, not the packed 0xRRGGBB literal.
                let (r, g, b) = rgb01(hex).unwrap_or_else(|| rgb01(default).unwrap_or((0.0, 0.0, 0.0)));
                out = out.replace(&format!("{{{}_r}}", p.key), &format_float(r));
                out = out.replace(&format!("{{{}_g}}", p.key), &format_float(g));
                out = out.replace(&format!("{{{}_b}}", p.key), &format_float(b));
                format_color(hex).unwrap_or_else(|| format_color(default).expect("valid default"))
            }
        };
        out = out.replace(&placeholder, &value);
    }
    out
}

/// "#rrggbb" → (r, g, b) each 0..1.
pub fn rgb01(hex: &str) -> Option<(f64, f64, f64)> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() < 6 {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f64 / 255.0);
    Some((c(0)?, c(2)?, c(4)?))
}

/// Full -vf chain for a clip's enabled effects (None if there are none).
pub fn render_chain(registry: &[EffectDef], effects: &[EffectInstance]) -> Option<String> {
    render_chain_at(registry, effects, 0)
}

/// Effect chain with animated params evaluated at `at_us` (clip-relative).
pub fn render_chain_at(
    registry: &[EffectDef],
    effects: &[EffectInstance],
    at_us: i64,
) -> Option<String> {
    let parts: Vec<String> = effects
        .iter()
        .filter(|e| e.enabled)
        .filter_map(|e| find_effect(registry, &e.effect_id).map(|d| render_effect_at(d, e, at_us)))
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(","))
    }
}

/// Paused-preview chain: BOTH effects and transform sampled at `at_us`.
pub fn clip_vf_sampled(
    registry: &[EffectDef],
    effects: &[EffectInstance],
    transform: &ue_core::model::Transform2D,
    canvas: Option<(u32, u32)>,
    at_us: i64,
) -> Option<String> {
    clip_vf_sampled_ex(registry, effects, transform, canvas, at_us, false)
}

/// Like `clip_vf_sampled`, but `transparent` composites the transform over an
/// alpha-0 canvas (a PiP layer) instead of an opaque one (the base). Both
/// evaluate every curve at `at_us` and bake it, so a SINGLE extracted frame
/// renders exactly as the export does at that instant.
pub fn clip_vf_sampled_ex(
    registry: &[EffectDef],
    effects: &[EffectInstance],
    transform: &ue_core::model::Transform2D,
    canvas: Option<(u32, u32)>,
    at_us: i64,
    transparent: bool,
) -> Option<String> {
    match (
        render_chain_at(registry, effects, at_us),
        transform_vf_full(&transform.sampled(at_us), canvas, transparent, "t", true),
    ) {
        (Some(e), Some(t)) => Some(format!("{e},{t}")),
        (Some(e), None) => Some(e),
        (None, Some(t)) => Some(t),
        (None, None) => None,
    }
}

/// Serializable catalog for the UI / MCP.
pub fn catalog_json(registry: &[EffectDef]) -> serde_json::Value {
    serde_json::to_value(registry).expect("registry is serializable")
}

// ---------------------------------------------------------------------------
// Transform2D → ffmpeg chain (v0 backend)
// ---------------------------------------------------------------------------

/// -vf chain for a clip's transform: crop → scale → rotation → flips →
/// position (PLAN §6.8 order). Position composes over a canvas the size of the
/// sequence (color+overlay, requires `canvas`). Opacity arrives with wgpu.
/// Curves: evaluated at t=0.
/// ffmpeg expression for an animated Param: exact Hold/Linear segments, Smooth
/// linearized. For a Const returns the plain number. `tvar` is the expression
/// of the time RELATIVE TO THE CLIP in seconds — "t" in the simple case, or e.g.
/// "(t/2+1.5)" when the stream runs with -ss and speed (playback).
pub fn param_expr(p: &ue_core::keyframe::Param, tvar: &str) -> String {
    use ue_core::keyframe::{Interp, Param};
    let curve = match p {
        Param::Const(v) => return format_float(*v),
        Param::Curve(c) if c.keys.is_empty() => return "0".into(),
        Param::Curve(c) => c,
    };
    let ts = |us: i64| format!("{:.6}", us as f64 / 1_000_000.0);
    let keys = &curve.keys;
    let mut expr = format_float(keys[keys.len() - 1].value);
    for i in (0..keys.len().saturating_sub(1)).rev() {
        let (k0, k1) = (&keys[i], &keys[i + 1]);
        let seg = match k0.interp {
            Interp::Hold => format_float(k0.value),
            _ => format!(
                "({}+({})*({tvar}-{})/({:.6}))",
                format_float(k0.value),
                format_float(k1.value - k0.value),
                ts(k0.t),
                ((k1.t - k0.t).max(1)) as f64 / 1_000_000.0,
            ),
        };
        expr = format!("if(lt({tvar},{}),{seg},{expr})", ts(k1.t));
    }
    format!("if(lt({tvar},{}),{},{expr})", ts(keys[0].t), format_float(keys[0].value))
}

/// Does the Param really animate (curve with 2+ keys)?
fn animated(p: &ue_core::keyframe::Param) -> bool {
    matches!(p, ue_core::keyframe::Param::Curve(c) if c.keys.len() > 1)
}

pub fn transform_vf(
    t: &ue_core::model::Transform2D,
    canvas: Option<(u32, u32)>,
) -> Option<String> {
    transform_vf_at(t, canvas, false, "t")
}

pub fn transform_vf_ex(
    t: &ue_core::model::Transform2D,
    canvas: Option<(u32, u32)>,
    transparent: bool,
) -> Option<String> {
    transform_vf_at(t, canvas, transparent, "t")
}

/// Like `transform_vf`; with `transparent` the position canvas and the
/// rotation fill carry alpha 0 (to compose the layer over others in export).
/// Position, rotation, SCALE and OPACITY with curves emit expressions in
/// `tvar` (they animate in export and playback); crop evaluates at t=0.
pub fn transform_vf_at(
    t: &ue_core::model::Transform2D,
    canvas: Option<(u32, u32)>,
    transparent: bool,
    tvar: &str,
) -> Option<String> {
    transform_vf_full(t, canvas, transparent, tvar, false)
}

/// `pts_rebase` rebases the foreground PTS before the canvas overlay: needed
/// ONLY for single-frame extraction (paused preview). See the comment below.
pub fn transform_vf_full(
    t: &ue_core::model::Transform2D,
    canvas: Option<(u32, u32)>,
    transparent: bool,
    tvar: &str,
    pts_rebase: bool,
) -> Option<String> {
    let mut parts: Vec<String> = vec![];

    // Does any geometric/opacity transform apply? Then the frame must be
    // FITTED to the sequence canvas before compositing: the preview may be
    // decoding a half-size proxy, and without the fit a mere position nudge
    // would suddenly render the clip at proxy size (field-reported bug).
    // Layers (transparent) keep their native size on purpose (PiP semantics).
    let pre_scale_animated = animated(&t.scale.0) || animated(&t.scale.1);
    let (pre_sx, pre_sy) =
        (t.scale.0.eval(0).clamp(0.01, 10.0), t.scale.1.eval(0).clamp(0.01, 10.0));
    let pre_has_scale =
        pre_scale_animated || (pre_sx - 1.0).abs() > 1e-4 || (pre_sy - 1.0).abs() > 1e-4;
    let pre_has_rot = animated(&t.rotation) || t.rotation.eval(0).abs() > 1e-4;
    let pre_op = t.opacity.eval(0).clamp(0.0, 1.0);
    let pre_has_op = animated(&t.opacity) || pre_op < 0.999;
    let pre_has_pos = animated(&t.position.0)
        || animated(&t.position.1)
        || t.position.0.eval(0).round() as i64 != 0
        || t.position.1.eval(0).round() as i64 != 0;
    let fit_to_canvas = !transparent
        && (pre_has_pos || pre_has_scale || pre_has_rot || pre_has_op)
        && canvas.is_some();
    if let (true, Some((cw, ch))) = (fit_to_canvas, canvas) {
        parts.push(format!("scale={cw}:{ch}:force_original_aspect_ratio=decrease"));
    }

    let (l, top, r, b) = (
        t.crop.0.eval(0).clamp(0.0, 0.49),
        t.crop.1.eval(0).clamp(0.0, 0.49),
        t.crop.2.eval(0).clamp(0.0, 0.49),
        t.crop.3.eval(0).clamp(0.0, 0.49),
    );
    if l + top + r + b > 1e-4 {
        parts.push(format!(
            "crop=w=trunc(iw*{}/2)*2:h=trunc(ih*{}/2)*2:x=iw*{}:y=ih*{}",
            format_float(1.0 - l - r),
            format_float(1.0 - top - b),
            format_float(l),
            format_float(top),
        ));
    }

    let scale_animated = animated(&t.scale.0) || animated(&t.scale.1);
    let (sx, sy) = (t.scale.0.eval(0).clamp(0.01, 10.0), t.scale.1.eval(0).clamp(0.01, 10.0));
    if scale_animated {
        // eval=frame: the size varies; compositing over the canvas normalizes it
        let ex = param_expr(&t.scale.0, tvar);
        let ey = param_expr(&t.scale.1, tvar);
        parts.push(format!(
            "scale=w='trunc(iw*clip({ex},0.01,10)/2)*2':h='trunc(ih*clip({ey},0.01,10)/2)*2':eval=frame"
        ));
    } else if (sx - 1.0).abs() > 1e-4 || (sy - 1.0).abs() > 1e-4 {
        parts.push(format!(
            "scale=trunc(iw*{}/2)*2:trunc(ih*{}/2)*2",
            format_float(sx),
            format_float(sy),
        ));
    }

    let rot_animated = animated(&t.rotation);
    let deg = t.rotation.eval(0);
    if rot_animated || deg.abs() > 1e-4 {
        let fill = if transparent { "black@0.0" } else { "black" };
        if transparent {
            parts.push("format=rgba".into());
        }
        if rot_animated {
            // animated angle: expression in tvar; output canvas = max diagonal
            let expr = param_expr(&t.rotation, tvar);
            parts.push(format!(
                "rotate=a='({expr})*PI/180':ow=hypot(iw\\,ih):oh=ow:c={fill}"
            ));
        } else {
            let rad = format_float(deg.to_radians());
            parts.push(format!("rotate={rad}:ow=rotw({rad}):oh=roth({rad}):c={fill}"));
        }
    }

    if t.flip_h {
        parts.push("hflip".into());
    }
    if t.flip_v {
        parts.push("vflip".into());
    }

    // opacity: static via colorchannelmixer; animated via geq over the alpha
    // (correct but per-pixel: only when it really animates)
    let op_animated = animated(&t.opacity);
    let op = t.opacity.eval(0).clamp(0.0, 1.0);
    if op_animated {
        // geq uses T (uppercase) as time; our tvar only contains 't'
        // as a letter, so the replacement is safe
        let expr = param_expr(&t.opacity, &tvar.replace('t', "T"));
        parts.push(format!(
            "format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='alpha(X,Y)*clip({expr},0,1)'"
        ));
    } else if op < 0.999 {
        parts.push(format!("format=rgba,colorchannelmixer=aa={op:.4}"));
    }

    // position: compose over a canvas the size of the sequence. Also
    // triggers when scale or opacity animate: normalizes the variable size
    // and flattens the alpha against the background.
    let pos_animated = animated(&t.position.0) || animated(&t.position.1);
    let (px, py) = (t.position.0.eval(0).round() as i64, t.position.1.eval(0).round() as i64);
    let needs_canvas =
        pos_animated || px != 0 || py != 0 || scale_animated || op_animated || op < 0.999;
    if let Some((cw, ch)) = canvas.filter(|_| needs_canvas) {
        static POS_UNIQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = POS_UNIQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let bg = if transparent { "black@0.0" } else { "black" };
        let bg_fmt = if transparent { ",format=rgba" } else { "" };
        let (xe, ye) = if pos_animated {
            (
                format!("'(W-w)/2+({})'", param_expr(&t.position.0, tvar)),
                format!("'(H-h)/2+({})'", param_expr(&t.position.1, tvar)),
            )
        } else {
            (format!("(W-w)/2+{px}"), format!("(H-h)/2+{py}"))
        };
        // format=auto: keep the fg alpha when compositing over a transparent canvas
        let of = if transparent { ":format=auto" } else { "" };
        // Single-frame extraction (paused preview): -ss lands between
        // keyframes, so the fg PTS is large while `color` starts at 0 and
        // overlay emits only the background (black paused frame). Rebasing
        // the fg PTS fixes it. Streams/export keep their timeline PTS,
        // which `enable=between(t,…)` and animations depend on.
        let fg_pts = if pts_rebase { "setpts=PTS-STARTPTS" } else { "null" };
        parts.push(format!(
            "{fg_pts}[p{n}fg];color=c={bg}:s={cw}x{ch}{bg_fmt}[p{n}bg];[p{n}bg][p{n}fg]overlay=x={xe}:y={ye}:shortest=1{of}"
        ));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(","))
    }
}

/// Full chain for a clip: effects + transform (in that order).
pub fn clip_vf(
    registry: &[EffectDef],
    effects: &[EffectInstance],
    transform: &ue_core::model::Transform2D,
    canvas: Option<(u32, u32)>,
) -> Option<String> {
    match (render_chain(registry, effects), transform_vf(transform, canvas)) {
        (Some(e), Some(t)) => Some(format!("{e},{t}")),
        (Some(e), None) => Some(e),
        (None, Some(t)) => Some(t),
        (None, None) => None,
    }
}

/// Chain for a clip as an overlaid LAYER (transparent canvas/rotation).
/// `tvar` = expression of the time relative to the clip (layers run in
/// timeline time: "(t-START)").
pub fn clip_vf_layer(
    registry: &[EffectDef],
    effects: &[EffectInstance],
    transform: &ue_core::model::Transform2D,
    canvas: Option<(u32, u32)>,
    tvar: &str,
) -> Option<String> {
    match (render_chain(registry, effects), transform_vf_at(transform, canvas, true, tvar)) {
        (Some(e), Some(t)) => Some(format!("{e},{t}")),
        (Some(e), None) => Some(e),
        (None, Some(t)) => Some(t),
        (None, None) => None,
    }
}

/// Full chain with explicit tvar (playback with -ss/speed).
pub fn clip_vf_at(
    registry: &[EffectDef],
    effects: &[EffectInstance],
    transform: &ue_core::model::Transform2D,
    canvas: Option<(u32, u32)>,
    tvar: &str,
) -> Option<String> {
    match (render_chain(registry, effects), transform_vf_at(transform, canvas, false, tvar)) {
        (Some(e), Some(t)) => Some(format!("{e},{t}")),
        (Some(e), None) => Some(e),
        (None, Some(t)) => Some(t),
        (None, None) => None,
    }
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
        // only brightness changed; the rest use defaults
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
        // corrupt color → manifest default
        let bad = inst("core.chroma_key", &[], &[("key_color", "green;rm -rf")]);
        assert!(render_effect(def, &bad).contains("color=0x00FF00"));
    }

    #[test]
    fn user_packs_load_report_errors_and_override_core() {
        let dir = std::env::temp_dir().join("ue-user-packs-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("vhs")).unwrap();
        std::fs::create_dir_all(dir.join("broken")).unwrap();
        std::fs::create_dir_all(dir.join("override")).unwrap();
        std::fs::write(
            dir.join("vhs/manifest.json"),
            r#"{"id":"user.vhs","name":"VHS","ffmpeg":"noise=alls=20","params":[]}"#,
        )
        .unwrap();
        std::fs::write(dir.join("broken/manifest.json"), "{this is not json").unwrap();
        std::fs::write(
            dir.join("override/manifest.json"),
            r#"{"id":"core.gaussian_blur","name":"Blur custom","ffmpeg":"gblur=sigma=99","params":[]}"#,
        )
        .unwrap();

        let (packs, errors) = load_packs_from_dir(&dir);
        assert_eq!(packs.len(), 2);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("broken"));

        let merged = merge_registries(core_registry(), packs);
        assert!(find_effect(&merged, "user.vhs").is_some());
        assert_eq!(
            find_effect(&merged, "core.gaussian_blur").unwrap().name,
            "Blur custom",
            "the user pack overrides the core"
        );
        // nonexistent dir → empty without error
        let (empty, errs) = load_packs_from_dir(std::path::Path::new("/does/not/exist"));
        assert!(empty.is_empty() && errs.is_empty());
    }

    #[test]
    fn transform_noop_is_none_and_order_is_crop_scale_rotate_flip() {
        use ue_core::model::Transform2D;
        assert_eq!(transform_vf(&Transform2D::default(), None), None);

        let mut t = Transform2D::default();
        t.crop.0 = 0.25.into(); // 25% from the left
        t.scale = (0.5.into(), 0.5.into());
        t.rotation = 180.0.into();
        t.flip_h = true;
        let vf = transform_vf(&t, None).unwrap();
        let crop_pos = vf.find("crop=").unwrap();
        let scale_pos = vf.find("scale=").unwrap();
        let rot_pos = vf.find("rotate=").unwrap();
        let flip_pos = vf.find("hflip").unwrap();
        assert!(crop_pos < scale_pos && scale_pos < rot_pos && rot_pos < flip_pos, "{vf}");
        assert!(vf.contains("rotate=3.1416"), "180° in radians: {vf}");
        assert!(vf.contains("x=iw*0.25"), "{vf}");
    }

    #[test]
    fn clip_vf_combines_effects_then_transform() {
        use ue_core::model::Transform2D;
        let reg = core_registry();
        let fx = [inst("core.gaussian_blur", &[("sigma", 3.0)], &[])];
        let mut t = Transform2D::default();
        t.flip_v = true;
        let vf = clip_vf(&reg, &fx, &t, None).unwrap();
        assert!(vf.starts_with("gblur"), "{vf}");
        assert!(vf.ends_with("vflip"), "{vf}");
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
        // unknown effect is ignored without breaking
        let unknown = inst("user.nonexistent", &[], &[]);
        assert!(render_chain(&reg, &[unknown]).is_none());
    }
}

#[cfg(test)]
mod color_component_tests {
    use super::*;
    use ue_core::model::EffectInstance;

    /// A colour param also exposes {key_r}/{key_g}/{key_b} as 0..1 floats, so a
    /// filter can TINT the source's alpha (the drop shadow's colour).
    #[test]
    fn color_params_expose_components() {
        let reg = core_registry();
        let def = find_effect(&reg, "core.drop_shadow").expect("drop_shadow exists");
        let inst = EffectInstance {
            effect_id: "core.drop_shadow".into(),
            enabled: true,
            params: Default::default(),
            color_params: [("shadow_color".to_string(), "#ff8000".to_string())]
                .into_iter()
                .collect(),
        };
        let out = render_effect_at(def, &inst, 0);
        assert!(out.contains("ra=1"), "red channel at full: {out}");
        assert!(out.contains("ga=0.5"), "green ~0.5: {out}");
        assert!(out.contains("ba=0"), "blue at zero: {out}");
        assert!(!out.contains('{'), "every placeholder substituted: {out}");
    }

    /// The default (black) keeps the classic shadow.
    #[test]
    fn default_shadow_is_black() {
        let reg = core_registry();
        let def = find_effect(&reg, "core.drop_shadow").unwrap();
        let inst = EffectInstance {
            effect_id: "core.drop_shadow".into(),
            enabled: true,
            params: Default::default(),
            color_params: Default::default(),
        };
        let out = render_effect_at(def, &inst, 0);
        assert!(out.contains("ra=0") && out.contains("ga=0") && out.contains("ba=0"), "{out}");
    }
}
