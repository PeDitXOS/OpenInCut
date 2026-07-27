//! Text-to-speech voiceover (PLAN §16 "dubbing", brought forward).
//!
//! Modular by design (PLAN §1.4 "radical modularity"): every synthesizer is a
//! [`TtsEngine`] behind a registry, and users can add engines WITHOUT touching
//! code by dropping a JSON manifest into the app's `tts_engines/` folder
//! (same philosophy as the effect packs). Built-ins:
//! - `say`: the macOS system synthesizer. Instant, offline, dozens of system
//!   voices (the "herramientas del sistema" path).
//! - `kokoro`: the Kokoro-82M AI voice (24 kHz), SELF-CONTAINED like the
//!   DNS64 denoiser (ue-media::denoise): the sidecar script is embedded and
//!   the app provisions its own venv on first use (`pip install kokoro`).
//!   `UE_TTS_PYTHON` overrides the interpreter ("off" disables); a
//!   Youtubers-toolkit venv is used as a courtesy on dev machines that
//!   already have one.
//!
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// One selectable voice, engine-agnostic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsVoice {
    /// What the engine expects (`say -v <id>` / kokoro voice id).
    pub id: String,
    /// Display name.
    pub name: String,
    /// BCP-47-ish tag ("es_MX", "en-US") for grouping in the picker.
    pub lang: String,
}

/// How an engine understands its `rate` knob; the UI renders the slider
/// straight from this, so engines with different semantics coexist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateSpec {
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub step: f64,
    /// Slider caption ("words/min", "speed ×").
    pub label: String,
}

/// Everything the UI needs to render one engine, voices included.
#[derive(Debug, Clone, Serialize)]
pub struct EngineInfo {
    pub id: String,
    pub name: String,
    pub available: bool,
    /// Why it is unavailable, or where it was found.
    pub detail: String,
    pub voices: Vec<TtsVoice>,
    pub rate: Option<RateSpec>,
}

/// A pluggable synthesizer. The registry is the only place that knows which
/// engines exist; callers speak in engine ids.
pub trait TtsEngine: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> String;
    /// (usable now, why / where found).
    fn availability(&self) -> (bool, String);
    fn voices(&self) -> Vec<TtsVoice>;
    fn rate_spec(&self) -> Option<RateSpec>;
    /// Output container the engine writes ("aiff", "wav").
    fn ext(&self) -> &str;
    fn synthesize(
        &self,
        text: &str,
        voice: Option<&str>,
        rate: Option<f64>,
        out: &Path,
    ) -> Result<(), String>;

    fn info(&self) -> EngineInfo {
        let (available, detail) = self.availability();
        EngineInfo {
            id: self.id().to_string(),
            name: self.name(),
            available,
            detail,
            voices: self.voices(),
            rate: self.rate_spec(),
        }
    }
}

/// All engines: built-ins first, then user manifests from `user_dir`
/// (`*.json`). A user manifest with a built-in's id replaces it, like
/// effect packs. Manifest errors are returned, never fatal.
/// `kokoro_env_dir` is where the app provisions Kokoro's own venv.
pub fn registry(
    kokoro_env_dir: Option<&Path>,
    user_dir: Option<&Path>,
) -> (Vec<Box<dyn TtsEngine>>, Vec<String>) {
    let mut engines: Vec<Box<dyn TtsEngine>> = vec![
        Box::new(SayEngine),
        Box::new(KokoroEngine { env_dir: kokoro_env_dir.map(Path::to_path_buf) }),
    ];
    let mut errors = vec![];
    if let Some(dir) = user_dir {
        let (user, errs) = load_user_engines(dir);
        errors = errs;
        for e in user {
            engines.retain(|b| b.id() != e.manifest.id);
            engines.push(Box::new(e));
        }
    }
    (engines, errors)
}

/// The UI-facing catalog (one `EngineInfo` per registered engine).
pub fn catalog(kokoro_env_dir: Option<&Path>, user_dir: Option<&Path>) -> Vec<EngineInfo> {
    registry(kokoro_env_dir, user_dir).0.iter().map(|e| e.info()).collect()
}

// ---------------------------------------------------------------------------
// Built-in: macOS `say`
// ---------------------------------------------------------------------------

pub struct SayEngine;

