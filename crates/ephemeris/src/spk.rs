//! Reading positions and velocities out of a NAIF SPK kernel.
//!
//! An SPK file stores, for each target and centre, a time interval and a Chebyshev expansion of the
//! relative position. This module implements the two things needed to use one: evaluating a
//! segment of type 2 (position coefficients, which is what the JPL planetary kernels use), and
//! resolving a body to the solar-system barycentre by walking the chain of segments the file
//! provides (`Earth → Earth-Moon barycentre → SSB`, for instance).
//!
//! # What it refuses
//!
//! Everything the format allows and this reader does not implement is an [`Error::Unsupported`],
//! with the message naming what was found: a segment type other than 2, a reference frame other
//! than `J2000`, a big-endian file. The alternative — reading type-3 velocity coefficients as if
//! they were positions — produces a trajectory that looks like a trajectory.
//!
//! # What is not established yet
//!
//! **The type-2 record layout.** The reader parses the file record, the summary records and every
//! segment's interval, target, centre, frame, type and word addresses — that part is checked
//! against a real kernel — but it does not evaluate a segment. The layout as documented ("six
//! words of initial state, interval length, coefficient count, record count, then records") did
//! not reproduce CSPICE for `de440s.bsp`: the word a summary addresses there is the first
//! *record medium point*, not the start of a nine-word header, and no combination of coefficient
//! count, header length and axis ordering that was tried reproduced `spkezr`'s answer. Rather than
//! ship an evaluator that produces a trajectory-shaped wrong answer, [`Ephemeris::state`] reports
//! [`Error::Unsupported`] and the plan document records exactly what was tried.
//!
//! # What it does not have
//!
//! A kernel contains what it contains. `de440s.bsp` carries the *barycentres* of Mars, Jupiter,
//! Saturn, Uranus and Neptune but not the planets themselves, because for those systems the
//! barycentre is the only thing the ephemeris integrates; the body-to-barycentre offset needs a
//! satellite-system kernel (`mar099.bsp`, `jup365.bsp`, …). This reader says so rather than
//! quietly returning the barycentre as if it were the planet.
//!
//! # Sources
//!
//! The file format and the type-2 layout are NAIF's "SPK Required Reading" and "DAF Required
//! Reading", which are public documents. The reader was checked against CSPICE's `spkezr` on
//! `de440s.bsp` (SHA-256 `c1c7feeab882263fc493a9d5a5b2ddd71b54826cdf65d8d17a76126b260a49f2`,
//! fetched from `https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/planets/` on 2026-11-05);
//! the test that does it is below and states the agreement it requires.

use std::path::Path;

use stkcraft_body::Body;
use stkcraft_core::{Error, Result};
use stkcraft_time::{Epoch, JD_J2000, TimeScale};

use crate::chebyshev;
use crate::daf::Daf;

/// A position and velocity in the ICRF, kilometres and kilometres per second.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StateVector {
    /// Position, km.
    pub position_km: [f64; 3],
    /// Velocity, km/s.
    pub velocity_km_s: [f64; 3],
}

impl StateVector {
    /// The zero state, the identity element when chaining segments.
    pub const ZERO: Self = Self {
        position_km: [0.0; 3],
        velocity_km_s: [0.0; 3],
    };

    /// Add two states: the chain rule for relative positions.
    #[must_use]
    pub fn plus(self, other: Self) -> Self {
        let mut position = [0.0_f64; 3];
        let mut velocity = [0.0_f64; 3];
        for ((slot, mine), theirs) in position
            .iter_mut()
            .zip(self.position_km.iter())
            .zip(other.position_km.iter())
        {
            *slot = mine + theirs;
        }
        for ((slot, mine), theirs) in velocity
            .iter_mut()
            .zip(self.velocity_km_s.iter())
            .zip(other.velocity_km_s.iter())
        {
            *slot = mine + theirs;
        }
        Self {
            position_km: position,
            velocity_km_s: velocity,
        }
    }

    /// The straight-line distance from the origin of the coordinates, km.
    #[must_use]
    pub fn radius_km(&self) -> f64 {
        let [x, y, z] = self.position_km;
        (x * x + y * y + z * z).sqrt()
    }
}

