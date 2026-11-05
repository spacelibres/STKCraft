//! Where the planets are.
//!
//! The solar-system kernel arc needs one question answered above all others: given a body and an
//! instant, where is it? This crate answers it through a single trait, [`Ephemeris`], so that a
//! second way of answering it — an analytic series with no data file, later — is a backend rather
//! than a rewrite.
//!
//! The backend that exists today reads a NAIF SPK kernel, the format JPL publishes the DE
//! ephemerides in. It is implemented from NAIF's public format documents and checked against
//! CSPICE over the same file; see [`spk`] for what that check requires and what the reader refuses
//! to guess at.
//!
//! # Coordinates and time
//!
//! A state is an ICRF position and velocity in kilometres and kilometres per second, relative to
//! the solar-system barycentre — the ICRF origin is the barycentre by definition, so "barycentric"
//! is not an extra convention here. The time argument is **TDB**, because that is what a kernel's
//! epochs are; an epoch in any other scale is refused rather than converted silently, since the
//! conversion is the caller's to make and `stkcraft-time` can make it exactly.
//!
//! # Data lives outside the repository
//!
//! No kernel is committed. `de440s.bsp` is 32 MB, it is fetched from NAIF with a pinned checksum
//! when it is wanted (`AGENTS.md`: bulk data lives outside this repository), and the tests that
//! need it say so when it is absent instead of passing as if they had run.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

pub mod chebyshev;
pub mod daf;
pub mod kepler;
pub mod spk;

pub use kepler::KeplerianEphemeris;
pub use spk::{Ephemeris, EphemerisSet, KernelEphemeris, Origin, StateVector};