impl TtsEngine for SayEngine {
    fn id(&self) -> &str {
        "say"
    }
    fn name(&self) -> String {
        "System voice (say)".into()
    }
    fn availability(&self) -> (bool, String) {
        if say_available() {
            (true, "/usr/bin/say".into())
        } else {
            (false, "only available on macOS".into())
        }
    }
    fn voices(&self) -> Vec<TtsVoice> {
        list_say_voices()
    }
    fn rate_spec(&self) -> Option<RateSpec> {
        Some(RateSpec { min: 90.0, max: 400.0, default: 175.0, step: 5.0, label: "words/min".into() })
    }
    fn ext(&self) -> &str {
        "aiff"
    }
    fn synthesize(
        &self,
        text: &str,
        voice: Option<&str>,
        rate: Option<f64>,
        out: &Path,
    ) -> Result<(), String> {
        let wpm = rate.map(|r| r.round().clamp(90.0, 400.0) as u32);
        synthesize_say(text, voice, wpm, out)
    }
}

/// Parses `say -v ?` output. Voice names may contain spaces ("Bad News"),
/// so the locale is the last whitespace token before the `#` sample.
pub fn parse_say_voices(output: &str) -> Vec<TtsVoice> {
    let mut voices = vec![];
    for line in output.lines() {
        let left = line.split('#').next().unwrap_or("").trim_end();
        let Some((name, lang)) = left.rsplit_once(char::is_whitespace) else { continue };
        let (name, lang) = (name.trim(), lang.trim());
        if name.is_empty() || lang.is_empty() {
            continue;
        }
        voices.push(TtsVoice { id: name.into(), name: name.into(), lang: lang.into() });
    }
    voices
}

/// System voices, or empty when `say` is unavailable (non-macOS).
pub fn list_say_voices() -> Vec<TtsVoice> {
    let Ok(out) = Command::new("say").args(["-v", "?"]).output() else { return vec![] };
    if !out.status.success() {
        return vec![];
    }
    parse_say_voices(&String::from_utf8_lossy(&out.stdout))
}

pub fn say_available() -> bool {
    cfg!(target_os = "macos") && Path::new("/usr/bin/say").exists()
}

