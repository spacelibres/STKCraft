//! The IAU-80 nutation series.
//!
//! Source: Seidelmann (ed.), *Explanatory Supplement to the Astronomical Almanac* (1992),
//! section 3.222, p111. The 106-term table is transcribed from ERFA `src/nut80.c`
//! (BSD-3-Clause), itself derived from the SOFA library, and is the same series Vallado
//! implements in `AstroLib.cpp :: nutation`; the two implementations agree to `3e-15` rad at
//! the worked-example instant, which is well below any real accuracy need and confirms the
//! transcription rather than merely asserting it.
//!
//! Coefficients are in units of 0.1 milliarcsecond, and their `t` terms in 0.1 mas per Julian
//! century of TT.

use stkcraft_core::{Error, Result};

use crate::arguments::iau_1980 as fundamental_arguments;

/// Arcseconds to radians.
const ARCSEC_TO_RAD: f64 = core::f64::consts::PI / 648_000.0;

/// The coefficients are in tenths of a milliarcsecond.
const TENTH_MAS_TO_RAD: f64 = ARCSEC_TO_RAD / 1e4;

/// One row of the IAU-80 series.
///
/// The first five fields are integer multiples of the Delaunay arguments `l`, `l'`, `F`, `D`
/// and `Omega`. The last four are the `delta psi` sine coefficients (constant and per century)
/// followed by the `delta epsilon` cosine coefficients (constant and per century), in 0.1 mas.
///
/// The table is stored as a flat tuple array rather than a struct because it is data, not
/// behaviour, and the field meanings are fixed by the published series.
#[allow(clippy::type_complexity)]
const TABLE: [(i32, i32, i32, i32, i32, f64, f64, f64, f64); 106] = [
    // 1-10
    (0, 0, 0, 0, 1, -171_996.0, -174.2, 92_025.0, 8.9),
    (0, 0, 0, 0, 2, 2_062.0, 0.2, -895.0, 0.5),
    (-2, 0, 2, 0, 1, 46.0, 0.0, -24.0, 0.0),
    (2, 0, -2, 0, 0, 11.0, 0.0, 0.0, 0.0),
    (-2, 0, 2, 0, 2, -3.0, 0.0, 1.0, 0.0),
    (1, -1, 0, -1, 0, -3.0, 0.0, 0.0, 0.0),
    (0, -2, 2, -2, 1, -2.0, 0.0, 1.0, 0.0),
    (2, 0, -2, 0, 1, 1.0, 0.0, 0.0, 0.0),
    (0, 0, 2, -2, 2, -13_187.0, -1.6, 5_736.0, -3.1),
    (0, 1, 0, 0, 0, 1_426.0, -3.4, 54.0, -0.1),
    // 11-20
    (0, 1, 2, -2, 2, -517.0, 1.2, 224.0, -0.6),
    (0, -1, 2, -2, 2, 217.0, -0.5, -95.0, 0.3),
    (0, 0, 2, -2, 1, 129.0, 0.1, -70.0, 0.0),
    (2, 0, 0, -2, 0, 48.0, 0.0, 1.0, 0.0),
    (0, 0, 2, -2, 0, -22.0, 0.0, 0.0, 0.0),
    (0, 2, 0, 0, 0, 17.0, -0.1, 0.0, 0.0),
    (0, 1, 0, 0, 1, -15.0, 0.0, 9.0, 0.0),
    (0, 2, 2, -2, 2, -16.0, 0.1, 7.0, 0.0),
    (0, -1, 0, 0, 1, -12.0, 0.0, 6.0, 0.0),
    (-2, 0, 0, 2, 1, -6.0, 0.0, 3.0, 0.0),
    // 21-30
    (0, -1, 2, -2, 1, -5.0, 0.0, 3.0, 0.0),
    (2, 0, 0, -2, 1, 4.0, 0.0, -2.0, 0.0),
    (0, 1, 2, -2, 1, 4.0, 0.0, -2.0, 0.0),
    (1, 0, 0, -1, 0, -4.0, 0.0, 0.0, 0.0),
    (2, 1, 0, -2, 0, 1.0, 0.0, 0.0, 0.0),
    (0, 0, -2, 2, 1, 1.0, 0.0, 0.0, 0.0),
    (0, 1, -2, 2, 0, -1.0, 0.0, 0.0, 0.0),
    (0, 1, 0, 0, 2, 1.0, 0.0, 0.0, 0.0),
    (-1, 0, 0, 1, 1, 1.0, 0.0, 0.0, 0.0),
    (0, 1, 2, -2, 0, -1.0, 0.0, 0.0, 0.0),
    // 31-40
    (0, 0, 2, 0, 2, -2_274.0, -0.2, 977.0, -0.5),
    (1, 0, 0, 0, 0, 712.0, 0.1, -7.0, 0.0),
    (0, 0, 2, 0, 1, -386.0, -0.4, 200.0, 0.0),
    (1, 0, 2, 0, 2, -301.0, 0.0, 129.0, -0.1),
    (1, 0, 0, -2, 0, -158.0, 0.0, -1.0, 0.0),
    (-1, 0, 2, 0, 2, 123.0, 0.0, -53.0, 0.0),
    (0, 0, 0, 2, 0, 63.0, 0.0, -2.0, 0.0),
    (1, 0, 0, 0, 1, 63.0, 0.1, -33.0, 0.0),
    (-1, 0, 0, 0, 1, -58.0, -0.1, 32.0, 0.0),
    (-1, 0, 2, 2, 2, -59.0, 0.0, 26.0, 0.0),
    // 41-50
    (1, 0, 2, 0, 1, -51.0, 0.0, 27.0, 0.0),
    (0, 0, 2, 2, 2, -38.0, 0.0, 16.0, 0.0),
    (2, 0, 0, 0, 0, 29.0, 0.0, -1.0, 0.0),
    (1, 0, 2, -2, 2, 29.0, 0.0, -12.0, 0.0),
    (2, 0, 2, 0, 2, -31.0, 0.0, 13.0, 0.0),
    (0, 0, 2, 0, 0, 26.0, 0.0, -1.0, 0.0),
    (-1, 0, 2, 0, 1, 21.0, 0.0, -10.0, 0.0),
    (-1, 0, 0, 2, 1, 16.0, 0.0, -8.0, 0.0),
    (1, 0, 0, -2, 1, -13.0, 0.0, 7.0, 0.0),
    (-1, 0, 2, 2, 1, -10.0, 0.0, 5.0, 0.0),
    // 51-60
    (1, 1, 0, -2, 0, -7.0, 0.0, 0.0, 0.0),
    (0, 1, 2, 0, 2, 7.0, 0.0, -3.0, 0.0),
    (0, -1, 2, 0, 2, -7.0, 0.0, 3.0, 0.0),
    (1, 0, 2, 2, 2, -8.0, 0.0, 3.0, 0.0),
    (1, 0, 0, 2, 0, 6.0, 0.0, 0.0, 0.0),
    (2, 0, 2, -2, 2, 6.0, 0.0, -3.0, 0.0),
    (0, 0, 0, 2, 1, -6.0, 0.0, 3.0, 0.0),
    (0, 0, 2, 2, 1, -7.0, 0.0, 3.0, 0.0),
    (1, 0, 2, -2, 1, 6.0, 0.0, -3.0, 0.0),
    (0, 0, 0, -2, 1, -5.0, 0.0, 3.0, 0.0),
    // 61-70
    (1, -1, 0, 0, 0, 5.0, 0.0, 0.0, 0.0),
    (2, 0, 2, 0, 1, -5.0, 0.0, 3.0, 0.0),
    (0, 1, 0, -2, 0, -4.0, 0.0, 0.0, 0.0),
    (1, 0, -2, 0, 0, 4.0, 0.0, 0.0, 0.0),
    (0, 0, 0, 1, 0, -4.0, 0.0, 0.0, 0.0),
    (1, 1, 0, 0, 0, -3.0, 0.0, 0.0, 0.0),
    (1, 0, 2, 0, 0, 3.0, 0.0, 0.0, 0.0),
    (1, -1, 2, 0, 2, -3.0, 0.0, 1.0, 0.0),
    (-1, -1, 2, 2, 2, -3.0, 0.0, 1.0, 0.0),
    (-2, 0, 0, 0, 1, -2.0, 0.0, 1.0, 0.0),
    // 71-80
    (3, 0, 2, 0, 2, -3.0, 0.0, 1.0, 0.0),
    (0, -1, 2, 2, 2, -3.0, 0.0, 1.0, 0.0),
    (1, 1, 2, 0, 2, 2.0, 0.0, -1.0, 0.0),
    (-1, 0, 2, -2, 1, -2.0, 0.0, 1.0, 0.0),
    (2, 0, 0, 0, 1, 2.0, 0.0, -1.0, 0.0),
    (1, 0, 0, 0, 2, -2.0, 0.0, 1.0, 0.0),
    (3, 0, 0, 0, 0, 2.0, 0.0, 0.0, 0.0),
    (0, 0, 2, 1, 2, 2.0, 0.0, -1.0, 0.0),
    (-1, 0, 0, 0, 2, 1.0, 0.0, -1.0, 0.0),
    (1, 0, 0, -4, 0, -1.0, 0.0, 0.0, 0.0),
    // 81-90
    (-2, 0, 2, 2, 2, 1.0, 0.0, -1.0, 0.0),
    (-1, 0, 2, 4, 2, -2.0, 0.0, 1.0, 0.0),
    (2, 0, 0, -4, 0, -1.0, 0.0, 0.0, 0.0),
    (1, 1, 2, -2, 2, 1.0, 0.0, -1.0, 0.0),
    (1, 0, 2, 2, 1, -1.0, 0.0, 1.0, 0.0),
    (-2, 0, 2, 4, 2, -1.0, 0.0, 1.0, 0.0),
    (-1, 0, 4, 0, 2, 1.0, 0.0, 0.0, 0.0),
    (1, -1, 0, -2, 0, 1.0, 0.0, 0.0, 0.0),
    (2, 0, 2, -2, 1, 1.0, 0.0, -1.0, 0.0),
    (2, 0, 2, 2, 2, -1.0, 0.0, 0.0, 0.0),
    // 91-100
    (1, 0, 0, 2, 1, -1.0, 0.0, 0.0, 0.0),
    (0, 0, 4, -2, 2, 1.0, 0.0, 0.0, 0.0),
    (3, 0, 2, -2, 2, 1.0, 0.0, 0.0, 0.0),
    (1, 0, 2, -2, 0, -1.0, 0.0, 0.0, 0.0),
    (0, 1, 2, 0, 1, 1.0, 0.0, 0.0, 0.0),
    (-1, -1, 0, 2, 1, 1.0, 0.0, 0.0, 0.0),
    (0, 0, -2, 0, 1, -1.0, 0.0, 0.0, 0.0),
    (0, 0, 2, -1, 2, -1.0, 0.0, 0.0, 0.0),
    (0, 1, 0, 2, 0, -1.0, 0.0, 0.0, 0.0),
    (1, 0, -2, -2, 0, -1.0, 0.0, 0.0, 0.0),
    // 101-106
    (0, -1, 2, 0, 1, -1.0, 0.0, 0.0, 0.0),
    (1, 1, 0, -2, 1, -1.0, 0.0, 0.0, 0.0),
    (1, 0, -2, 2, 0, -1.0, 0.0, 0.0, 0.0),
    (2, 0, 0, 2, 0, 1.0, 0.0, 0.0, 0.0),
    (0, 0, 2, 4, 2, -1.0, 0.0, 0.0, 0.0),
    (0, 1, 0, 1, 0, 1.0, 0.0, 0.0, 0.0),
];

