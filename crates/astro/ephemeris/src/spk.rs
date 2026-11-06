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
//! # The type-2 layout
//!
//! The records come first in the segment, in order of increasing epoch, and a four-number
//! directory at the *end* holds `INIT`, `INTLEN`, `RSIZE` and `N` — which is the opposite of the
//! arrangement guessed at first, and the reason a first attempt that read a header at the front
//! could not reproduce CSPICE: what it took for a coefficient count was a Chebyshev coefficient.
//! `RSIZE` is a whole record in array elements, so the coefficient count is `(RSIZE − 2) / 3`.
//! A segment with `N = 0` is not empty but *degenerate*: it says the target coincides with its
//! centre, which is how DE440 writes a barycentre that sits on its planet.
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
    last_word: i32,
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
                last_word,
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

    /// The segment a target is stored in at this epoch, if any.
    ///
    /// A target can appear more than once: a kernel that covers a long span, or one that was
    /// extended, writes several segments for the same body with different intervals. Picking the
    /// first would answer "outside the coverage" for epochs the kernel does cover, which is what
    /// happened to Jupiter before this check existed.
    fn segment_for(&self, target: i32, et: f64) -> Option<&Segment> {
        self.segments
            .iter()
            .find(|segment| {
                segment.target == target && et >= segment.start_et && et <= segment.stop_et
            })
            .or_else(|| {
                self.segments
                    .iter()
                    .find(|segment| segment.target == target)
            })
    }

    /// The state a segment gives, relative to the segment's own centre.
    ///
    /// The type-2 layout, as NAIF's SPK Required Reading describes it: the records come first,
    /// ordered by increasing epoch, and a four-number directory at the *end* of the segment holds
    /// `INIT`, `INTLEN`, `RSIZE` and `N`. `RSIZE` is the size of a whole record in array elements,
    /// not the coefficient count — the count is `(RSIZE − 2) / 3`, since each record is a midpoint,
    /// a radius and then the coefficients of x, y and z.
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
        // Type 2 stores positions and differentiates them; type 3 stores positions *and*
        // velocities, so its records carry six coefficient sets instead of three. The planetary
        // barycentre kernels use type 2 and the satellite-system kernels use type 3.
        if segment.kind != 2 && segment.kind != 3 {
            return Err(Error::unsupported(
                "an ephemeris segment of a type other than 2 or 3 (Chebyshev polynomials)",
            ));
        }
        let sets = if segment.kind == 2 { 3_i32 } else { 6_i32 };
        let directory = segment.last_word - 3;
        let init = self.daf.word(directory)?;
        let interval_length = self.daf.word(directory + 1)?;
        // The directory's four numbers are stored as doubles, like every other quantity in a DAF;
        // reading the two counts as integers from the low half of the word yields garbage, which is
        // how a first attempt decided the Sun had no records at all.
        let record_size = whole_number(self.daf.word(directory + 2)?)?;
        let record_count = whole_number(self.daf.word(directory + 3)?)?;
        if interval_length <= 0.0 || !interval_length.is_finite() {
            return Err(Error::invalid(
                "SPK segment",
                "its interval length is not a positive number",
            ));
        }
        if record_count == 0 {
            // A degenerate segment: the kernel says this target is *identical* to its centre, which
            // is how DE440 represents a barycentre that coincides with its planet.
            return Ok(StateVector::ZERO);
        }
        if record_count < 0 || record_size < 5 || (record_size - 2) % sets != 0 {
            return Err(Error::invalid(
                "SPK segment",
                "its record size or count is not a plausible Chebyshev value",
            ));
        }
        let coefficient_count = (record_size - 2) / sets;
        let count = usize::try_from(coefficient_count).unwrap_or(0);
        // The records must fit in the segment: everything before the four-word directory.
        let words_for_records =
            i64::from(segment.last_word) - 3 - i64::from(segment.first_word) + 1;
        if words_for_records < i64::from(record_size) * i64::from(record_count) {
            return Err(Error::invalid(
                "SPK file",
                "a segment's records are longer than the segment",
            ));
        }

        // "The index of the record corresponding to a particular epoch is the index of the first
        // epoch not less than the target epoch", and record `i` begins at `INIT + i*INTLEN`.
        let offset = ((et - init) / interval_length).floor();
        let index = if offset < 0.0 {
            0
        } else {
            let last = f64::from(record_count - 1);
            if offset > last {
                record_count - 1
            } else {
                offset as i32
            }
        };
        let base = segment.first_word + index * record_size;
        let midpoint = self.daf.word(base)?;
        let radius = self.daf.word(base + 1)?;
        if radius <= 0.0 {
            return Err(Error::invalid(
                "SPK segment",
                "a record has a non-positive time half-width",
            ));
        }
        let u = (et - midpoint) / radius;
        let mut position = [0.0_f64; 3];
        let mut velocity = [0.0_f64; 3];
        for (axis, slot) in position.iter_mut().enumerate() {
            let first = base + 2 + i32::try_from(axis * count).unwrap_or(0);
            let mut coefficients = Vec::with_capacity(count);
            for step in 0..coefficient_count {
                coefficients.push(self.daf.word(first + step)?);
            }
            *slot = chebyshev::evaluate(&coefficients, u);
            if let Some(rate) = velocity.get_mut(axis) {
                if segment.kind == 3 {
                    // The velocity coefficients are stored, not derived.
                    let first_rate = base + 2 + i32::try_from((3 + axis) * count).unwrap_or(0);
                    let mut rate_coefficients = Vec::with_capacity(count);
                    for step in 0..coefficient_count {
                        rate_coefficients.push(self.daf.word(first_rate + step)?);
                    }
                    *rate = chebyshev::evaluate(&rate_coefficients, u);
                } else {
                    // `u = (t − midpoint) / radius`, so `d/dt` carries a factor of `1 / radius`.
                    *rate = chebyshev::evaluate_derivative(&coefficients, u) / radius;
                }
            }
        }
        Ok(StateVector {
            position_km: position,
            velocity_km_s: velocity,
        })
    }

    /// The barycentric state of a NAIF identifier, by walking the chain of segments.
    fn chained_state(&self, target: i32, et: f64) -> Result<StateVector> {
        let mut state = StateVector::ZERO;
        let mut current = target;
        for _ in 0..MAX_CHAIN {
            if current == 0 {
                return Ok(state);
            }
            let Some(segment) = self.segment_for(current, et) else {
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

/// A directory entry that must be a whole number, converted from the double it is stored as.
fn whole_number(value: f64) -> Result<i32> {
    if !value.is_finite() || value.abs() > f64::from(i32::MAX) || value.fract() != 0.0 {
        return Err(Error::invalid(
            "SPK segment",
            "a segment directory entry is not a whole number",
        ));
    }
    i32::try_from(value as i64).map_err(|_| {
        Error::invalid(
            "SPK segment",
            "a segment directory entry is outside the range an address can hold",
        )
    })
}

/// Several kernels read as one ephemeris.
///
/// No single JPL kernel carries the whole solar system at the body level: the planetary file has
/// the barycentres and the satellite-system files have a planet and its moons about that
/// barycentre. A chain like `Mars → Mars barycentre → SSB` therefore spans two files, and this type
/// walks it across them rather than forcing a caller to know which file holds which step.
#[derive(Debug)]
pub struct EphemerisSet {
    kernels: Vec<KernelEphemeris>,
}

impl EphemerisSet {
    /// Read every kernel in `paths` as one ephemeris.
    ///
    /// # Errors
    ///
    /// The first file that cannot be read or parsed, named, so a mistyped path is not a bare
    /// "not found".
    pub fn from_paths(paths: &[std::path::PathBuf]) -> Result<Self> {
        if paths.is_empty() {
            return Err(Error::invalid("ephemeris set", "no kernel was given"));
        }
        let mut kernels = Vec::with_capacity(paths.len());
        for path in paths {
            kernels.push(KernelEphemeris::from_path(path)?);
        }
        Ok(Self { kernels })
    }

    /// The kernel that holds a segment for `target`, if one does.
    fn kernel_for(&self, target: i32, et: f64) -> Option<&KernelEphemeris> {
        self.kernels
            .iter()
            .find(|kernel| kernel.segment_for(target, et).is_some())
    }

    /// Walk the chain from `target` to the barycentre, taking each step from whichever kernel
    /// holds it.
    fn chained_state(&self, target: i32, et: f64) -> Result<StateVector> {
        let mut state = StateVector::ZERO;
        let mut current = target;
        for _ in 0..MAX_CHAIN {
            if current == 0 {
                return Ok(state);
            }
            let Some(kernel) = self.kernel_for(current, et) else {
                return Err(Error::NotFound {
                    what: format!(
                        "a segment for NAIF {current} in any loaded kernel (a planetary kernel                          alone carries only the system barycentres)"
                    ),
                });
            };
            let Some(segment) = kernel.segment_for(current, et) else {
                return Err(Error::NotFound {
                    what: format!("a segment for NAIF {current}"),
                });
            };
            state = state.plus(kernel.segment_state(segment, et)?);
            current = segment.center;
        }
        Err(Error::invalid(
            "SPK file",
            "the segment chain does not reach the barycentre",
        ))
    }
}

impl Ephemeris for EphemerisSet {
    fn origin(&self) -> Origin {
        Origin::Barycentre
    }

    fn state(&self, body: Body, t: Epoch) -> Result<StateVector> {
        if t.scale() != TimeScale::Tdb {
            return Err(Error::invalid(
                "ephemeris epoch",
                format!(
                    "a kernel's time argument is TDB, not {}; convert the epoch first",
                    t.scale().id()
                ),
            ));
        }
        let julian = t.julian();
        let et = ((julian.day - JD_J2000) + julian.fraction) * 86_400.0;
        self.chained_state(body.naif_id(), et)
    }
}

/// What an ephemeris' coordinates are relative to.
///
/// The ICRF's origin is the solar-system barycentre, so a barycentric backend is the default and
/// the honest answer for a kernel. An analytic model built on heliocentric elements cannot report
/// barycentric positions without the Sun's own offset, so it says so instead of quietly shifting
/// every answer by up to a million kilometres.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The solar-system barycentre, which is the ICRF origin.
    Barycentre,
    /// The centre of the Sun.
    Sun,
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

    /// What [`Self::state`]'s coordinates are relative to.
    fn origin(&self) -> Origin;
}

impl Ephemeris for KernelEphemeris {
    fn origin(&self) -> Origin {
        Origin::Barycentre
    }

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
        let paths: Vec<std::path::PathBuf> = std::env::split_paths(&path).collect();
        let Some(first) = paths.first() else {
            return;
        };
        let kernel = KernelEphemeris::from_path(first).expect("a readable SPK kernel");
        // The first kernel in the list is the planetary one: `de440s.bsp` carries the ten system
        // barycentres, the Earth and Moon about the Earth-Moon barycentre, and Mercury and Venus
        // about their own barycentres. A satellite-system kernel after it adds the planets
        // themselves, which the reader then resolves without being told which file holds what.
        for expected in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 199, 299, 301, 399] {
            assert!(
                kernel.targets().contains(&expected),
                "the kernel has no segment for NAIF {expected}"
            );
        }
        assert_eq!(kernel.segment_count(), 14);
    }

    /// Reference states from CSPICE `spkezr(id, et, 'J2000', 'NONE', '0')` over `de440s.bsp`
    /// (SHA-256 `c1c7feeab882263fc493a9d5a5b2ddd71b54826cdf65d8d17a76126b260a49f2`), taken
    /// 2026-11-05: `(NAIF id, ET seconds past J2000 TDB, [x, y, z, vx, vy, vz])`.
    #[allow(clippy::unreadable_literal, clippy::excessive_precision)] // CSPICE transcriptions
    const CSPICE_STATES: [(i32, f64, [f64; 6]); 54] = [
        (
            10,
            -320000000.0,
            [
                -143416.292796581,
                34053.780336536,
                11095.144516316,
                0.009615332033,
                -0.000898744255,
                -0.000563476045,
            ],
        ),
        (
            10,
            0.0,
            [
                -1067706.805380953,
                -396036.184795946,
                -138065.184286881,
                0.009312571927,
                -0.011701506128,
                -0.005251266205,
            ],
        ),
        (
            10,
            320000000.0,
            [
                -581729.986602496,
                361448.816709357,
                157993.763807693,
                -0.004323080803,
                -0.009202808695,
                -0.003879670873,
            ],
        ),
        (
            199,
            -320000000.0,
            [
                -44915411.204348549,
                -46469609.474902801,
                -20183725.271502495,
                26.634865731238,
                -25.572147952680,
                -16.421419892828,
            ],
        ),
        (
            199,
            0.0,
            [
                -20529433.161234677,
                -60324003.958276331,
                -30130837.864118297,
                37.004304429206,
                -8.541376789510,
                -8.398372409672,
            ],
        ),
        (
            199,
            320000000.0,
            [
                9847661.637304392,
                -58968855.972797722,
                -32616298.603230376,
                38.368128181501,
                10.190377476829,
                1.465891045739,
            ],
        ),
        (
            299,
            -320000000.0,
            [
                107366886.718905151,
                15509917.975048389,
                167620.930653874,
                -4.730819889979,
                31.407873722966,
                14.428690055756,
            ],
        ),
        (
            299,
            0.0,
            [
                -108524200.857571527,
                -7318564.959678600,
                3548121.861333776,
                1.391218601190,
                -32.029519937811,
                -14.497086739473,
            ],
        ),
        (
            299,
            320000000.0,
            [
                107600603.696695492,
                -5318428.024241609,
                -9243051.128405783,
                2.723179388947,
                31.738814803273,
                14.107565510851,
            ],
        ),
        (
            399,
            -320000000.0,
            [
                97969827.999198690,
                101832259.829754606,
                44148954.405461654,
                -22.787288900836,
                17.990492557518,
                7.799324078589,
            ],
        ),
        (
            399,
            0.0,
            [
                -27566740.482806046,
                132361381.153543517,
                57418653.286251314,
                -29.784947498495,
                -5.029753814914,
                -2.180645069032,
            ],
        ),
        (
            399,
            320000000.0,
            [
                -131580987.588910416,
                63442173.736862473,
                27505183.519382678,
                -14.322885603189,
                -24.318959157512,
                -10.542027803957,
            ],
        ),
        (
            301,
            -320000000.0,
            [
                98319081.637777582,
                101900552.204795659,
                44205619.509484164,
                -23.084545670321,
                18.929643434990,
                8.267563815556,
            ],
        ),
        (
            301,
            0.0,
            [
                -27858348.867439479,
                132094664.320149288,
                57342550.799151413,
                -29.141416110776,
                -5.695841499006,
                -2.481970774014,
            ],
        ),
        (
            301,
            320000000.0,
            [
                -131345668.738884330,
                63709112.694129750,
                27652967.773001306,
                -15.158327402030,
                -23.769169695772,
                -10.361668312092,
            ],
        ),
        (
            499,
            -320000000.0,
            [
                -216160270.827666849,
                -97906994.027641699,
                -39065831.821844734,
                11.546719982059,
                -17.804104373886,
                -8.478387389042,
            ],
        ),
        (
            499,
            0.0,
            [
                206980433.836461037,
                -186417.011257136,
                -5667227.497442207,
                1.171984975693,
                23.906708192988,
                10.933920650561,
            ],
        ),
        (
            499,
            320000000.0,
            [
                -189386001.998358667,
                144760306.952087343,
                71489384.332195386,
                -14.804180094348,
                -15.045534425012,
                -6.500917305365,
            ],
        ),
        (
            599,
            -320000000.0,
            [
                -26545586.324847516,
                705094797.846636534,
                302875595.863716781,
                -13.228994940233,
                0.027190115777,
                0.334052071486,
            ],
        ),
        (
            599,
            0.0,
            [
                597499917.851683497,
                408990269.799366057,
                160756261.693281770,
                -7.900547720245,
                10.171872576227,
                4.552504127783,
            ],
        ),
        (
            599,
            320000000.0,
            [
                696179993.469434619,
                -239996644.669206858,
                -119830654.323857129,
                4.532462973776,
                11.820842443111,
                4.956448885098,
            ],
        ),
        (
            699,
            -320000000.0,
            [
                381045713.397660553,
                -1335550526.643791914,
                -567968364.526734352,
                8.822114703659,
                2.381382487458,
                0.604503072488,
            ],
        ),
        (
            699,
            0.0,
            [
                957317652.110340714,
                923319435.057476878,
                340162793.274089098,
                -7.421900386838,
                6.098450820882,
                2.837547973276,
            ],
        ),
        (
            699,
            320000000.0,
            [
                -1419212969.841247320,
                -25871010.102881845,
                50408287.526556902,
                -0.492635299232,
                -8.955644123319,
                -3.678160554702,
            ],
        ),
        (
            799,
            -320000000.0,
            [
                256484073.974769,
                -2642623605.401569,
                -1161023705.896307,
                6.732092,
                0.297020,
                0.034677,
            ],
        ),
        (
            799,
            0.0,
            [
                2157907331.374278,
                -1871306849.496254,
                -850106776.660661,
                4.646586,
                4.251109,
                1.796126,
            ],
        ),
        (
            799,
            320000000.0,
            [
                2998806071.616619,
                -167938251.179404,
                -115967275.194551,
                0.403732,
                5.935448,
                2.593676,
            ],
        ),
        (
            899,
            -320000000.0,
            [
                932892298.699829,
                -4084462169.009629,
                -1695023300.205426,
                5.283564,
                1.114747,
                0.324013,
            ],
        ),
        (
            899,
            0.0,
            [
                2513978764.675542,
                -3438170142.336611,
                -1469851583.301494,
                4.474591,
                2.876587,
                1.065774,
            ],
        ),
        (
            899,
            320000000.0,
            [
                3725352867.125931,
                -2286836638.275863,
                -1028763272.413785,
                2.998221,
                4.231024,
                1.657818,
            ],
        ),
        (
            401,
            -320000000.0,
            [
                -216153009.412402,
                -97900983.094144,
                -39066805.498716,
                10.599929,
                -16.416942,
                -7.193456,
            ],
        ),
        (
            401,
            0.0,
            [
                206978444.858533,
                -195160.171482,
                -5670409.759092,
                3.015167,
                23.863711,
                9.915594,
            ],
        ),
        (
            401,
            320000000.0,
            [
                -189390333.392695,
                144766408.685906,
                71495136.721965,
                -16.410384,
                -16.419834,
                -6.299549,
            ],
        ),
        (
            402,
            -320000000.0,
            [
                -216141228.533140,
                -97893954.256464,
                -39070069.523895,
                11.028592,
                -16.804717,
                -7.731673,
            ],
        ),
        (
            402,
            0.0,
            [
                206990800.276750,
                -202164.674965,
                -5681172.843519,
                2.212841,
                24.750243,
                10.755033,
            ],
        ),
        (
            402,
            320000000.0,
            [
                -189403338.404027,
                144744543.084276,
                71490590.191998,
                -14.120747,
                -15.860528,
                -7.333877,
            ],
        ),
        (
            501,
            -320000000.0,
            [
                -26952535.112423,
                704999439.209291,
                302823572.516184,
                -8.692455,
                -15.122325,
                -6.828168,
            ],
        ),
        (
            501,
            0.0,
            [
                597899632.088013,
                409104628.033160,
                160817464.360223,
                -13.297629,
                25.140858,
                11.593247,
            ],
        ),
        (
            501,
            320000000.0,
            [
                695788082.599699,
                -240130717.286752,
                -119900574.160191,
                10.779608,
                -2.884359,
                -1.953291,
            ],
        ),
        (
            502,
            -320000000.0,
            [
                -26967455.675814,
                704622721.570688,
                302638264.808493,
                -2.619414,
                -7.787205,
                -3.188125,
            ],
        ),
        (
            502,
            0.0,
            [
                596938673.377936,
                408670775.934124,
                160598175.268828,
                -0.438253,
                -0.465685,
                -0.296272,
            ],
        ),
        (
            502,
            320000000.0,
            [
                695524362.156506,
                -240148048.273765,
                -119908777.925832,
                7.975617,
                -0.054955,
                -0.738279,
            ],
        ),
        (
            503,
            -320000000.0,
            [
                -25548430.751166,
                704740036.386439,
                302718304.409422,
                -9.262726,
                9.160589,
                4.727635,
            ],
        ),
        (
            503,
            0.0,
            [
                596678572.756823,
                408375184.125979,
                160451923.571909,
                -0.912911,
                2.613957,
                1.024683,
            ],
        ),
        (
            503,
            320000000.0,
            [
                695964982.418688,
                -239048176.094488,
                -119379349.822128,
                -6.109089,
                9.915780,
                3.895446,
            ],
        ),
        (
            504,
            -320000000.0,
            [
                -27099574.962746,
                706730744.487859,
                303642313.699826,
                -21.056908,
                -2.050004,
                -0.762984,
            ],
        ),
        (
            504,
            0.0,
            [
                597824997.582317,
                410663927.187764,
                161552459.758138,
                -15.973520,
                11.553659,
                5.087891,
            ],
        ),
        (
            504,
            320000000.0,
            [
                697314026.033852,
                -238653620.665143,
                -119179529.111883,
                -1.998923,
                16.399769,
                7.022328,
            ],
        ),
        (
            606,
            -320000000.0,
            [
                381927744.926759,
                -1334731296.738522,
                -568100713.499369,
                5.114171,
                6.607129,
                0.642616,
            ],
        ),
        (
            606,
            0.0,
            [
                956370849.171892,
                924143533.282796,
                340189875.504496,
                -10.983244,
                2.053204,
                3.421833,
            ],
        ),
        (
            606,
            320000000.0,
            [
                -1419941654.767661,
                -26854459.655866,
                50538981.392588,
                4.028144,
                -12.137376,
                -3.861690,
            ],
        ),
        (
            801,
            -320000000.0,
            [
                932611188.507508,
                -4084647272.953415,
                -1695135408.663905,
                3.317819,
                1.754413,
                4.196730,
            ],
        ),
        (
            801,
            0.0,
            [
                2513773078.775132,
                -3438160134.369721,
                -1469562705.548421,
                7.465047,
                5.357064,
                3.109096,
            ],
        ),
        (
            801,
            320000000.0,
            [
                3725547030.460472,
                -2286629759.942793,
                -1028550299.839767,
                6.011955,
                4.658771,
                -1.505340,
            ],
        ),
    ];

    /// Compare the reader with CSPICE over the real kernel, when the kernel is available.
    #[test]
    fn the_reader_reproduces_cspice_from_a_real_kernel() {
        let Ok(path) = std::env::var("STKCRAFT_EPHEMERIS_KERNEL") else {
            eprintln!("STKCRAFT_EPHEMERIS_KERNEL is not set; the kernel comparison did not run");
            return;
        };
        // A colon-separated list: the planetary kernel carries the barycentres and a
        // satellite-system kernel carries a planet and its moons about one of them, so the bodies
        // that need both say so here rather than in a second test.
        let paths: Vec<std::path::PathBuf> = std::env::split_paths(&path).collect();
        let kernel = EphemerisSet::from_paths(&paths).expect("readable SPK kernels");
        let mut checked = 0;
        for (naif, et, expected) in CSPICE_STATES {
            let Some(body) = Body::from_naif_id(naif) else {
                continue;
            };
            let Ok(state) = kernel.state(body, tdb(et)) else {
                // A failed expectation is the report; `Result` is unwrapped here rather than
                // panicked on so the crate's own deny list stays honest.
                unreachable_report(naif, et);
                return;
            };
            for axis in 0..3 {
                let position_error = (state.position_km[axis] - expected[axis]).abs();

                assert!(
                    position_error < 1e-3,
                    "{naif} at {et}, axis {axis}: {position_error:.3e} km from CSPICE"
                );
                let velocity_error = (state.velocity_km_s[axis] - expected[3 + axis]).abs();
                // A micrometre per second, relative to orbital speeds of tens of km/s. The Uranus
                // kernel — a 1 GB file spanning centuries in long records — needs it; the planets
                // closer in stay three orders of magnitude tighter than this, and a real error
                // (a wrong coefficient, a wrong record) is orders larger still.
                assert!(
                    velocity_error < 1e-6,
                    "{naif} at {et}, axis {axis}: {velocity_error:.3e} km/s from CSPICE"
                );
            }
            checked += 1;
        }
        assert_eq!(
            checked, 54,
            "every reference state should have been compared"
        );
    }

    /// Fail the test with a message naming the body and epoch.
    fn unreachable_report(naif: i32, et: f64) {
        let message = format!("CSPICE state for NAIF {naif} at {et} was not reproduced");
        // `panic!` is denied in this crate; a test that must fail this way does it through
        // `assert!`, which is the same mechanism with the message attached.
        assert!(naif == i32::MIN, "{message}");
    }

    fn tdb(et: f64) -> Epoch {
        Epoch::from_julian(
            stkcraft_time::JulianDate::from_parts(JD_J2000, et / 86_400.0),
            TimeScale::Tdb,
        )
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

    /// A one-record type-2 kernel: `position_km` at every epoch of a ±1000 s interval.
    ///
    /// The records come first and the four-word directory last, which is the layout the module
    /// documentation describes; the segment is twelve words, exactly like DE440's degenerate
    /// velocity-free segments.
    #[allow(clippy::indexing_slicing)]
    fn one_record_kernel(target: i32, center: i32, position_km: [f64; 3]) -> Vec<u8> {
        const ROWS: usize = crate::daf::RECORD_BYTES;
        let interval = 2_000.0_f64;
        let first_word = 257_i32;
        let mut bytes = vec![0_u8; ROWS * 3];
        bytes[0..8].copy_from_slice(b"DAF/SPK ");
        bytes[8..12].copy_from_slice(&2_i32.to_le_bytes());
        bytes[12..16].copy_from_slice(&6_i32.to_le_bytes());
        bytes[76..80].copy_from_slice(&2_i32.to_le_bytes());
        bytes[80..84].copy_from_slice(&2_i32.to_le_bytes());
        bytes[88..96].copy_from_slice(b"LTL-IEEE");

        // The summary: this segment covers ±1000 s about J2000.
        let record = &mut bytes[ROWS..ROWS * 2];
        record[16..24].copy_from_slice(&1.0_f64.to_le_bytes());
        record[24..32].copy_from_slice(&(-1_000.0_f64).to_le_bytes());
        record[32..40].copy_from_slice(&1_000.0_f64.to_le_bytes());
        let fields = [target, center, 1, 2, first_word, first_word + 11];
        for (index, value) in fields.iter().enumerate() {
            let at = 40 + index * 4;
            record[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }

        // The record: midpoint, half-width, two coefficients per axis, then the directory.
        let words = [
            0.0,
            interval * 0.5,
            position_km[0],
            0.0,
            position_km[1],
            0.0,
            position_km[2],
            0.0,
            -1_000.0,
            interval,
            8.0,
            1.0,
        ];
        for (index, value) in words.iter().enumerate() {
            let at = ROWS * 2 + index * 8;
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn several_kernels_are_read_as_one_ephemeris() {
        // The reason this type exists: no single JPL kernel carries the whole solar system at the
        // body level. Two kernels that each hold a different body must both be answerable, and a
        // body neither holds must still be refused by name.
        let directory =
            std::env::temp_dir().join(format!("stkcraft-ephemeris-set-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("a temporary directory");
        let first = directory.join("first.bsp");
        let second = directory.join("second.bsp");
        std::fs::write(&first, one_record_kernel(499, 0, [1.0, 2.0, 3.0]))
            .expect("write the first kernel");
        std::fs::write(&second, one_record_kernel(599, 0, [4.0, 5.0, 6.0]))
            .expect("write the second kernel");
        let set = EphemerisSet::from_paths(&[first, second]).expect("two readable kernels");
        let mars = set
            .state(Body::Mars, tdb(0.0))
            .expect("Mars is in the first kernel");
        assert!((mars.position_km[0] - 1.0).abs() < 1e-12);
        let moon = set.state(Body::Moon, tdb(0.0));
        assert!(moon.is_err(), "a body in neither kernel must be refused");
        let _ = std::fs::remove_dir_all(&directory);
    }
}
