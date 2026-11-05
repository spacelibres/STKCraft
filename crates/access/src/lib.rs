//! Access and coverage geometry for `STKCraft`.
//!
//! This is the layer that answers the question the product exists to answer: **when can this
//! satellite see that ground station, and for how long.** Everything here is a pure function
//! of positions and a time interval; nothing knows about orbits, files or windows.
//!
//! The pieces, in dependency order:
//!
//! - [`ellipsoid`] — the WGS-84 Earth, so a station sits where it really is.
//! - [`site`] — a geodetic station and the local east/north/up axes at it.
//! - [`geometry`] — elevation, azimuth and range from a site to a target.
//! - [`sensor`] — the acceptance region around those angles.
//! - [`occultation`] — whether the Earth is in the way.
//! - [`window`] — turning a visibility predicate into start and end times.
//!
//! # Accuracy, stated rather than implied
//!
//! The line of sight is the **geometric straight line**. There is no atmospheric refraction,
//! no light-time correction and no aberration:
//!
//! - **Refraction** is zero above about `5°` elevation and reaches half a degree at the
//!   horizon. It is not modelled, so a horizon mask at or near `0°` is a geometric horizon,
//!   not an optical one.
//! - **Light time** is at most `0.24 s` for a geostationary satellite, which moves the answer
//!   by a fraction of an arcsecond. Not modelled.
//!
//! Both are real omissions and are named here so a result is not read as more precise than it
//! is. Neither is approximated with a plausible-looking formula.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

pub mod ellipsoid;
pub mod geometry;
pub mod occultation;
pub mod planet;
pub mod problem;
pub mod sensor;
pub mod site;
pub mod window;

pub use ellipsoid::Ellipsoid;
pub use geometry::{Topocentric, look_angle};
pub use occultation::is_occulted;
pub use planet::PlanetAccess;
pub use problem::{AccessProblem, fixed_target_windows};
pub use sensor::Sensor;
pub use site::{GroundSite, LocalFrame};
pub use window::{AccessWindow, DEFAULT_STEP, find_windows};