/// Synthesizes with the system voice into an AIFF. The text goes through a
/// temp file (`-f`): argv has size limits and a script starting with "-"
/// would be parsed as a flag.
pub fn synthesize_say(
    text: &str,
    voice: Option<&str>,
    rate_wpm: Option<u32>,
    out: &Path,
) -> Result<(), String> {
    let txt = out.with_extension("txt");
    std::fs::write(&txt, text).map_err(|e| format!("could not write the script: {e}"))?;
    let mut cmd = Command::new("say");
    if let Some(v) = voice.filter(|v| !v.is_empty()) {
        cmd.args(["-v", v]);
    }
    if let Some(r) = rate_wpm {
        cmd.args(["-r", &r.to_string()]);
    }
    let res = cmd.arg("-o").arg(out).arg("-f").arg(&txt).output();
    let _ = std::fs::remove_file(&txt);
    let res = res.map_err(|e| format!("could not run `say`: {e}"))?;
    if !res.status.success() {
        return Err(format!("say failed: {}", String::from_utf8_lossy(&res.stderr).trim()));
    }
    if !out.exists() {
        return Err("say produced no audio file".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Built-in: Kokoro (self-contained, like ue-media's DNS64 denoiser)
// ---------------------------------------------------------------------------

pub struct KokoroEngine {
    /// Where the app provisions Kokoro's own venv (e.g. `<app_data>/kokoro`).
    pub env_dir: Option<PathBuf>,
}

impl KokoroEngine {
    /// Interpreter WITHOUT provisioning: `UE_TTS_PYTHON` (value "off"/
    /// "none"/"0" disables), then the app-owned venv, then — as a courtesy
    /// on dev machines that have it — a Youtubers-toolkit venv with kokoro.
    fn python(&self) -> Option<PathBuf> {
        match std::env::var("UE_TTS_PYTHON") {
            Ok(v) if matches!(v.as_str(), "off" | "none" | "0" | "") => return None,
            Ok(v) => return Some(PathBuf::from(v)),
            Err(_) => {}
        }
        if let Some(dir) = &self.env_dir {
            let own = ue_media::denoise::venv_python(dir);
            if own.exists() {
                return Some(own);
            }
        }
        toolkit_kokoro_python()
    }
}

impl TtsEngine for KokoroEngine {
    fn id(&self) -> &str {
        "kokoro"
    }
    fn name(&self) -> String {
        "Kokoro AI".into()
    }
    fn availability(&self) -> (bool, String) {
        if std::env::var("UE_TTS_PYTHON").is_ok_and(|v| matches!(v.as_str(), "off" | "none" | "0")) {
            return (false, "disabled by UE_TTS_PYTHON — unset it to re-enable".into());
        }
        if self.python().is_some() {
            return (true, "local AI voice, offline".into());
        }
        match (self.env_dir.is_some(), ue_media::denoise::find_system_python().is_some()) {
            (true, true) => (true, "sets itself up on first use (one-time download)".into()),
            _ => (
                false,
                "install Python 3 ≥ 3.9 (e.g. `brew install python`) to enable it".into(),
            ),
        }
    }
    fn voices(&self) -> Vec<TtsVoice> {
        kokoro_voices()
    }
    fn rate_spec(&self) -> Option<RateSpec> {
        Some(RateSpec { min: 0.5, max: 2.0, default: 1.0, step: 0.05, label: "speed ×".into() })
    }
    fn ext(&self) -> &str {
        "wav"
    }
    fn synthesize(
        &self,
        text: &str,
        voice: Option<&str>,
        rate: Option<f64>,
        out: &Path,
    ) -> Result<(), String> {
        let python = match self.python() {
            Some(p) => p,
            None => {
                let dir = self.env_dir.as_deref().ok_or("no folder for the Kokoro env")?;
                ensure_kokoro_env(dir)?
            }
        };
        let voice = voice.filter(|v| !v.is_empty()).unwrap_or("af_heart");
        let speed = rate.unwrap_or(1.0).clamp(0.5, 2.0);
        synthesize_kokoro(&python, text, voice, speed, out)
    }
}

/// A Youtubers-toolkit venv that already has kokoro installed (dev
/// convenience only; the app never depends on the toolkit checkout).
fn toolkit_kokoro_python() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let toolkit = PathBuf::from(home).join("Videos Reel/Youtubers-toolkit");
    let python = toolkit.join("venv/bin/python");
    if !python.is_file() {
        return None;
    }
    let lib = toolkit.join("venv/lib");
    let has_kokoro = std::fs::read_dir(&lib)
        .map(|entries| {
            entries.flatten().any(|e| e.path().join("site-packages/kokoro").is_dir())
        })
        .unwrap_or(false);
    has_kokoro.then_some(python)
}

/// Provisions the app-owned Kokoro venv (one-time, self-contained: only a
/// system `python3` is required), exactly like `ensure_denoiser_env`:
/// `python3 -m venv` + `pip install kokoro soundfile`. Validated by import;
/// a broken half-install is removed so the next attempt starts clean.
pub fn ensure_kokoro_env(env_dir: &Path) -> Result<PathBuf, String> {
    let python = ue_media::denoise::venv_python(env_dir);
    if python.exists() {
        return Ok(python);
    }
    std::fs::create_dir_all(env_dir).map_err(|e| e.to_string())?;
    let venv = env_dir.join("venv");
    eprintln!("[tts] provisioning the Kokoro environment in {venv:?} (one-time)…");
    let system = ue_media::denoise::find_system_python()
        .ok_or("no python3/python ≥ 3.9 found on the system")?;
    let ok = Command::new(&system)
        .args(["-m", "venv"])
        .arg(&venv)
        .status()
        .map_err(|e| format!("could not run {system}: {e}"))?
        .success();
    if !ok {
        return Err("venv creation failed".into());
    }
    let ok = Command::new(&python)
        .args(["-m", "pip", "install", "--quiet", "kokoro==0.9.4", "misaki[en,espeak]==0.9.4", "soundfile"])
        .status()
        .map_err(|e| format!("could not run pip: {e}"))?
        .success();
    let importable = ok
        && Command::new(&python)
            .args(["-c", "import kokoro, soundfile, numpy"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    if !importable {
        let _ = std::fs::remove_dir_all(&venv);
        return Err("kokoro install failed (network? python < 3.9?)".into());
    }
    eprintln!("[tts] Kokoro environment ready");
    Ok(python)
}

/// Curated Kokoro voices (kokoro derives the language from the id's first
/// letter: a=en-US, b=en-GB, e=es). The toolkit's default is af_heart.
pub fn kokoro_voices() -> Vec<TtsVoice> {
    let v = |id: &str, name: &str, lang: &str| TtsVoice {
        id: id.into(),
        name: name.into(),
        lang: lang.into(),
    };
    vec![
        v("ef_dora", "Dora", "es"),
        v("em_alex", "Alex", "es"),
        v("em_santa", "Santa", "es"),
        v("af_heart", "Heart", "en-US"),
        v("af_bella", "Bella", "en-US"),
        v("af_nicole", "Nicole", "en-US"),
        v("af_sarah", "Sarah", "en-US"),
        v("am_adam", "Adam", "en-US"),
        v("am_michael", "Michael", "en-US"),
        v("am_fenrir", "Fenrir", "en-US"),
        v("am_puck", "Puck", "en-US"),
        v("bf_emma", "Emma", "en-GB"),
        v("bf_isabella", "Isabella", "en-GB"),
        v("bm_george", "George", "en-GB"),
        v("bm_lewis", "Lewis", "en-GB"),
        v("fj_amine", "Amine", "fa"),
        v("fj_darius", "Darius", "fa"),
        v("fj_parisa", "Parisa", "fa"),
        v("ej_ahmed", "Ahmed", "ar"),
        v("ej_fatima", "Fatima", "ar"),
        v("ej_khalid", "Khalid", "ar"),
    ]
}

/// The Kokoro sidecar script, embedded so packaged builds carry it (same
/// mechanism as ue-media's DNS64 sidecar).
const KOKORO_SCRIPT: &str = include_str!("../../../scripts/tts_kokoro.py");

/// phonemizer (kokoro's espeak backend, used for Spanish and as an English
/// fallback) does not search Homebrew's lib dir on macOS; point it at the
/// dylib explicitly unless the user already did.
pub fn espeak_library() -> Option<PathBuf> {
    [
        "/opt/homebrew/lib/libespeak-ng.dylib",
        "/opt/homebrew/lib/libespeak-ng.1.dylib",
        "/usr/local/lib/libespeak-ng.dylib",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
}

/// Runs the embedded Kokoro sidecar with the given interpreter. Downloads
/// the model on first use (~330 MB to the HF cache), so the first run can
/// take minutes.
pub fn synthesize_kokoro(
    python: &Path,
    text: &str,
    voice: &str,
    speed: f64,
    out: &Path,
) -> Result<(), String> {
    use std::io::Write;
    let script = std::env::temp_dir().join("ue_tts_kokoro.py");
    std::fs::write(&script, KOKORO_SCRIPT).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(python);
    if std::env::var_os("PHONEMIZER_ESPEAK_LIBRARY").is_none() {
        if let Some(lib) = espeak_library() {
            cmd.env("PHONEMIZER_ESPEAK_LIBRARY", lib);
        }
    }
    let mut child = cmd
        .arg(&script)
        .args([voice, &format!("{speed}")])
        .arg(out)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run the toolkit python: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(text.as_bytes())
        .map_err(|e| format!("could not send the script: {e}"))?;
    let res = child.wait_with_output().map_err(|e| e.to_string())?;
    if !res.status.success() {
        // last lines only: torch/transformers greet with pages of warnings
        let err = String::from_utf8_lossy(&res.stderr);
        let tail: Vec<&str> = err.lines().rev().take(4).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!("kokoro failed: {}", tail.join(" | ")));
    }
    if !out.exists() {
        return Err("kokoro produced no audio file".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// User engines: JSON manifest → command template
// ---------------------------------------------------------------------------

/// A user-defined engine (`<app_config>/tts_engines/*.json`):
///
/// ```json
/// {
///   "id": "espeak", "name": "eSpeak NG", "ext": "wav",
///   "argv": ["espeak-ng", "-v", "{voice}", "-s", "{rate}", "-w", "{out}", "{text}"],
///   "voices": [{ "id": "es", "name": "Español", "lang": "es" }],
///   "rate": { "min": 80, "max": 300, "default": 170, "step": 5, "label": "wpm" }
/// }
/// ```
///
/// Placeholders in argv: `{text}`, `{voice}`, `{rate}`, `{out}`. With
/// `"stdin_text": true` the script is piped to stdin instead of `{text}`.
#[derive(Debug, Clone, Deserialize)]
pub struct EngineManifest {
    pub id: String,
    pub name: String,
    #[serde(default = "default_ext")]
    pub ext: String,
    pub argv: Vec<String>,
    #[serde(default)]
    pub voices: Vec<TtsVoice>,
    #[serde(default)]
    pub rate: Option<RateSpec>,
    #[serde(default)]
    pub stdin_text: bool,
}

fn default_ext() -> String {
    "wav".into()
}

pub struct CommandEngine {
    pub manifest: EngineManifest,
}

impl CommandEngine {
    /// Substitutes the placeholders for one synthesis run.
    fn render_argv(&self, text: &str, voice: &str, rate: f64, out: &Path) -> Vec<String> {
        let rate_str =
            if rate.fract() == 0.0 { format!("{}", rate as i64) } else { format!("{rate}") };
        self.manifest
            .argv
            .iter()
            .map(|a| {
                a.replace("{text}", text)
                    .replace("{voice}", voice)
                    .replace("{rate}", &rate_str)
                    .replace("{out}", &out.to_string_lossy())
            })
            .collect()
    }
}

impl TtsEngine for CommandEngine {
    fn id(&self) -> &str {
        &self.manifest.id
    }
    fn name(&self) -> String {
        self.manifest.name.clone()
    }
    fn availability(&self) -> (bool, String) {
        let Some(bin) = self.manifest.argv.first() else {
            return (false, "manifest has an empty argv".into());
        };
        match which(bin) {
            Some(p) => (true, p.display().to_string()),
            None => (false, format!("{bin} not found in PATH")),
        }
    }
    fn voices(&self) -> Vec<TtsVoice> {
        self.manifest.voices.clone()
    }
    fn rate_spec(&self) -> Option<RateSpec> {
        self.manifest.rate.clone()
    }
    fn ext(&self) -> &str {
        &self.manifest.ext
    }
    fn synthesize(
        &self,
        text: &str,
        voice: Option<&str>,
        rate: Option<f64>,
        out: &Path,
    ) -> Result<(), String> {
        use std::io::Write;
        let default_voice =
            self.manifest.voices.first().map(|v| v.id.clone()).unwrap_or_default();
        let voice = voice.filter(|v| !v.is_empty()).unwrap_or(&default_voice);
        let rate = rate
            .or(self.manifest.rate.as_ref().map(|r| r.default))
            .unwrap_or(1.0);
        let rate = match &self.manifest.rate {
            Some(spec) => rate.clamp(spec.min, spec.max),
            None => rate,
        };
        let argv = self.render_argv(text, voice, rate, out);
        let (bin, args) = argv.split_first().ok_or("manifest has an empty argv")?;
        let mut cmd = Command::new(bin);
        cmd.args(args);
        let res = if self.manifest.stdin_text {
            let mut child = cmd
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| format!("could not run {bin}: {e}"))?;
            child
                .stdin
                .take()
                .ok_or("no stdin")?
                .write_all(text.as_bytes())
                .map_err(|e| e.to_string())?;
            child.wait_with_output().map_err(|e| e.to_string())?
        } else {
            cmd.output().map_err(|e| format!("could not run {bin}: {e}"))?
        };
        if !res.status.success() {
            return Err(format!(
                "{bin} failed: {}",
                String::from_utf8_lossy(&res.stderr).trim()
            ));
        }
        if !out.exists() {
            return Err(format!("{bin} produced no audio file"));
        }
        Ok(())
    }
}

/// Reads every `*.json` manifest in `dir`. Bad manifests are reported by
/// file name and skipped, like effect packs.
pub fn load_user_engines(dir: &Path) -> (Vec<CommandEngine>, Vec<String>) {
    let mut engines = vec![];
    let mut errors = vec![];
    let Ok(entries) = std::fs::read_dir(dir) else { return (engines, errors) };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    for p in paths {
        let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
        match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str::<EngineManifest>(&s).map_err(|e| e.to_string()))
        {
            Ok(m) if m.id.trim().is_empty() => errors.push(format!("{name}: empty id")),
            Ok(m) if m.argv.is_empty() => errors.push(format!("{name}: empty argv")),
            Ok(m) => engines.push(CommandEngine { manifest: m }),
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    (engines, errors)
}

/// PATH lookup ("espeak-ng" → /opt/homebrew/bin/espeak-ng); absolute and
/// relative paths with a separator are checked directly.
pub fn which(bin: &str) -> Option<PathBuf> {
    if bin.contains('/') {
        let p = PathBuf::from(bin);
        return p.is_file().then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(bin)).find(|p| p.is_file())
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_say_voice_listing() {
        let fixture = "\
Albert              en_US    # Hello! My name is Albert.
Amélie              fr_CA    # Bonjour! Je m’appelle Amélie.
Bad News            en_US    # Hello! My name is Bad News.
Eddy (Spanish (Mexico)) es_MX # ¡Hola! Me llamo Eddy.
";
        let voices = parse_say_voices(fixture);
        assert_eq!(voices.len(), 4);
        assert_eq!(voices[0].id, "Albert");
        assert_eq!(voices[0].lang, "en_US");
        // spaces inside the name survive
        assert_eq!(voices[2].id, "Bad News");
        assert_eq!(voices[2].lang, "en_US");
        // parenthesised variants keep the trailing locale
        assert_eq!(voices[3].id, "Eddy (Spanish (Mexico))");
        assert_eq!(voices[3].lang, "es_MX");
    }

    #[test]
    fn kokoro_catalog_is_consistent() {
        for v in kokoro_voices() {
            // kokoro derives the language from the first letter of the id
            let lang0 = v.id.chars().next().unwrap();
            match v.lang.as_str() {
                "es" => assert_eq!(lang0, 'e', "{}", v.id),
                "en-US" => assert_eq!(lang0, 'a', "{}", v.id),
                "en-GB" => assert_eq!(lang0, 'b', "{}", v.id),
                "fa" => assert_eq!(lang0, 'f', "{}", v.id),
                "ar" => assert_eq!(lang0, 'e', "{}", v.id),
                other => panic!("unexpected lang {other}"),
            }
        }
    }

    #[test]
    fn registry_always_lists_builtins() {
        let (engines, errors) = registry(None, None);
        assert!(errors.is_empty());
        assert_eq!(engines.iter().map(|e| e.id()).collect::<Vec<_>>(), ["say", "kokoro"]);
        // availability is machine-dependent (python, toolkit venv), but the
        // catalog must always render: names, voices and a rate spec
        for e in &engines {
            let info = e.info();
            assert!(!info.name.is_empty());
            assert!(!info.detail.is_empty());
            assert!(e.rate_spec().is_some());
        }
        assert!(!engines[1].voices().is_empty(), "kokoro ships a curated voice list");
    }

    #[test]
    fn user_manifest_loads_and_overrides() {
        let dir = std::env::temp_dir().join(format!("ue_tts_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("espeak.json"),
            r#"{ "id": "espeak", "name": "eSpeak NG",
                 "argv": ["espeak-ng", "-v", "{voice}", "-w", "{out}", "{text}"],
                 "voices": [{ "id": "es", "name": "Español", "lang": "es" }] }"#,
        )
        .unwrap();
        // overrides the built-in `say`
        std::fs::write(
            dir.join("say.json"),
            r#"{ "id": "say", "name": "My say", "argv": ["true"] }"#,
        )
        .unwrap();
        std::fs::write(dir.join("broken.json"), "{ nope").unwrap();

        let (engines, errors) = registry(None, Some(&dir));
        let ids: Vec<&str> = engines.iter().map(|e| e.id()).collect();
        assert!(ids.contains(&"espeak"));
        assert_eq!(ids.iter().filter(|i| **i == "say").count(), 1);
        let say = engines.iter().find(|e| e.id() == "say").unwrap();
        assert_eq!(say.name(), "My say"); // user wins
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("broken.json"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn command_engine_renders_placeholders() {
        let eng = CommandEngine {
            manifest: EngineManifest {
                id: "x".into(),
                name: "X".into(),
                ext: "wav".into(),
                argv: vec![
                    "bin".into(),
                    "-v".into(),
                    "{voice}".into(),
                    "-s".into(),
                    "{rate}".into(),
                    "-w".into(),
                    "{out}".into(),
                    "{text}".into(),
                ],
                voices: vec![],
                rate: None,
                stdin_text: false,
            },
        };
        let argv = eng.render_argv("hola mundo", "es", 170.0, Path::new("/tmp/o.wav"));
        assert_eq!(argv, ["bin", "-v", "es", "-s", "170", "-w", "/tmp/o.wav", "hola mundo"]);
    }
}
