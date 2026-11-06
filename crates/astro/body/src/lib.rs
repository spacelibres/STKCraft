//! The bodies of the solar system, as the IAU describes them.
//!
//! This crate answers three questions about a body and nothing else:
//!
//! 1. **How big and how massive is it?** [`Body::gm_km3_s2`] and [`Body::radii_km`], from DE440 and
//!    the IAU working group's cartographic constants.
//! 2. **How is it oriented?** [`Body::orientation`] evaluates the IAU 2015 rotation model — pole
//!    right ascension and declination, and the prime meridian angle `W` — and
//!    [`Orientation::icrf_to_body_fixed`] turns that into the matrix that rotates ICRF vectors
//!    into the body-fixed frame.
//! 3. **Where on its surface is a point?** [`Ellipsoid`], with planetocentric and planetodetic
//!    (planetographic) latitudes converted against the body's own ellipsoid rather than a hard
//!    coded WGS-84.
//!
//! # Sources
//!
//! Constants and rotation models come from NAIF's `pck00011.tpc` and `gm_de440.tpc`, which are the
//! machine-readable realisations of the IAU 2015 rotation model (Archinal et al. 2018, CMDA
//! 130:22) and of the DE440 masses (Park et al. 2021, AJ 161:105). The tables in
//! [`generated`] are generated from those files and every one of them carries the file and its
//! checksum in the module comment.
//!
//! The transformation convention is not assumed. It is pinned by tests against CSPICE (`pxform`)
//! over the same kernel: the matrices this crate produces agree with CSPICE to 1e−11 for every
//! body whose model is fully transcribed, and the one body that is not (Mars) has a test that
//! records the known difference instead of hiding it. See
//! `docs/plans/2026-11-05-solar-system-kernel.md`.
//!
//! # What this crate does not do
//!
//! It does not know where a body *is* — that is the ephemeris crate's job — and it does not
//! propagate anything. Positions and orientations are separate questions with separate sources,
//! and keeping them apart is what lets each be verified against its own reference.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

mod generated;

pub mod ellipsoid;
pub mod rotation;

pub use ellipsoid::Ellipsoid;
pub use rotation::Orientation;

use stkcraft_time::Epoch;

/// The constants and the rotation model of one body, as the kernel states them.
///
/// Angles are in degrees. The pole polynomials are per Julian century of TDB and the prime
/// meridian polynomial is per Julian day of TDB, which is the kernel's own convention; converting
/// to radians happens once, in [`Body::orientation`].
pub(crate) struct BodyData {
    /// The NAIF identifier of the body's *centre of mass*.
    pub(crate) naif_id: i32,
    /// The name the IAU gives it, which is also the NAIF name.
    pub(crate) name: &'static str,
    /// The gravitational parameter, km³/s².
    pub(crate) gm_km3_s2: f64,
    /// Equatorial x, equatorial y and polar radii, km.
    pub(crate) radii_km: [f64; 3],
    /// Right ascension of the north pole: constant, per century, per century².
    pub(crate) pole_ra: [f64; 3],
    /// Declination of the north pole: constant, per century, per century².
    pub(crate) pole_dec: [f64; 3],
    /// Prime meridian `W`: constant, per day, per day².
    pub(crate) prime_meridian: [f64; 3],
    /// `(angle index, degrees)` pairs for the sine terms of the pole's right ascension.
    pub(crate) ra_terms: &'static [(u8, f64)],
    /// `(angle index, degrees)` pairs for the cosine terms of the pole's declination.
    pub(crate) dec_terms: &'static [(u8, f64)],
    /// `(angle index, degrees)` pairs for the sine terms of the prime meridian.
    pub(crate) pm_terms: &'static [(u8, f64)],
    /// The system's nutation-precession angles: `(constant, per century)`, degrees.
    pub(crate) angles: &'static [[f64; 2]],
}

/// A body of the solar system that this kernel models.
///
/// The eight planets, the Sun, the Moon, and seven of the larger moons of other planets: Phobos,
/// Deimos, Io, Europa, Ganymede, Callisto and Titan, plus Triton, whose ephemeris comes from its own
/// 2.1 GB kernel rather than the Neptune system file."
///
/// Serialises as its IAU name (`"Mars"`), not as an index: a document written today must load in a
/// build whose table has grown a body, and an index would silently point at the wrong planet.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum Body {
    /// The Sun.
    Sun,
    /// Mercury.
    Mercury,
    /// Venus.
    Venus,
    /// The Earth.
    Earth,
    /// The Moon.
    Moon,
    /// Mars.
    Mars,
    /// Jupiter.
    Jupiter,
    /// Saturn.
    Saturn,
    /// Uranus.
    Uranus,
    /// Neptune.
    Neptune,
    /// Phobos, the inner moon of Mars.
    Phobos,
    /// Deimos, the outer moon of Mars.
    Deimos,
    /// Io, moon of Jupiter.
    Io,
    /// Europa, moon of Jupiter.
    Europa,
    /// Ganymede, moon of Jupiter.
    Ganymede,
    /// Callisto, moon of Jupiter.
    Callisto,
    /// Titan, moon of Saturn.
    Titan,
    /// Triton, moon of Neptune.
    Triton,
}

