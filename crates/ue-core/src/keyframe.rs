//! Keyframe curves and evaluation. Times relative to the start of the clip (µs).

use serde::{Deserialize, Serialize};

use crate::time::TimeUs;

/// Interpolation of the SEGMENT that starts at this keyframe.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Interp {
    /// Holds the value until the next key.
    Hold,
    /// Linear interpolation.
    Linear,
    /// Cubic Hermite. Tangents in units of value/second; if missing they are
    /// computed automatically (Catmull-Rom style with flat endpoints).
    Smooth {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tan_out: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tan_in: Option<f64>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub t: TimeUs,
    pub value: f64,
    pub interp: Interp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyframeCurve {
    pub keys: Vec<Keyframe>,
}

/// Animatable parameter: constant or curve.
/// JSON: a plain number ⇔ Const; an object {keys: […]} ⇔ Curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Param {
    Const(f64),
    Curve(KeyframeCurve),
}

impl Param {
    pub fn eval(&self, t: TimeUs) -> f64 {
        match self {
            Param::Const(v) => *v,
            Param::Curve(c) => c.eval(t),
        }
    }
}

impl From<f64> for Param {
    fn from(v: f64) -> Self {
        Param::Const(v)
    }
}

impl KeyframeCurve {
    pub fn new(keys: Vec<Keyframe>) -> Self {
        Self { keys }
    }

    /// Evaluates the curve at `t` (µs relative to the clip).
    /// Before the first key it returns its value; after the last, the last one's.
    pub fn eval(&self, t: TimeUs) -> f64 {
        let keys = &self.keys;
        if keys.is_empty() {
            return 0.0;
        }
        if t <= keys[0].t {
            return keys[0].value;
        }
        let last = &keys[keys.len() - 1];
        if t >= last.t {
            return last.value;
        }
        // binary search for the segment [i, i+1] with keys[i].t <= t < keys[i+1].t
        let i = match keys.binary_search_by(|k| k.t.cmp(&t)) {
            Ok(exact) => return keys[exact].value,
            Err(ins) => ins - 1,
        };
        let (k0, k1) = (&keys[i], &keys[i + 1]);
        let dt_us = (k1.t - k0.t) as f64;
        let u = (t - k0.t) as f64 / dt_us;
        match k0.interp {
            Interp::Hold => k0.value,
            Interp::Linear => k0.value + (k1.value - k0.value) * u,
            Interp::Smooth { tan_out, tan_in } => {
                let dt_s = dt_us / 1_000_000.0;
                let m0 = tan_out.unwrap_or_else(|| self.auto_tangent(i));
                let m1 = tan_in.unwrap_or_else(|| self.auto_tangent(i + 1));
                hermite(k0.value, m0, k1.value, m1, u, dt_s)
            }
        }
    }

    /// Automatic tangent (Catmull-Rom; flat endpoints), in value/second.
    fn auto_tangent(&self, i: usize) -> f64 {
        let keys = &self.keys;
        if i == 0 || i + 1 >= keys.len() {
            return 0.0;
        }
        let prev = &keys[i - 1];
        let next = &keys[i + 1];
        let dt_s = (next.t - prev.t) as f64 / 1_000_000.0;
        if dt_s <= 0.0 {
            return 0.0;
        }
        (next.value - prev.value) / dt_s
    }

    /// Splits the curve at `offset` (µs relative to the clip) for a clip split.
    /// Returns (left, right-rebased-to-0). Boundary keys are inserted with the
    /// evaluated value to preserve value continuity.
    pub fn split(&self, offset: TimeUs) -> (KeyframeCurve, KeyframeCurve) {
        let boundary = self.eval(offset);
        let mut left: Vec<Keyframe> = self
            .keys
            .iter()
            .filter(|k| k.t < offset)
            .cloned()
            .collect();
        let mut right: Vec<Keyframe> = self
            .keys
            .iter()
            .filter(|k| k.t > offset)
            .map(|k| Keyframe { t: k.t - offset, ..k.clone() })
            .collect();
        // exact key at the boundary (if it existed, it's shared on both sides)
        let exact = self.keys.iter().find(|k| k.t == offset);
        let boundary_interp = exact.map(|k| k.interp).unwrap_or(Interp::Linear);
        left.push(Keyframe { t: offset, value: boundary, interp: boundary_interp });
        right.insert(0, Keyframe { t: 0, value: boundary, interp: boundary_interp });
        (KeyframeCurve::new(left), KeyframeCurve::new(right))
    }
}