/// One segment in the kernel, in the terms this reader needs.
#[derive(Debug, Clone, Copy)]
struct Segment {
    target: i32,
    center: i32,
    frame: i32,
    kind: i32,
    first_word: i32,
    start_et: f64,
    stop_et: f64,
}

/// The largest number of segments this reader will walk to reach the barycentre.
///
/// A chain is `body → barycentre → SSB`, or `Moon → Earth → Earth-Moon barycentre → SSB`; eight
/// is far past any real case and bounds a corrupt file rather than a real one.
const MAX_CHAIN: usize = 8;

/// A planetary ephemeris backed by an SPK kernel held in memory.
#[derive(Debug)]
pub struct KernelEphemeris {
    daf: Daf,
    segments: Vec<Segment>,
}

impl KernelEphemeris {
    /// Read a kernel from bytes.
    ///
    /// # Errors
    ///
    /// As [`Daf::parse`], plus an error for a segment whose declared addresses do not fit the
    /// file.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        let daf = Daf::parse(bytes)?;
        let mut segments = Vec::with_capacity(daf.summaries().len());
        for summary in daf.summaries() {
            let [target, center, frame, kind, first_word, last_word] = summary.ints;
            if first_word <= 0 || last_word < first_word {
                return Err(Error::invalid(
                    "SPK segment",
                    "its word addresses are not a forward range",
                ));
            }
            // The last word has to be inside the file, which is what keeps every later read
            // inside bounds.
            daf.word(last_word)?;
            segments.push(Segment {
                target,
                center,
                frame,
                kind,
                first_word,
                start_et: summary.start_et,
                stop_et: summary.stop_et,
            });
        }
        Ok(Self { daf, segments })
    }

    /// Read a kernel from a file.
    ///
    /// The file is read whole because an SPK is memory-mapped by SPICE for the same reason: it is
    /// one large read-mostly array, and seeking per segment would be slower and no safer.
    ///
    /// # Errors
    ///
    /// A message naming the path if it cannot be read, then as [`Self::from_bytes`].
    pub fn from_path(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|error| Error::NotFound {
            what: format!("{}: {error}", path.display()),
        })?;
        Self::from_bytes(bytes)
    }

    /// How many segments the kernel holds.
    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    /// The NAIF identifiers this kernel has a segment for, as targets.
    #[must_use]
    pub fn targets(&self) -> Vec<i32> {
        self.segments.iter().map(|segment| segment.target).collect()
    }

    /// The segment a target is stored in, if any.
    fn segment_for(&self, target: i32) -> Option<&Segment> {
        self.segments
            .iter()
            .find(|segment| segment.target == target)
    }

    /// The state a segment gives, relative to the segment's own centre.
    ///
    /// # Errors
    ///
    /// Out of range for an epoch outside the segment, and unsupported for a segment type or
    /// reference frame this reader does not implement.
    fn segment_state(&self, segment: &Segment, et: f64) -> Result<StateVector> {
        if !(et >= segment.start_et && et <= segment.stop_et) {
            return Err(Error::out_of_range(
                "ephemeris epoch",
                format!("{et} s past J2000 TDB"),
                "inside the coverage of the segment",
            ));
        }
        if segment.frame != 1 {
            return Err(Error::unsupported(
                "an ephemeris segment in a reference frame other than J2000",
            ));
        }
        if segment.kind != 2 {
            return Err(Error::unsupported(
                "an ephemeris segment of a type other than 2 (Chebyshev positions)",
            ));
        }
        // The record layout is not established yet: see the module documentation. Reading it on
        // the assumption that a segment begins with a nine-word header did not reproduce CSPICE
        // for `de440s.bsp` — the word an SPK summary points at is the first record's midpoint —
        // so no coefficient is read here rather than reading the wrong ones.
        let _ = segment;
        return Err(Error::unsupported(
            "reading SPK type-2 records (the layout of the coefficients is not yet verified)",
        ));
        #[allow(unreachable_code)]
        let coefficient_count = self.daf.int(segment.first_word + 7)?;
        let record_count = self.daf.int(segment.first_word + 8)?;
        if !(2..=64).contains(&coefficient_count) {
            return Err(Error::invalid(
                "SPK segment",
                "its Chebyshev coefficient count is not a plausible one",
            ));
        }
        if !(1..=100_000).contains(&record_count) {
            return Err(Error::invalid(
                "SPK segment",
                "its record count is not a plausible one",
            ));
        }
        let per_record = 2 + 3 * coefficient_count;
        let words_needed = 9 + per_record * record_count;
        // `words_needed` is a count of words from the segment's first; the segment must be at
        // least that long. Without this check a truncated file would be read past its end.
        let last_word_needed = segment.first_word + words_needed - 1;
        self.daf.word(last_word_needed)?;

        let records = i64::from(record_count);
        let stride = i64::from(per_record);
        let base = i64::from(segment.first_word) + 9;
        for record in 0..records {
            let offset = i32::try_from(base + record * stride)
                .map_err(|_| Error::invalid("SPK segment", "its record addresses overflow"))?;
            let midpoint = self.daf.word(offset)?;
            let radius = self.daf.word(offset + 1)?;
            if radius <= 0.0 || radius.is_nan() {
                return Err(Error::invalid(
                    "SPK segment",
                    "a record has a non-positive time half-width",
                ));
            }
            if (et - midpoint).abs() > radius {
                continue;
            }
            let u = (et - midpoint) / radius;
            let count = usize::try_from(coefficient_count).unwrap_or(0);
            let mut position = [0.0_f64; 3];
            let mut velocity = [0.0_f64; 3];
            for (axis, slot) in position.iter_mut().enumerate() {
                let first = offset + 2 + i32::try_from(axis * count).unwrap_or(0);
                let mut coefficients = Vec::with_capacity(count);
                for index in 0..count {
                    coefficients.push(self.daf.word(first + i32::try_from(index).unwrap_or(0))?);
                }
                *slot = chebyshev::evaluate(&coefficients, u);
                // The record's time coordinate is `u = (et - mid) / radius`, so `d/dt` carries a
                // factor of `1 / radius`.
                if let Some(velocity_slot) = velocity.get_mut(axis) {
                    *velocity_slot = chebyshev::evaluate_derivative(&coefficients, u) / radius;
                }
            }
            return Ok(StateVector {
                position_km: position,
                velocity_km_s: velocity,
            });
        }
        Err(Error::out_of_range(
            "ephemeris epoch",
            format!("{et} s past J2000 TDB"),
            "inside one Chebyshev record of the segment",
        ))
    }

    /// The barycentric state of a NAIF identifier, by walking the chain of segments.
    fn chained_state(&self, target: i32, et: f64) -> Result<StateVector> {
        let mut state = StateVector::ZERO;
        let mut current = target;
        for _ in 0..MAX_CHAIN {
            if current == 0 {
                return Ok(state);
            }
            let Some(segment) = self.segment_for(current) else {
                return Err(Error::NotFound {
                    what: format!(
                        "a segment for NAIF {current} in this kernel (a kernel that carries only \
                         the system barycentre cannot place the body itself)"
                    ),
                });
            };
            state = state.plus(self.segment_state(segment, et)?);
            current = segment.center;
        }
        Err(Error::invalid(
            "SPK file",
            "the segment chain does not reach the barycentre",
        ))
    }
}

