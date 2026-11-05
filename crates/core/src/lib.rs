//! Foundational types shared by every `STKCraft` crate.
//!
//! This crate is the bottom of the dependency graph (L0): units, error types and the
//! small value types that the time, frame and propagation layers all agree on. It has
//! no knowledge of orbits, scenarios or rendering.
//!
//! Units are newtypes rather than bare `f64` so that a call site cannot silently pass
//! degrees where radians are expected. Conversions are explicit and named.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

pub mod error;
pub mod units;
pub mod vector;

pub use error::{Error, Result};
pub use units::{Angle, Distance, Duration, EpochOffset, Mass, Velocity};
pub use vector::{Vector3, Vector3d};