/// Nutation in longitude and obliquity, radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Nutation {
    /// Nutation in longitude, `delta psi`.
    pub longitude: f64,
    /// Nutation in obliquity, `delta epsilon`.
    pub obliquity: f64,
}

/// The mean obliquity of the ecliptic, radians.
///
/// Source: Seidelmann (ed.), *Explanatory Supplement to the Astronomical Almanac* (1992),
/// section 3.222, the IAU-80 expression
/// `84381.448" - 46.8150" t - 0.00059" t^2 + 0.001813" t^3`.
///
/// # Errors
///
/// Returns [`Error::Invalid`] for a non-finite time.
pub fn mean_obliquity(ttt: f64) -> Result<f64> {
    if !ttt.is_finite() {
        return Err(Error::Invalid {
            what: "Julian centuries of TT",
            reason: "must be finite".to_owned(),
        });
    }
    let arcsec = 84_381.448 + (-46.8150 + (-0.000_59 + 0.001_813 * ttt) * ttt) * ttt;
    Ok(arcsec * ARCSEC_TO_RAD)
}

/// The IAU-80 nutation series, radians.
///
/// The terms are summed from the last to the first so that the largest (`Omega`) term is added
/// last. That is the order the published series is written in and the order both reference
/// implementations use; summing the other way changes the answer in the last few bits, which
/// is visible when comparing against a reference table to fifteen digits.
///
/// # Errors
///
/// Returns [`Error::Invalid`] for a non-finite time.
pub fn iau_1980(ttt: f64) -> Result<Nutation> {
    let arguments = fundamental_arguments(ttt)?;

    let mut delta_psi = 0.0_f64;
    let mut delta_eps = 0.0_f64;
    for &(nl, nlp, nf, nd, nom, sine, sine_rate, cosine, cosine_rate) in TABLE.iter().rev() {
        let argument = f64::from(nl) * arguments.moon_mean_longitude
            + f64::from(nlp) * arguments.sun_mean_longitude
            + f64::from(nf) * arguments.moon_argument_of_latitude
            + f64::from(nd) * arguments.moon_elongation
            + f64::from(nom) * arguments.moon_node_longitude;

        // A zero coefficient contributes nothing but still costs a `sin` or `cos`, and the
        // reference skips it; matching that keeps the summation order identical.
        let s = sine + sine_rate * ttt;
        if s != 0.0 {
            delta_psi += s * argument.sin();
        }
        let c = cosine + cosine_rate * ttt;
        if c != 0.0 {
            delta_eps += c * argument.cos();
        }
    }

    Ok(Nutation {
        longitude: delta_psi * TENTH_MAS_TO_RAD,
        obliquity: delta_eps * TENTH_MAS_TO_RAD,
    })
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    const TTT: f64 = 0.042_623_631_9;

    #[test]
    fn the_series_reproduces_two_independent_implementations() {
        // Source: Seidelmann (ed.), Explanatory Supplement (1992), section 3.222. The values
        // below were produced by Vallado `AstroLib.cpp :: nutation` and independently by ERFA
        // `eraNut80`; the two differ by 3.2e-15 rad in longitude and 1.4e-15 rad in obliquity,
        // so a tolerance of 1e-14 sits between them and any real transcription error.
        let n = iau_1980(TTT).expect("finite time");
        assert!(
            (n.longitude - -5.953_027_089_3e-5).abs() < 1e-14,
            "delta psi was {:e}",
            n.longitude
        );
        assert!(
            (n.obliquity - 3.545_876_334_9e-5).abs() < 1e-14,
            "delta epsilon was {:e}",
            n.obliquity
        );
    }

    #[test]
    fn nutation_stays_inside_its_published_envelope() {
        // The IAU-80 series is bounded by construction: |delta psi| < 20" and
        // |delta epsilon| < 10" for any time the series is valid for. A value outside that is
        // a corrupted table, not a large nutation.
        let bound_psi = 20.0 / 206_264.8;
        let bound_eps = 10.0 / 206_264.8;
        for year in 1900..=2100 {
            let ttt = (f64::from(year) - 2000.0) / 100.0;
            let n = iau_1980(ttt).expect("finite time");
            assert!(
                n.longitude.abs() < bound_psi,
                "delta psi in {year} was {}",
                n.longitude
            );
            assert!(
                n.obliquity.abs() < bound_eps,
                "delta epsilon in {year} was {}",
                n.obliquity
            );
        }
    }

    #[test]
    fn the_table_has_all_published_terms() {
        // The IAU-80 series has exactly 106 terms. A short table is a silent accuracy loss,
        // so the length is asserted rather than trusted. The first and last rows are checked
        // too, so a truncated or reordered paste cannot pass on length alone.
        assert_eq!(TABLE.len(), 106);
        let first = TABLE.first().copied().expect("non-empty");
        assert_eq!(first.0, 0);
        assert_eq!(first.4, 1);
        assert!((first.5 - -171_996.0).abs() < f64::EPSILON);
        let last = TABLE.last().copied().expect("non-empty");
        assert_eq!((last.0, last.1, last.2, last.3, last.4), (0, 1, 0, 1, 0));
        assert!((last.5 - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn mean_obliquity_matches_the_published_value() {
        // At J2000 the IAU-80 expression reduces to its leading term, 84381.448 arcseconds,
        // which is 0.4090928042223289 rad. Checked two ways: against the arcsecond value
        // directly, and against the reference implementation's own printed obliquity for the
        // worked-example instant, which exercises the polynomial terms as well.
        let at_j2000 = mean_obliquity(0.0).expect("finite time");
        let expected = 84_381.448 * ARCSEC_TO_RAD;
        assert!(
            (at_j2000 - expected).abs() < 1e-15,
            "obliquity at J2000 was {at_j2000}"
        );

        // Vallado `AstroLib.cpp :: nutation` prints meaneps = 0.409083130123 at this instant.
        let at_epoch = mean_obliquity(0.042_623_631_9).expect("finite time");
        assert!(
            (at_epoch - 0.409_083_130_123).abs() < 1e-12,
            "obliquity at the example epoch was {at_epoch}"
        );
    }

    #[test]
    fn a_non_finite_time_is_refused() {
        assert!(iau_1980(f64::NAN).is_err());
        assert!(iau_1980(f64::INFINITY).is_err());
        assert!(mean_obliquity(f64::NAN).is_err());
        assert!(mean_obliquity(f64::NEG_INFINITY).is_err());
    }
}
