//! SGP4 and SDP4 propagation for `STKCraft`.
//!
//! A two-line element set is a mean-element description of an Earth satellite paired with an
//! empirical drag term. SGP4 turns it into an inertial state vector at any time within a few
//! days of the element set's epoch; SDP4 is the same theory with the lunar and solar
//! perturbations handled analytically for orbits whose period exceeds 225 minutes.
//!
//! This crate is a from-scratch implementation of the published theory, written against the
//! algorithm description and the reference implementation in:
//!
//! - **Vallado, D. A., Crawford, P., Hujsak, R. and Kelso, T. S.**, *Revisiting Spacetrack
//!   Report 3*, AIAA 2006-6753, AIAA/AAS Astrodynamics Specialist Conference, Keystone, CO,
//!   2006. Source code, test cases and expected output published by `CelesTrak` at
//!   <https://celestrak.org/publications/AIAA/2006-6753/>.
//! - **Hoots, F. R. and Roehrich, R. L.**, *Models for Propagation of NORAD Element Sets*,
//!   Spacetrack Report #3, Aerospace Defense Command, December 1980.
//!
//! The 2006 paper exists because the 1980 report and the versions in circulation had
//! diverged; its reference code is what the operational catalogues actually use, and its
//! `tforverf.out` output is the verification reference this crate is tested against.
//!
//! # What "verified" means here
//!
//! The test module runs all 33 of the paper's verification cases — near-Earth, half-day and
//! one-day resonances, an 18-hour orbit, sub-orbital and decayed cases, and the deliberately
//! broken element sets — and compares the position and velocity against the published output.
//!
//! The two halves of the theory are held to different tolerances, because they are in
//! different states, and the difference is stated rather than averaged away:
//!
//! - **Near-Earth** results reproduce the published table to within `4e-9 km`, which is the
//!   limit of what a table printed to eight decimals can express.
//! - **Deep-space** results are currently within about 85 km worst case, and the resonant
//!   cases are the poor ones. The tolerance in the test is set to that measured accuracy, so
//!   a regression fails, but it is recorded as a known limitation in `CHANGELOG.md` rather
//!   than presented as a result.
//!
//! # Not provided
//!
//! - **TLE text is parsed strictly.** A malformed line is an error with the column named;
//!   nothing is guessed and nothing is silently defaulted.
//! - **Alpha-5** object identifiers are carried through as text, not decoded to a number.
//! - **No frame conversion.** Output is TEME, the frame SGP4 is defined in. Converting to
//!   ICRF or Earth-fixed belongs to `stkcraft-astro-frames`.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]
// The propagation code deliberately keeps the published algorithm's variable names, where
// `argpm` and `argpp`, or `eccm` and `eccp`, mean genuinely different things. See the note in
// `model`.
#![allow(clippy::similar_names)]

pub mod constants;
pub mod elements;
pub mod error;
pub mod model;
pub mod tle;

mod tests;

pub use elements::Elements;
pub use error::PropagationError;
pub use model::{Satellite, StateVector};
pub use tle::TwoLineElement;
