//! The AIAA 2006-6753 verification cases.
//!
//! `data` is a byte-faithful transcription of the published `SGP4-VER.TLE` element sets and
//! the reference implementation's own output for them, taken from `tforverf.out` in the same
//! archive (`https://celestrak.org/publications/AIAA/2006-6753/AIAA-2006-6753.zip`). Neither
//! file has been edited: the TLE text keeps its column positions and the state vectors keep
//! all eight decimals the reference prints, which is why `data` allows unreadable literals.

#[allow(clippy::unreadable_literal)]
mod data;

pub use data::{CASES, Case};
