//! ue-ai: content analysis. v0: silence detection (PLAN §7.C).
//! Improved port of Youtubers-toolkit's `trim_by_silence`: fine RMS
//! windows, dual threshold with hysteresis, minimum durations and padding.

pub mod emotion;
pub mod silence;
