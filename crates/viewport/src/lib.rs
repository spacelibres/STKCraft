//! The 3D viewport for `STKCraft`.
//!
//! This crate draws; it does not compute. Everything it shows arrives in a [`Frame`] that the
//! engine has already filled with positions, and the viewport's only job is to put them on the
//! screen. That split is deliberate and is the reason replacing this crate cannot change an
//! answer: an orbit drawn here is a polyline through propagated states, never a Keplerian ellipse
//! reconstructed from elements.
//!
//! ```text
//! engine ──Frame──▶ viewport ──wgpu──▶ screen
//!                          └──egui────▶ overlay
//! ```
//!
//! # Layers
//!
//! This crate sits at the UI layer, above [`LAST_NON_UI_LAYER`], which is what allows it to
//! depend on `wgpu` and `egui`. It depends on `stkcraft-core` for units and errors and on
//! nothing else from the engine, so the app can feed it from a live scenario, a saved file or a
//! test fixture with no change here.
//!
//! [`LAST_NON_UI_LAYER`]: ../../xtask/src/layers.rs

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

pub mod camera;
pub mod frame;
pub mod geometry;
pub mod renderer;
pub mod scene;

pub use camera::{Camera, Mat4};
pub use frame::{
    EARTH_EQUATORIAL_RADIUS_KM, EARTH_POLAR_RADIUS_KM, Frame, SatelliteGlyph, StationGlyph,
    color_from_bytes,
};
pub use geometry::{Mesh, Vertex};
