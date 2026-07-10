//! Background-noise reduction of a conformed WAV (ffmpeg afftdn).
//! The toolkit used a torch DNS64 model; afftdn is the dependency-free
//! equivalent that ships inside ffmpeg (spectral denoiser with noise-floor
//! tracking) and is plenty for fans/hiss/room tone.

use std::path::Path;
use std::process::Command;

use crate::{ffmpeg_bin, MediaError, MediaResult};

/// The exact filter used in BOTH live (pre-rendered conform variant) and
/// export (inline in the audio chain) so they sound identical.
// nf must start NEAR the real noise floor (tn refines it); a too-low nf
// makes the filter treat the noise as signal. Measured on a speech+white
// noise fixture: -25 dB of noise for ~1 dB of voice.
pub const DENOISE_FILTER: &str = "afftdn=nr=30:nf=-25:tn=1";

/// Path of the denoised sibling of a conform WAV (`x.wav` → `x.denoise.wav`).
pub fn denoised_path(conform: &Path) -> std::path::PathBuf {
    conform.with_extension("denoise.wav")
}

/// Renders the denoised variant (48 kHz stereo s16le, like the conform).
/// No-op if it already exists.
pub fn denoise_wav(conform: &Path) -> MediaResult<std::path::PathBuf> {
    let out = denoised_path(conform);
    if out.exists() {
        return Ok(out);
    }
    let tmp = out.with_extension("part.wav");
    let status = Command::new(ffmpeg_bin())
        .args(["-y", "-v", "error", "-i"])
        .arg(conform)
        .args(["-af", DENOISE_FILTER, "-ar", "48000", "-ac", "2", "-c:a", "pcm_s16le"])
        .arg(&tmp)
        .status()
        .map_err(|e| MediaError::Spawn("ffmpeg".into(), e.to_string()))?;
    if !status.success() || !tmp.exists() {
        let _ = std::fs::remove_file(&tmp);
        return Err(MediaError::Tool("ffmpeg".into(), "denoise failed".into()));
    }
    std::fs::rename(&tmp, &out)?;
    Ok(out)
}
