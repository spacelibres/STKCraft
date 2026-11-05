//! Reference frames and coordinate transforms for `STKCraft`.
//!
//! An orbit is meaningless without the frame it is expressed in, and turning a state
//! vector into something a 3D view can draw means walking a chain of frame conversions:
//! ICRF to GCRF, GCRF to the Earth-fixed frame through precession, nutation and Earth
//! rotation, and then to the local horizon of a ground station.
//!
//! This crate owns that chain. It built the frame identities and the fixed rotations
//! first; the time-dependent rotations (precession, nutation, polar motion) arrive as
//! separate, individually tested pieces.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

pub mod rotation;

pub use rotation::Rotation3;