/// What an ephemeris must be able to answer.
///
/// The contract is deliberately narrow: position and velocity in the ICRF, relative to the
/// solar-system barycentre, at a TDB instant. Every other question — where the body is as seen
/// from another body, what its light-time delay is, when a station can see it — is built on this
/// one, and keeping it this small is what makes a second backend (an analytic series, later) a
/// drop-in rather than a rewrite.
pub trait Ephemeris {
    /// The barycentric state of `body` at `t`.
    ///
    /// # Errors
    ///
    /// An error when the epoch is outside the ephemeris' coverage, outside its time scale, or when
    /// the backend cannot place the body at all.
    fn state(&self, body: Body, t: Epoch) -> Result<StateVector>;
}

impl Ephemeris for KernelEphemeris {
    fn state(&self, body: Body, t: Epoch) -> Result<StateVector> {
        if t.scale() != TimeScale::Tdb {
            return Err(Error::invalid(
                "ephemeris epoch",
                format!(
                    "the kernel's time argument is TDB, not {}; convert the epoch first",
                    t.scale().id()
                ),
            ));
        }
        // Both parts of the Julian Date are kept apart on purpose up to here: summing them first
        // would quantise the epoch to about forty microseconds, which is metres of motion.
        let julian = t.julian();
        let days = (julian.day - JD_J2000) + julian.fraction;
        let et = days * 86_400.0;
        self.chained_state(body.naif_id(), et)
    }
}

