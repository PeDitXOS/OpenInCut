//! Salida cpal. Un hilo dedicado posee el stream (cpal::Stream no es Send);
//! el control viaja por atomics compartidos. El audio es el reloj maestro:
//! la posición se deriva de los frames servidos al dispositivo.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::mixer::{mix_frame, MixItem};
use crate::{frames_to_us, us_to_frames, AudioError, AudioResult, RATE};

/// Posición en punto fijo 48k-frames << 16 (para pasos fraccionales al
/// convertir la tasa del dispositivo).
const FP: i64 = 1 << 16;

struct Shared {
    playing: AtomicBool,
    pos_fp: AtomicI64,
    items: Mutex<Arc<Vec<MixItem>>>,
    items_version: AtomicU64,
    /// RMS del último buffer servido, por canal (bits de f32).
    meter_l: AtomicU32,
    meter_r: AtomicU32,
}

pub struct Player {
    shared: Arc<Shared>,
    _thread: std::thread::JoinHandle<()>,
    pub device_rate: u32,
}

impl Player {
    pub fn new() -> AudioResult<Player> {
        let shared = Arc::new(Shared {
            playing: AtomicBool::new(false),
            pos_fp: AtomicI64::new(0),
            items: Mutex::new(Arc::new(vec![])),
            items_version: AtomicU64::new(0),
            meter_l: AtomicU32::new(0),
            meter_r: AtomicU32::new(0),
        });
        let shared2 = shared.clone();
        let (tx, rx) = mpsc::channel::<AudioResult<u32>>();

        let thread = std::thread::Builder::new()
            .name("ue-audio".into())
            .spawn(move || {
                let stream_result = build_stream(shared2);
                match stream_result {
                    Ok((stream, rate)) => {
                        if stream.play().is_err() {
                            let _ = tx.send(Err(AudioError::Cpal("stream.play() falló".into())));
                            return;
                        }
                        let _ = tx.send(Ok(rate));
                        // mantener vivo el stream para siempre
                        let _stream = stream;
                        loop {
                            std::thread::park();
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                    }
                }
            })
            .map_err(|e| AudioError::Cpal(e.to_string()))?;

        let device_rate = rx
            .recv()
            .map_err(|_| AudioError::Cpal("el hilo de audio murió".into()))??;
        Ok(Player { shared, _thread: thread, device_rate })
    }

    pub fn play(&self, from_us: i64) {
        self.shared.pos_fp.store(us_to_frames(from_us) * FP, Ordering::SeqCst);
        self.shared.playing.store(true, Ordering::SeqCst);
    }

    pub fn pause(&self) -> i64 {
        self.shared.playing.store(false, Ordering::SeqCst);
        self.position_us()
    }

    pub fn seek(&self, us: i64) {
        self.shared.pos_fp.store(us_to_frames(us) * FP, Ordering::SeqCst);
    }

    pub fn position_us(&self) -> i64 {
        frames_to_us(self.shared.pos_fp.load(Ordering::SeqCst) / FP)
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::SeqCst)
    }

    pub fn set_items(&self, items: Vec<MixItem>, version: u64) {
        *self.shared.items.lock().unwrap() = Arc::new(items);
        self.shared.items_version.store(version, Ordering::SeqCst);
    }

    pub fn items_version(&self) -> u64 {
        self.shared.items_version.load(Ordering::SeqCst)
    }

    /// RMS (0..1) por canal del último buffer de audio servido.
    pub fn meters(&self) -> (f32, f32) {
        (
            f32::from_bits(self.shared.meter_l.load(Ordering::Relaxed)),
            f32::from_bits(self.shared.meter_r.load(Ordering::Relaxed)),
        )
    }
}

fn build_stream(shared: Arc<Shared>) -> AudioResult<(cpal::Stream, u32)> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or(AudioError::NoDevice)?;
    let config = device
        .default_output_config()
        .map_err(|e| AudioError::Cpal(e.to_string()))?;
    let rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    // paso fraccional: cuántos frames fuente (48k) avanza cada frame del dispositivo
    let step_fp = (RATE as i64 * FP) / rate as i64;

    let stream = device
        .build_output_stream(
            &config.into(),
            move |out: &mut [f32], _| {
                if !shared.playing.load(Ordering::Relaxed) {
                    out.fill(0.0);
                    shared.meter_l.store(0, Ordering::Relaxed);
                    shared.meter_r.store(0, Ordering::Relaxed);
                    return;
                }
                let items = shared.items.lock().unwrap().clone();
                let mut fp = shared.pos_fp.load(Ordering::Relaxed);
                let (mut sq_l, mut sq_r, mut n) = (0.0f64, 0.0f64, 0u32);
                for frame in out.chunks_mut(channels) {
                    let (l, r) = mix_frame(&items, fp / FP);
                    sq_l += (l as f64) * (l as f64);
                    sq_r += (r as f64) * (r as f64);
                    n += 1;
                    match channels {
                        1 => frame[0] = (l + r) * 0.5,
                        _ => {
                            frame[0] = l;
                            frame[1] = r;
                            for extra in frame.iter_mut().skip(2) {
                                *extra = 0.0;
                            }
                        }
                    }
                    fp += step_fp;
                }
                shared.pos_fp.store(fp, Ordering::Relaxed);
                if n > 0 {
                    let rms_l = (sq_l / n as f64).sqrt() as f32;
                    let rms_r = (sq_r / n as f64).sqrt() as f32;
                    shared.meter_l.store(rms_l.to_bits(), Ordering::Relaxed);
                    shared.meter_r.store(rms_r.to_bits(), Ordering::Relaxed);
                }
            },
            |err| eprintln!("[ue-audio] error de stream: {err}"),
            None,
        )
        .map_err(|e| AudioError::Cpal(e.to_string()))?;
    Ok((stream, rate))
}