impl Body {
    /// Every body this crate knows, in order from the Sun.
    pub const ALL: [Self; 18] = [
        Self::Sun,
        Self::Mercury,
        Self::Venus,
        Self::Earth,
        Self::Moon,
        Self::Mars,
        Self::Jupiter,
        Self::Saturn,
        Self::Uranus,
        Self::Neptune,
        Self::Phobos,
        Self::Deimos,
        Self::Io,
        Self::Europa,
        Self::Ganymede,
        Self::Callisto,
        Self::Titan,
        Self::Triton,
    ];

    /// The body's entry in the generated table.
    ///
    /// Destructured rather than indexed: a pattern over the array is checked against its length
    /// at compile time, so a table that lost a body stops the build instead of silently serving
    /// the wrong constants for every body after the gap.
    const fn data(self) -> &'static BodyData {
        let [
            sun,
            mercury,
            venus,
            earth,
            moon,
            mars,
            jupiter,
            saturn,
            uranus,
            neptune,
            phobos,
            deimos,
            io,
            europa,
            ganymede,
            callisto,
            titan,
            triton,
        ] = &generated::BODIES;
        match self {
            Self::Sun => sun,
            Self::Mercury => mercury,
            Self::Venus => venus,
            Self::Earth => earth,
            Self::Moon => moon,
            Self::Mars => mars,
            Self::Jupiter => jupiter,
            Self::Saturn => saturn,
            Self::Uranus => uranus,
            Self::Neptune => neptune,
            Self::Phobos => phobos,
            Self::Deimos => deimos,
            Self::Io => io,
            Self::Europa => europa,
            Self::Ganymede => ganymede,
            Self::Callisto => callisto,
            Self::Titan => titan,
            Self::Triton => triton,
        }
    }

    /// The NAIF identifier, which is what an ephemeris kernel is keyed on.
    #[must_use]
    pub const fn naif_id(self) -> i32 {
        self.data().naif_id
    }

    /// The body's name, as the IAU and NAIF spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.data().name
    }

    /// The gravitational parameter, km³/s².
    ///
    /// From DE440 (`gm_de440.tpc`): the body's own mass, not the mass of its satellite system,
    /// because that is what a body-centred orbit needs.
    #[must_use]
    pub const fn gm_km3_s2(self) -> f64 {
        self.data().gm_km3_s2
    }

    /// Equatorial x, equatorial y and polar radii in kilometres, from the IAU 2015 report.
    #[must_use]
    pub const fn radii_km(self) -> [f64; 3] {
        self.data().radii_km
    }

    /// The body's ellipsoid.
    #[must_use]
    pub const fn ellipsoid(self) -> Ellipsoid {
        Ellipsoid::from_radii(self.radii_km())
    }

    /// The body with the given NAIF identifier, if this crate models it.
    #[must_use]
    pub const fn from_naif_id(id: i32) -> Option<Self> {
        match id {
            10 => Some(Self::Sun),
            199 => Some(Self::Mercury),
            299 => Some(Self::Venus),
            399 => Some(Self::Earth),
            301 => Some(Self::Moon),
            499 => Some(Self::Mars),
            599 => Some(Self::Jupiter),
            699 => Some(Self::Saturn),
            799 => Some(Self::Uranus),
            899 => Some(Self::Neptune),
            401 => Some(Self::Phobos),
            402 => Some(Self::Deimos),
            501 => Some(Self::Io),
            502 => Some(Self::Europa),
            503 => Some(Self::Ganymede),
            504 => Some(Self::Callisto),
            606 => Some(Self::Titan),
            801 => Some(Self::Triton),
            _ => None,
        }
    }

    /// The body's orientation at a TDB instant, from the IAU 2015 rotation model.
    ///
    /// The independent variable is TDB, as `pck00011.tpc` states for the 2015 report; passing an
    /// epoch in another scale would be a silent error of a fraction of an arcsecond, which is
    /// exactly the kind of thing this crate is supposed to make impossible, so the caller has to
    /// bring a TDB epoch and the doc comment says so.
    ///
    /// # Numerals
    ///
    /// A non-finite Julian Date yields an orientation of zero angles rather than NaNs, for the
    /// same reason the time crate's TDB correction does: the failure belongs to the epoch.
    #[must_use]
    pub fn orientation(self, tdb: Epoch) -> Orientation {
        let data = self.data();
        let jd = tdb.julian().sum();
        if !jd.is_finite() {
            return Orientation {
                pole_ra: 0.0,
                pole_dec: 0.0,
                prime_meridian: 0.0,
            };
        }
        let centuries = (jd - stkcraft_time::JD_J2000) / stkcraft_time::DAYS_PER_JULIAN_CENTURY;
        let days = jd - stkcraft_time::JD_J2000;

        // The pole's right ascension and the prime meridian use sines, the declination uses
        // cosines. That asymmetry is not a mistake: it is how the IAU models are written, and
        // `pck00011.tpc`'s worked Jupiter example spells all three out. Getting it wrong is
        // invisible in a structural test and wrong by degrees on screen, which is why
        // `rotation`'s tests compare against CSPICE.
        let mut pole_ra = polynomial(data.pole_ra, centuries);
        let mut pole_dec = polynomial(data.pole_dec, centuries);
        let mut prime_meridian = polynomial(data.prime_meridian, days);

        for (index, coefficient) in data.ra_terms {
            pole_ra += coefficient * angle(data, *index, centuries).sin();
        }
        for (index, coefficient) in data.dec_terms {
            pole_dec += coefficient * angle(data, *index, centuries).cos();
        }
        for (index, coefficient) in data.pm_terms {
            prime_meridian += coefficient * angle(data, *index, centuries).sin();
        }

        Orientation {
            pole_ra: pole_ra.to_radians(),
            pole_dec: pole_dec.to_radians(),
            prime_meridian: prime_meridian.rem_euclid(360.0).to_radians(),
        }
    }
}

