//! ue-core: modelo de datos del proyecto, acciones reversibles, historial y
//! evaluación de keyframes. Este crate es puro: sin IO de media, sin GPU, sin Tauri.

pub mod action;
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