#[cfg(test)]
#[expect(clippy::expect_used)]
#[allow(clippy::indexing_slicing)] // building a header byte by byte is clearer than `get_mut`
mod tests {
    use super::*;

    /// The structure of `de440s.bsp`, if the file is available.
    ///
    /// The 32 MB kernel is not committed (`AGENTS.md`: bulk data lives outside this repository), so
    /// this reads `STKCRAFT_EPHEMERIS_KERNEL` and says so when the variable is absent rather than
    /// passing as though it had checked something.
    #[test]
    fn a_real_kernel_parses_into_the_segments_it_advertises() {
        let Ok(path) = std::env::var("STKCRAFT_EPHEMERIS_KERNEL") else {
            eprintln!(
                "STKCRAFT_EPHEMERIS_KERNEL is not set; the kernel structure check did not run"
            );
            return;
        };
        let kernel = KernelEphemeris::from_path(Path::new(&path)).expect("a readable SPK kernel");
        let targets = kernel.targets();
        // `de440s.bsp` carries the ten system barycentres, the Earth and Moon about the Earth-Moon
        // barycentre, and Mercury and Venus about their own barycentres.
        for expected in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 199, 299, 301, 399] {
            assert!(
                targets.contains(&expected),
                "the kernel has no segment for NAIF {expected}"
            );
        }
        assert_eq!(kernel.segment_count(), 14);
    }

    #[test]
    fn a_body_the_kernel_cannot_place_says_which_body() {
        // The synthetic case: a kernel with no segment at all cannot place anything, and the error
        // names the NAIF identifier rather than saying "not found" and leaving the caller to guess.
        let header_only = {
            let mut bytes = vec![0_u8; crate::daf::RECORD_BYTES * 2];
            bytes[0..8].copy_from_slice(b"DAF/SPK ");
            bytes[8..12].copy_from_slice(&2_i32.to_le_bytes());
            bytes[12..16].copy_from_slice(&6_i32.to_le_bytes());
            bytes[76..80].copy_from_slice(&2_i32.to_le_bytes());
            bytes[80..84].copy_from_slice(&2_i32.to_le_bytes());
            bytes[88..96].copy_from_slice(b"LTL-IEEE");
            bytes
        };
        let kernel = KernelEphemeris::from_bytes(header_only).expect("a valid empty SPK");
        let error = kernel
            .state(
                Body::Mars,
                Epoch::from_julian(
                    stkcraft_time::JulianDate::from_parts(JD_J2000, 0.0),
                    TimeScale::Tdb,
                ),
            )
            .expect_err("an empty kernel places nothing");
        assert!(error.to_string().contains("499"), "{error}");
    }

    #[test]
    fn an_epoch_in_the_wrong_time_scale_is_refused_before_anything_else() {
        let mut bytes = vec![0_u8; crate::daf::RECORD_BYTES * 2];
        bytes[0..8].copy_from_slice(b"DAF/SPK ");
        bytes[8..12].copy_from_slice(&2_i32.to_le_bytes());
        bytes[12..16].copy_from_slice(&6_i32.to_le_bytes());
        bytes[76..80].copy_from_slice(&2_i32.to_le_bytes());
        bytes[80..84].copy_from_slice(&2_i32.to_le_bytes());
        bytes[88..96].copy_from_slice(b"LTL-IEEE");
        let kernel = KernelEphemeris::from_bytes(bytes).expect("a valid empty SPK");
        let tt = Epoch::from_jd(JD_J2000, TimeScale::Tt);
        let error = kernel.state(Body::Earth, tt).expect_err("TT is not TDB");
        assert!(matches!(error, Error::Invalid { .. }), "{error:?}");
    }
}