impl core::fmt::Display for Body {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.name())
    }
}

/// Evaluate `constant + rate·x + rate²·x²`.
fn polynomial(coefficients: [f64; 3], x: f64) -> f64 {
    coefficients[0] + coefficients[1] * x + coefficients[2] * x * x
}

/// One nutation-precession angle, in radians.
///
/// An index past the end of the table is zero rather than a panic: the table comes from a
/// generated file, and a mismatch between it and the coefficient list is a build-time defect that
/// a lenient zero makes visible as a slightly wrong pole rather than as a crash in the render loop.
fn angle(data: &BodyData, index: u8, centuries: f64) -> f64 {
    let entry = usize::from(index);
    data.angles
        .get(entry)
        .map_or(0.0, |pair| (pair[0] + pair[1] * centuries).to_radians())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_body_has_the_constants_a_body_centred_orbit_needs() {
        for body in Body::ALL {
            assert!(body.gm_km3_s2() > 0.0, "{body} has no GM");
            let radii = body.radii_km();
            assert!(
                radii
                    .iter()
                    .all(|radius| radius.is_finite() && *radius > 0.0),
                "{body} has a nonsense radius"
            );
            // Triaxial moons have `x > y` as often as not (Phobos is 13 by 11.4 by 9.1 km), so the
            // invariant is only that the polar radius is not the largest.
            assert!(
                radii[2] <= radii[0].max(radii[1]) * 1.001,
                "{body} has a polar radius larger than its equatorial ones"
            );
            assert_ne!(body.name(), "", "{body:?} has no name");
        }
    }

    #[test]
    fn naif_ids_match_the_ones_the_kernels_use() {
        // These are the identifiers an SPK kernel is keyed on, so a typo here would silently
        // fetch the barycentre instead of the body.
        for (body, id) in [
            (Body::Sun, 10),
            (Body::Mercury, 199),
            (Body::Venus, 299),
            (Body::Earth, 399),
            (Body::Moon, 301),
            (Body::Mars, 499),
            (Body::Jupiter, 599),
            (Body::Saturn, 699),
            (Body::Uranus, 799),
            (Body::Neptune, 899),
        ] {
            assert_eq!(body.naif_id(), id);
            assert_eq!(Body::from_naif_id(id), Some(body));
        }
        assert_eq!(
            Body::from_naif_id(3),
            None,
            "the Earth-Moon barycentre is not a body"
        );
    }

    #[test]
    fn the_gm_values_are_the_de440_ones() {
        // Spot checks straight out of `gm_de440.tpc`. The kinetic-energy scale of every orbit in
        // the app hangs off these, so a transcription slip has to fail here.
        assert!((Body::Sun.gm_km3_s2() - 1.327_124_400_412_794_2e11).abs() < 1e2);
        assert!((Body::Earth.gm_km3_s2() - 3.986_004_355_070_226_6e5).abs() < 1e-6);
        assert!((Body::Moon.gm_km3_s2() - 4_902.800_118_457_55).abs() < 1e-9);
        assert!((Body::Mars.gm_km3_s2() - 4.282_837_362_069_909e4).abs() < 1e-6);
    }

    #[test]
    fn a_nonsense_epoch_gives_zero_angles_rather_than_nans() {
        let epoch = Epoch::from_jd(f64::NAN, stkcraft_time::TimeScale::Tdb);
        let orientation = Body::Earth.orientation(epoch);
        assert_eq!(orientation.pole_ra, 0.0);
        assert_eq!(orientation.pole_dec, 0.0);
        assert_eq!(orientation.prime_meridian, 0.0);
    }
}
