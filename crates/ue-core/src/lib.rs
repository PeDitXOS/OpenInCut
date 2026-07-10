//! ue-core: project data model, reversible actions, history, and keyframe
//! evaluation. This crate is pure: no media IO, no GPU, no Tauri.

pub mod action;

/// Debug logging: on in debug builds, or with UE_DEBUG=1 in release.
/// UE_DEBUG=0 silences debug builds.
pub fn debug_enabled() -> bool {
    match std::env::var("UE_DEBUG") {
        Ok(v) => !matches!(v.as_str(), "0" | "off" | ""),
        Err(_) => cfg!(debug_assertions),
    }
}

/// One debug line with uptime, e.g. `[  12.345] [edit] Split clip (2 actions)`.
pub fn dlog(category: &str, msg: &str) {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    if debug_enabled() {
        let t = START.get_or_init(Instant::now).elapsed().as_secs_f64();
        eprintln!("[{t:>9.3}] [{category}] {msg}");
    }
}
pub mod error;
pub mod history;
pub mod keyframe;
pub mod model;
pub mod ops;
pub mod store;
pub mod time;
pub mod validate;

pub use action::Action;
pub use error::UeError;
pub use model::*;
pub use store::ProjectStore;
pub use time::{TimeUs, US_PER_SEC};
