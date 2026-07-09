//! Tiempo en microsegundos enteros (`TimeUs`) y cuantización a frames.

pub type TimeUs = i64;
pub const US_PER_SEC: i64 = 1_000_000;

/// Redondea `t` al frame más cercano de una secuencia con fps racional `(num, den)`
/// y devuelve el tiempo (en µs) de ese frame. Garantiza que los cortes caen en
/// frontera de frame exacta.
pub fn quantize_to_frame(t: TimeUs, fps: (u32, u32)) -> TimeUs {
    let frame = time_to_frame(t, fps);
    frame_to_time(frame, fps)
}

/// Frame más cercano a `t` (µs).
pub fn time_to_frame(t: TimeUs, fps: (u32, u32)) -> i64 {
    let (num, den) = (fps.0 as i128, fps.1 as i128);
    let numer = t as i128 * num;
    let denom = den * US_PER_SEC as i128;
    // división redondeada (t >= 0 en la práctica; soportamos negativos por robustez)
    let half = denom / 2;
    let r = if numer >= 0 {
        (numer + half) / denom
    } else {
        (numer - half) / denom
    };
    r as i64
}

/// Tiempo (µs) del frame `frame`.
pub fn frame_to_time(frame: i64, fps: (u32, u32)) -> TimeUs {
    let (num, den) = (fps.0 as i128, fps.1 as i128);
    let numer = frame as i128 * den * US_PER_SEC as i128;
    let half = num / 2;
    let r = if numer >= 0 {
        (numer + half) / num
    } else {
        (numer - half) / num
    };
    r as TimeUs
}

/// Duración de un frame en µs (redondeada), útil para mínimos.
pub fn frame_duration_us(fps: (u32, u32)) -> TimeUs {
    (frame_to_time(1, fps)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_roundtrip_ntsc() {
        let fps = (30000, 1001); // 29.97
        for f in [0i64, 1, 2, 29, 30, 1000, 54321] {
            let t = frame_to_time(f, fps);
            assert_eq!(time_to_frame(t, fps), f, "frame {f}");
            assert_eq!(quantize_to_frame(t, fps), t);
        }
    }

    #[test]
    fn quantize_snaps_to_nearest() {
        let fps = (30, 1);
        let frame_dur = frame_duration_us(fps); // 33_333
        // apenas por debajo de la mitad → frame 0; por encima → frame 1
        assert_eq!(quantize_to_frame(frame_dur / 2 - 10, fps), 0);
        assert_eq!(quantize_to_frame(frame_dur / 2 + 10, fps), frame_dur);
    }

    #[test]
    fn integer_fps() {
        let fps = (25, 1);
        assert_eq!(frame_to_time(25, fps), US_PER_SEC);
        assert_eq!(time_to_frame(US_PER_SEC, fps), 25);
    }
}
