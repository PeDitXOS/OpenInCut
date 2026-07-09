//! Sesión de decode continuo: un proceso ffmpeg persistente que emite MJPEG
//! por stdout. Leer el siguiente frame es barato (decode secuencial), a
//! diferencia de `frame::render_frame` que arranca un proceso por frame.
//! Es el "DecodePool v0" del PLAN §5.2; el upgrade a libav mantiene esta API.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};

use crate::{ffmpeg_bin, MediaError, MediaResult};

pub struct MjpegSession {
    child: Child,
    stdout: ChildStdout,
    buf: Vec<u8>,
    pub asset_path: PathBuf,
    pub start_src_us: i64,
    pub fps: u32,
    pub frames_read: i64,
}

impl MjpegSession {
    /// Abre una sesión que decodifica `path` desde `start_src_us`, reescalada
    /// a `max_width` y re-muestreada a `fps` constantes. `extra_vf` = cadena
    /// de efectos del clip.
    pub fn open(
        path: &Path,
        start_src_us: i64,
        max_width: u32,
        fps: u32,
        extra_vf: Option<&str>,
    ) -> MediaResult<Self> {
        let ss = format!("{:.6}", start_src_us as f64 / 1_000_000.0);
        let base = format!("fps={fps},scale='min({max_width},iw)':-2");
        let vf = match extra_vf {
            Some(chain) if !chain.is_empty() => format!("{chain},{base}"),
            _ => base,
        };
        let mut child = Command::new(ffmpeg_bin())
            .args(["-v", "error", "-ss", &ss, "-i"])
            .arg(path)
            .args(["-an", "-vf", &vf, "-f", "mjpeg", "-q:v", "6", "pipe:1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| MediaError::Spawn("ffmpeg".into(), e.to_string()))?;
        let stdout = child.stdout.take().expect("stdout piped");
        Ok(MjpegSession {
            child,
            stdout,
            buf: Vec::with_capacity(512 * 1024),
            asset_path: path.to_path_buf(),
            start_src_us,
            fps,
            frames_read: 0,
        })
    }

    /// Tiempo fuente del PRÓXIMO frame que devolvería `next_frame`.
    pub fn next_src_us(&self) -> i64 {
        self.start_src_us + self.frames_read * 1_000_000 / self.fps as i64
    }

    /// Lee el siguiente JPEG del stream. `None` = fin del archivo.
    pub fn next_frame(&mut self) -> MediaResult<Option<Vec<u8>>> {
        let mut chunk = [0u8; 64 * 1024];
        loop {
            if let Some(frame) = extract_jpeg(&mut self.buf) {
                self.frames_read += 1;
                return Ok(Some(frame));
            }
            let n = self.stdout.read(&mut chunk)?;
            if n == 0 {
                return Ok(None); // EOF
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }
}

impl Drop for MjpegSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Extrae el primer JPEG completo (FFD8 … FFD9) del acumulador, si lo hay.
/// El escaneo de FFD9 es seguro en MJPEG de ffmpeg: dentro de los datos
/// entrópicos los 0xFF van escapados (FF00) y los únicos marcadores sueltos
/// son los de restart (FFD0–FFD7).
pub fn extract_jpeg(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    let start = buf.windows(2).position(|w| w == [0xFF, 0xD8])?;
    let end_rel = buf[start..].windows(2).position(|w| w == [0xFF, 0xD9])?;
    let end = start + end_rel + 2;
    let frame = buf[start..end].to_vec();
    buf.drain(..end);
    Some(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_jpeg_handles_partial_and_multiple() {
        let jpeg_a: Vec<u8> = [0xFF, 0xD8, 1, 2, 3, 0xFF, 0xD9].to_vec();
        let jpeg_b: Vec<u8> = [0xFF, 0xD8, 9, 8, 0xFF, 0x00, 0xD9, 7, 0xFF, 0xD9].to_vec();

        // parcial: sin FFD9 todavía → None y el buffer queda intacto
        let mut buf = jpeg_a[..4].to_vec();
        assert!(extract_jpeg(&mut buf).is_none());
        assert_eq!(buf.len(), 4);

        // completo + siguiente parcial
        let mut buf = [jpeg_a.clone(), jpeg_b[..3].to_vec()].concat();
        assert_eq!(extract_jpeg(&mut buf).unwrap(), jpeg_a);
        assert!(extract_jpeg(&mut buf).is_none());

        // el resto de b llega → se extrae b entero (con FF00 escapado dentro)
        buf.extend_from_slice(&jpeg_b[3..]);
        assert_eq!(extract_jpeg(&mut buf).unwrap(), jpeg_b);
        assert!(buf.is_empty());

        // basura antes del SOI se descarta
        let mut buf = [vec![0u8, 1, 2], jpeg_a.clone()].concat();
        assert_eq!(extract_jpeg(&mut buf).unwrap(), jpeg_a);
    }
}