fn hermite(p0: f64, m0: f64, p1: f64, m1: f64, u: f64, dt_s: f64) -> f64 {
    let u2 = u * u;
    let u3 = u2 * u;
    let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
    let h10 = u3 - 2.0 * u2 + u;
    let h01 = -2.0 * u3 + 3.0 * u2;
    let h11 = u3 - u2;
    h00 * p0 + h10 * m0 * dt_s + h01 * p1 + h11 * m1 * dt_s
}

impl Param {
    /// Splits a parameter for a clip split (Const is cloned as is).
    pub fn split(&self, offset: TimeUs) -> (Param, Param) {
        match self {
            Param::Const(v) => (Param::Const(*v), Param::Const(*v)),
            Param::Curve(c) => {
                let (l, r) = c.split(offset);
                (Param::Curve(l), Param::Curve(r))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lin(t: TimeUs, value: f64) -> Keyframe {
        Keyframe { t, value, interp: Interp::Linear }
    }

    #[test]
    fn eval_linear_and_edges() {
        let c = KeyframeCurve::new(vec![lin(0, 0.0), lin(1_000_000, 10.0)]);
        assert_eq!(c.eval(-5), 0.0);
        assert_eq!(c.eval(0), 0.0);
        assert_eq!(c.eval(500_000), 5.0);
        assert_eq!(c.eval(1_000_000), 10.0);
        assert_eq!(c.eval(2_000_000), 10.0);
    }

    #[test]
    fn eval_hold() {
        let c = KeyframeCurve::new(vec![
            Keyframe { t: 0, value: 1.0, interp: Interp::Hold },
            lin(1_000_000, 2.0),
        ]);
        assert_eq!(c.eval(999_999), 1.0);
        assert_eq!(c.eval(1_000_000), 2.0);
    }

    #[test]
    fn eval_smooth_endpoints_and_monotone_midpoint() {
        let c = KeyframeCurve::new(vec![
            Keyframe { t: 0, value: 0.0, interp: Interp::Smooth { tan_out: None, tan_in: None } },
            Keyframe { t: 1_000_000, value: 10.0, interp: Interp::Linear },
        ]);
        assert_eq!(c.eval(0), 0.0);
        assert_eq!(c.eval(1_000_000), 10.0);
        // with flat automatic tangents at the endpoints, the midpoint is 5 (symmetry)
        assert!((c.eval(500_000) - 5.0).abs() < 1e-9);
        // ease: near the start it stays below the straight line
        assert!(c.eval(150_000) < 1.5);
    }

    #[test]
    fn split_preserves_boundary_value() {
        let c = KeyframeCurve::new(vec![lin(0, 0.0), lin(2_000_000, 20.0)]);
        let (l, r) = c.split(500_000);
        assert!((l.eval(500_000) - 5.0).abs() < 1e-9);
        assert!((r.eval(0) - 5.0).abs() < 1e-9);
        // and the right half still reaches 20
        assert!((r.eval(1_500_000) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn param_json_shape() {
        let p: Param = 3.5.into();
        assert_eq!(serde_json::to_string(&p).unwrap(), "3.5");
        let c = Param::Curve(KeyframeCurve::new(vec![lin(0, 1.0)]));
        let s = serde_json::to_string(&c).unwrap();
        assert!(s.contains("keys"));
        let back: Param = serde_json::from_str(&s).unwrap();
        assert_eq!(back, c);
        let n: Param = serde_json::from_str("3.5").unwrap();
        assert_eq!(n, p);
    }
}
