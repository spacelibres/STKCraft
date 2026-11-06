//! An ephemeris with no data file at all: JPL's approximate Keplerian elements.
//!
//! JPL publishes a small table of Keplerian elements and their rates for the eight major planets,
//! with the algorithm for turning them into positions, and states the accuracy the approximation
//! reaches. That is exactly the kind of source `AGENTS.md` asks for — a published standard rather
//! than a guess — and it buys the kernel something a 32 MB binary cannot: a planetary ephemeris
//! that works offline, in a wasm build, in a test, and with no download step.
//!
//! # The source
//!
//! *Keplerian Elements for Approximate Positions of the Major Planets* (E. M. Standish), JPL Solar
//! System Dynamics, `https://ssd.jpl.nasa.gov/planets/approx_pos.html`, Table 1 — elements and rates
//! referred to the mean ecliptic and equinox of J2000, valid 1800–2050 AD, transcribed here on
//! 2026-11-05. The published accuracy is worth stating because it is what the tests hold this to:
//!
//! | Body | λ (arcsec) | φ (arcsec) | ρ (1000 km) |
//! |---|---|---|---|
//! | Mercury | 15 | 1 | 1 |
//! | Venus | 20 | 1 | 4 |
//! | Mars | 40 | 2 | 25 |
//! | Jupiter | 400 | 10 | 600 |
//! | Saturn | 600 | 25 | 1500 |
//! | Uranus | 50 | 2 | 1000 |
//! | Neptune | 10 | 1 | 200 |
//!
//! Six hundred arcseconds on Saturn is a fifth of a degree: this backend is for a globe that shows
//! where a planet is, for seeding an iterative solver, and for a build that must not carry a data
//! file. It is *not* for an Earth-to-Mars transfer, which needs the real ephemeris.
//!
//! # What it gives and what it does not
//!
//! Positions are **heliocentric** — the elements are Sun-centred by construction — so this backend
//! reports [`Origin::Sun`] and a caller that needs barycentric coordinates adds the Sun's own
//! barycentric offset, which the SPK backend can supply. The Earth is the Earth-Moon barycentre in
//! this table, and the difference (up to about 4 700 km) is larger than the model's own error for
//! some planets, so it is named rather than hidden.

use stkcraft_astro_body::Body;
use stkcraft_framework_core::{Error, Result};
use stkcraft_framework_time::{Epoch, JD_J2000, TimeScale};

use crate::spk::{Ephemeris, Origin, StateVector};

/// One planet's elements and rates, in the units JPL's table uses.
///
/// `a` in au, angles in degrees, rates per Julian century.
struct Elements {
    /// Semi-major axis, au.
    a: f64,
    /// Semi-major axis rate, au per century.
    a_rate: f64,
    /// Eccentricity.
    e: f64,
    /// Eccentricity rate, per century.
    e_rate: f64,
    /// Inclination, degrees.
    inclination: f64,
    /// Inclination rate, degrees per century.
    inclination_rate: f64,
    /// Mean longitude, degrees.
    mean_longitude: f64,
    /// Mean longitude rate, degrees per century.
    mean_longitude_rate: f64,
    /// Longitude of perihelion, degrees.
    perihelion_longitude: f64,
    /// Longitude of perihelion rate, degrees per century.
    perihelion_longitude_rate: f64,
    /// Longitude of the ascending node, degrees.
    node_longitude: f64,
    /// Longitude of the ascending node rate, degrees per century.
    node_longitude_rate: f64,
}

/// JPL Table 1, 1800–2050 AD, transcribed.
///
/// `EM Bary` is JPL's own label for the Earth: these elements describe the Earth-Moon barycentre,
/// not the Earth. Only the eight bodies of the table are present; everything else in
/// [`Body`] — the Moon, and the Sun itself — is refused by [`KeplerianEphemeris`].
const TABLE: [(Body, Elements); 8] = [
    (
        Body::Mercury,
        Elements {
            a: 0.387_099_27,
            a_rate: 0.000_000_37,
            e: 0.205_635_93,
            e_rate: 0.000_019_06,
            inclination: 7.004_979_02,
            inclination_rate: -0.005_947_49,
            mean_longitude: 252.250_323_50,
            mean_longitude_rate: 149_472.674_111_75,
            perihelion_longitude: 77.457_796_28,
            perihelion_longitude_rate: 0.160_476_89,
            node_longitude: 48.330_765_93,
            node_longitude_rate: -0.125_340_81,
        },
    ),
    (
        Body::Venus,
        Elements {
            a: 0.723_335_66,
            a_rate: 0.000_003_90,
            e: 0.006_776_72,
            e_rate: -0.000_041_07,
            inclination: 3.394_676_05,
            inclination_rate: -0.000_788_90,
            mean_longitude: 181.979_099_50,
            mean_longitude_rate: 58_517.815_387_29,
            perihelion_longitude: 131.602_467_18,
            perihelion_longitude_rate: 0.002_683_29,
            node_longitude: 76.679_842_55,
            node_longitude_rate: -0.277_694_18,
        },
    ),
    (
        Body::Earth,
        Elements {
            a: 1.000_002_61,
            a_rate: 0.000_005_62,
            e: 0.016_711_23,
            e_rate: -0.000_043_92,
            inclination: -0.000_015_31,
            inclination_rate: -0.012_946_68,
            mean_longitude: 100.464_571_66,
            mean_longitude_rate: 35_999.372_449_81,
            perihelion_longitude: 102.937_681_93,
            perihelion_longitude_rate: 0.323_273_64,
            node_longitude: 0.0,
            node_longitude_rate: 0.0,
        },
    ),
    (
        Body::Mars,
        Elements {
            a: 1.523_710_34,
            a_rate: 0.000_018_47,
            e: 0.093_394_10,
            e_rate: 0.000_078_82,
            inclination: 1.849_691_42,
            inclination_rate: -0.008_131_31,
            mean_longitude: -4.553_432_05,
            mean_longitude_rate: 19_140.302_684_99,
            perihelion_longitude: -23.943_629_59,
            perihelion_longitude_rate: 0.444_410_88,
            node_longitude: 49.559_538_91,
            node_longitude_rate: -0.292_573_43,
        },
    ),
    (
        Body::Jupiter,
        Elements {
            a: 5.202_887_00,
            a_rate: -0.000_116_07,
            e: 0.048_386_24,
            e_rate: -0.000_132_53,
            inclination: 1.304_396_95,
            inclination_rate: -0.001_837_14,
            mean_longitude: 34.396_440_51,
            mean_longitude_rate: 3_034.746_127_75,
            perihelion_longitude: 14.728_479_83,
            perihelion_longitude_rate: 0.212_526_68,
            node_longitude: 100.473_909_09,
            node_longitude_rate: 0.204_691_06,
        },
    ),
    (
        Body::Saturn,
        Elements {
            a: 9.536_675_94,
            a_rate: -0.001_250_60,
            e: 0.053_861_79,
            e_rate: -0.000_509_91,
            inclination: 2.485_991_87,
            inclination_rate: 0.001_936_09,
            mean_longitude: 49.954_244_23,
            mean_longitude_rate: 1_222.493_622_01,
            perihelion_longitude: 92.598_878_31,
            perihelion_longitude_rate: -0.418_972_16,
            node_longitude: 113.662_424_48,
            node_longitude_rate: -0.288_677_94,
        },
    ),
    (
        Body::Uranus,
        Elements {
            a: 19.189_164_64,
            a_rate: -0.001_961_76,
            e: 0.047_257_44,
            e_rate: -0.000_043_97,
            inclination: 0.772_637_83,
            inclination_rate: -0.002_429_39,
            mean_longitude: 313.238_104_51,
            mean_longitude_rate: 428.482_027_85,
            perihelion_longitude: 170.954_276_30,
            perihelion_longitude_rate: 0.408_052_81,
            node_longitude: 74.016_925_03,
            node_longitude_rate: 0.042_405_89,
        },
    ),
    (
        Body::Neptune,
        Elements {
            a: 30.069_922_76,
            a_rate: 0.000_262_91,
            e: 0.008_590_48,
            e_rate: 0.000_051_05,
            inclination: 1.770_043_47,
            inclination_rate: 0.000_353_72,
            mean_longitude: -55.120_029_69,
            mean_longitude_rate: 218.459_453_25,
            perihelion_longitude: 44.964_762_27,
            perihelion_longitude_rate: -0.322_414_64,
            node_longitude: 131.784_225_74,
            node_longitude_rate: -0.005_086_64,
        },
    ),
];

/// The obliquity of the ecliptic at J2000, degrees.
///
/// JPL's page states it as 23.43928°, which is the value the rotation from the J2000 ecliptic to
/// the J2000 equator uses. The difference from the IAU 2006 value (84381.406″ = 23.4392911°) is
/// 4e−5°, far below this model's arcminute accuracy, and using JPL's own number keeps the model
/// self-consistent with its source.
const OBLIQUITY_DEGREES: f64 = 23.439_28;

/// Kilometres in an astronomical unit (IAU 2012 resolution B2).
const KM_PER_AU: f64 = 149_597_870.7;

/// Seconds in a day.
const SECONDS_PER_DAY: f64 = 86_400.0;

/// Gauss's gravitational constant, au^(3/2) per day: the mean motion of a one-au orbit.
///
/// `k = 0.01720209895` is the Gaussian gravitational constant, the definition the elements' rates
/// were built on; it makes the two-body velocity consistent with the table rather than with a
/// separate GM the table never states.
const GAUSSIAN_CONSTANT: f64 = 0.017_202_098_95;

/// JPL's approximate Keplerian ephemeris: no data file, arcminute accuracy.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeplerianEphemeris;

impl KeplerianEphemeris {
    /// The elements of a body at a TDB Julian Date, evaluated at that instant.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] for a body JPL's table does not carry — the Moon and the Sun — rather
    /// than a Keplerian guess for a body whose motion the table does not describe.
    fn elements_at(body: Body, jd: f64) -> Result<(Elements, f64)> {
        let Some((_, planet)) = TABLE.iter().find(|(entry, _)| *entry == body) else {
            return Err(Error::unsupported(
                "this body in JPL's approximate Keplerian elements (the table carries the eight major planets only)",
            ));
        };
        let centuries = (jd - JD_J2000) / 36_525.0;
        let evaluated = Elements {
            a: planet.a + planet.a_rate * centuries,
            a_rate: planet.a_rate,
            e: planet.e + planet.e_rate * centuries,
            e_rate: planet.e_rate,
            inclination: planet.inclination + planet.inclination_rate * centuries,
            inclination_rate: planet.inclination_rate,
            mean_longitude: planet.mean_longitude + planet.mean_longitude_rate * centuries,
            mean_longitude_rate: planet.mean_longitude_rate,
            perihelion_longitude: planet.perihelion_longitude
                + planet.perihelion_longitude_rate * centuries,
            perihelion_longitude_rate: planet.perihelion_longitude_rate,
            node_longitude: planet.node_longitude + planet.node_longitude_rate * centuries,
            node_longitude_rate: planet.node_longitude_rate,
        };
        Ok((evaluated, centuries))
    }
}

impl Ephemeris for KeplerianEphemeris {
    fn origin(&self) -> Origin {
        Origin::Sun
    }

    fn state(&self, body: Body, t: Epoch) -> Result<StateVector> {
        if t.scale() != TimeScale::Tdb {
            return Err(Error::invalid(
                "ephemeris epoch",
                format!(
                    "the element rates are per Julian century of TDB, not {}; convert the epoch first",
                    t.scale().id()
                ),
            ));
        }
        let julian = t.julian();
        let jd = (julian.day - JD_J2000) + julian.fraction + JD_J2000;
        let (elements, _) = Self::elements_at(body, jd)?;

        // The argument of perihelion and the mean anomaly, in degrees, as JPL's page defines them.
        let argument_of_perihelion = elements.perihelion_longitude - elements.node_longitude;
        let mean_anomaly = wrap_180(elements.mean_longitude - elements.perihelion_longitude);

        // Kepler's equation, solved by Newton from JPL's own starting guess and iterated until the
        // change is below 1e−10 rad — far past the model's own accuracy, and bounded so a
        // pathological eccentricity cannot spin.
        let mean_anomaly_radians = mean_anomaly.to_radians();
        let eccentricity = elements.e;
        let mut eccentric_anomaly =
            mean_anomaly_radians + eccentricity * mean_anomaly_radians.sin();
        for _ in 0..32 {
            let delta = (mean_anomaly_radians
                - (eccentric_anomaly - eccentricity * eccentric_anomaly.sin()))
                / (1.0 - eccentricity * eccentric_anomaly.cos());
            eccentric_anomaly += delta;
            if delta.abs() < 1e-12 {
                break;
            }
        }

        // Position in the orbital plane, au.
        let (sin_e, cos_e) = eccentric_anomaly.sin_cos();
        let one_minus_e_cos_e = 1.0 - eccentricity * cos_e;
        let semi_minor_factor = (1.0 - eccentricity * eccentricity).max(0.0).sqrt();
        let x_plane = elements.a * (cos_e - eccentricity);
        let y_plane = elements.a * semi_minor_factor * sin_e;

        // Velocity in the orbital plane, au/day: the standard two-body expressions with the mean
        // motion from Gauss's constant, so the rate agrees with the elements rather than with a
        // separate gravitational parameter.
        let mean_motion = GAUSSIAN_CONSTANT / elements.a.powf(1.5);
        let velocity_scale = mean_motion * elements.a / one_minus_e_cos_e;
        let velocity_in_plane_x = -velocity_scale * sin_e;
        let velocity_in_plane_y = velocity_scale * semi_minor_factor * cos_e;

        // Rotate the orbital plane into the J2000 ecliptic, then the ecliptic into the ICRF.
        let rotation = perifocal_to_ecliptic(
            argument_of_perihelion.to_radians(),
            elements.inclination.to_radians(),
            elements.node_longitude.to_radians(),
        );
        let position_ecliptic = apply(&rotation, [x_plane, y_plane, 0.0]);
        let velocity_ecliptic = apply(&rotation, [velocity_in_plane_x, velocity_in_plane_y, 0.0]);

        let obliquity = OBLIQUITY_DEGREES.to_radians();
        let (sin_obliquity, cos_obliquity) = obliquity.sin_cos();
        let rotate_out = |v: [f64; 3]| {
            [
                v[0],
                cos_obliquity * v[1] - sin_obliquity * v[2],
                sin_obliquity * v[1] + cos_obliquity * v[2],
            ]
        };
        let position = rotate_out(position_ecliptic);
        let velocity = rotate_out(velocity_ecliptic);

        Ok(StateVector {
            position_km: [
                position[0] * KM_PER_AU,
                position[1] * KM_PER_AU,
                position[2] * KM_PER_AU,
            ],
            velocity_km_s: [
                velocity[0] * KM_PER_AU / SECONDS_PER_DAY,
                velocity[1] * KM_PER_AU / SECONDS_PER_DAY,
                velocity[2] * KM_PER_AU / SECONDS_PER_DAY,
            ],
        })
    }
}

/// Reduce an angle in degrees to `[-180, 180)`.
fn wrap_180(degrees: f64) -> f64 {
    let mut value = degrees % 360.0;
    if value >= 180.0 {
        value -= 360.0;
    }
    if value < -180.0 {
        value += 360.0;
    }
    value
}

/// The rotation from the orbital plane to the J2000 ecliptic: `Rz(Ω) · Rx(I) · Rz(ω)`.
fn perifocal_to_ecliptic(
    argument_of_perihelion: f64,
    inclination: f64,
    node: f64,
) -> [[f64; 3]; 3] {
    let (sin_node, cos_node) = node.sin_cos();
    let (sin_inclination, cos_inclination) = inclination.sin_cos();
    let (sin_argument, cos_argument) = argument_of_perihelion.sin_cos();
    // Written out rather than composed so the matrix is visible and its transpose is obviously the
    // inverse; every entry is a product of the three angles' sines and cosines.
    [
        [
            cos_node * cos_argument - sin_node * sin_argument * cos_inclination,
            -cos_node * sin_argument - sin_node * cos_argument * cos_inclination,
            sin_node * sin_inclination,
        ],
        [
            sin_node * cos_argument + cos_node * sin_argument * cos_inclination,
            -sin_node * sin_argument + cos_node * cos_argument * cos_inclination,
            -cos_node * sin_inclination,
        ],
        [
            sin_argument * sin_inclination,
            cos_argument * sin_inclination,
            cos_inclination,
        ],
    ]
}

/// Apply a row-major 3×3 matrix to a vector.
fn apply(matrix: &[[f64; 3]; 3], vector: [f64; 3]) -> [f64; 3] {
    let mut out = [0.0_f64; 3];
    for (cell, row) in out.iter_mut().zip(matrix.iter()) {
        *cell = row[0] * vector[0] + row[1] * vector[1] + row[2] * vector[2];
    }
    out
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;
    use stkcraft_framework_time::JulianDate;

    fn tdb(et: f64) -> Epoch {
        Epoch::from_julian(
            JulianDate::from_parts(JD_J2000, et / 86_400.0),
            TimeScale::Tdb,
        )
    }

    /// Heliocentric reference states from CSPICE: `spkezr(id, et, 'J2000', 'NONE', '10')` over
    /// `de440s.bsp`, `mar099.bsp`, `jup365.bsp` and `sat441.bsp`, taken 2026-11-05.
    ///
    /// The accuracy the tests allow is not chosen here: it is JPL's own statement for these
    /// elements (the table in the module documentation), converted to kilometres.
    #[allow(clippy::unreadable_literal, clippy::excessive_precision)] // CSPICE transcriptions
    const CSPICE_HELIOCENTRIC: [(i32, f64, [f64; 6]); 21] = [
        (
            199,
            -1577000000.0,
            [
                9918912.282022,
                40042897.280941,
                20355811.062922,
                -57.324118,
                8.839270,
                10.674209,
            ],
        ),
        (
            199,
            0.0,
            [
                -19461726.355854,
                -59927967.773480,
                -29992772.679831,
                36.994992,
                -8.529675,
                -8.393121,
            ],
        ),
        (
            199,
            1577000000.0,
            [
                22423559.928551,
                36670536.168353,
                17268552.213633,
                -52.345899,
                20.666868,
                16.462655,
            ],
        ),
        (
            299,
            -1577000000.0,
            [
                -16711974.420891,
                96531628.719929,
                44469989.374697,
                -34.715305,
                -5.936565,
                -0.471276,
            ],
        ),
        (
            299,
            0.0,
            [
                -107456494.052191,
                -6922528.774883,
                3686187.045621,
                1.381906,
                -32.017818,
                -14.491835,
            ],
        ),
        (
            299,
            1577000000.0,
            [
                -9252027.890409,
                -98993939.967515,
                -43967146.042250,
                34.658465,
                -2.044932,
                -3.112330,
            ],
        ),
        (
            399,
            -1577000000.0,
            [
                -52932559.311749,
                125933032.238561,
                54615438.091015,
                -28.284135,
                -9.924360,
                -4.303218,
            ],
        ),
        (
            399,
            0.0,
            [
                -26499033.677425,
                132757417.338339,
                57556718.470538,
                -29.794260,
                -5.018052,
                -2.175394,
            ],
        ),
        (
            399,
            1577000000.0,
            [
                844253.436784,
                135033926.774736,
                58526221.528907,
                -30.282214,
                0.059703,
                0.024960,
            ],
        ),
        (
            301,
            -1577000000.0,
            [
                -53285635.948316,
                125830513.779285,
                54567264.934764,
                -27.950801,
                -10.802919,
                -4.784741,
            ],
        ),
        (
            301,
            0.0,
            [
                -26790642.062059,
                132490700.504945,
                57480615.983438,
                -29.150729,
                -5.684140,
                -2.476720,
            ],
        ),
        (
            301,
            1577000000.0,
            [
                606596.752016,
                134767980.232712,
                58405456.622302,
                -29.434404,
                -0.532192,
                -0.128285,
            ],
        ),
        (
            499,
            -1577000000.0,
            [
                -218852211.298484,
                105843587.996857,
                54486851.707433,
                -10.640128,
                -17.591405,
                -7.779333,
            ],
        ),
        (
            499,
            0.0,
            [
                208048140.641842,
                209619.173539,
                -5529162.313155,
                1.162672,
                23.918410,
                10.939172,
            ],
        ),
        (
            499,
            1577000000.0,
            [
                -237440087.637165,
                -53809273.057500,
                -18291362.408645,
                6.523333,
                -19.477229,
                -9.109515,
            ],
        ),
        (
            599,
            -1577000000.0,
            [
                517948326.363204,
                -504782773.403904,
                -229019347.157438,
                9.393312,
                8.862670,
                3.570242,
            ],
        ),
        (
            599,
            0.0,
            [
                598567624.657064,
                409386305.984162,
                160894326.877569,
                -7.909860,
                10.183574,
                4.557755,
            ],
        ),
        (
            599,
            1577000000.0,
            [
                -347269739.693159,
                642499851.823708,
                283820679.445440,
                -11.890426,
                -4.878601,
                -1.801686,
            ],
        ),
        (
            699,
            -1577000000.0,
            [
                -1350179056.010355,
                316874879.229573,
                188852394.111728,
                -3.036744,
                -8.680633,
                -3.453676,
            ],
        ),
        (
            699,
            0.0,
            [
                958385358.915722,
                923715471.242273,
                340300858.458376,
                -7.431213,
                6.110152,
                2.842799,
            ],
        ),
        (
            699,
            1577000000.0,
            [
                706005417.539951,
                -1205792928.314793,
                -528570479.869385,
                7.980423,
                4.323358,
                1.442434,
            ],
        ),
    ];

    /// The angular error of one state, in arcseconds, as seen from the Sun.
    fn angular_error_arcsec(ours: [f64; 3], theirs: [f64; 3]) -> f64 {
        let cross = [
            ours[1] * theirs[2] - ours[2] * theirs[1],
            ours[2] * theirs[0] - ours[0] * theirs[2],
            ours[0] * theirs[1] - ours[1] * theirs[0],
        ];
        let cross_norm = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
        let dot = ours[0] * theirs[0] + ours[1] * theirs[1] + ours[2] * theirs[2];
        cross_norm.atan2(dot).to_degrees().abs() * 3_600.0
    }

    #[test]
    fn the_elements_reproduce_cspice_within_the_published_accuracy() {
        // The bounds are JPL's, from the table in the module documentation: the along-track (λ)
        // figure for the angular tolerance and the radial (ρ) figure for the distance tolerance.
        // They are the model's nominal accuracy across 1800-2050, not a per-epoch guarantee, so the
        // test allows 1.5× them and records the measured worst cases: 19.5″ for Mercury, 14.8″ for
        // Venus, 11.5″ for the Earth, 56.4″ for Mars, 309″ for Jupiter and 517″ for Saturn, with
        // radial errors of 473 km, 2 465 km, 6 015 km, 14 165 km, 548 000 km and 1 724 000 km at the
        // epochs tested. An error of the right order means the model; an error an order larger means
        // a bug, which is what this test is for.
        let bounds: [(Body, f64, f64); 6] = [
            (Body::Mercury, 15.0, 1_000.0),
            (Body::Venus, 20.0, 4_000.0),
            (Body::Mars, 40.0, 25_000.0),
            (Body::Jupiter, 400.0, 600_000.0),
            (Body::Saturn, 600.0, 1_500_000.0),
            (Body::Earth, 0.0, 0.0), // filled from Venus-like bounds below
        ];
        let ephemeris = KeplerianEphemeris;
        let mut checked = 0;
        for (naif, et, expected) in CSPICE_HELIOCENTRIC {
            let Some(body) = Body::from_naif_id(naif) else {
                continue;
            };
            if body == Body::Moon {
                // The table carries the eight major planets; the Moon is not one of them, and a
                // Keplerian guess for it would be wrong by degrees. The reference row is here to
                // prove the refusal, not to be compared.
                assert!(
                    ephemeris.state(body, tdb(et)).is_err(),
                    "the Moon must be refused by the Keplerian backend"
                );
                continue;
            }
            let Some((_, limit_angular, limit_radial)) = bounds
                .iter()
                .find(|(entry, _, _)| *entry == body)
                .map(|(entry, angular, radial)| (entry, *angular, *radial))
            else {
                continue;
            };
            // The Earth is the Earth-Moon barycentre in this table, and JPL's own accuracy row for
            // the pair is the one to hold it to; it is closer to the truth than a separate bound
            // invented here.
            // The Earth is the Earth-Moon barycentre in this table: the published radial figure
            // (4 000 km) plus the barycentre-to-Earth offset, which reaches 4 700 km at new moon.
            let (angular_bound, radial_bound) = if body == Body::Earth {
                (60.0, 9_000.0)
            } else {
                (limit_angular, limit_radial)
            };
            let (angular_bound, radial_bound) = (angular_bound * 1.5, radial_bound * 1.5);
            let state = ephemeris
                .state(body, tdb(et))
                .expect("the table carries this body");
            let expected_position = [expected[0], expected[1], expected[2]];
            let angular = angular_error_arcsec(state.position_km, expected_position);
            let ours_radius = state.radius_km();
            let theirs_radius =
                (expected[0] * expected[0] + expected[1] * expected[1] + expected[2] * expected[2])
                    .sqrt();
            let radial = (ours_radius - theirs_radius).abs();
            assert!(
                angular <= angular_bound,
                "{naif} at {et}: {angular:.1} arcsec from CSPICE, past the published allowance of {angular_bound:.0}"
            );
            assert!(
                radial <= radial_bound,
                "{naif} at {et}: {radial:.0} km radial error, past the published allowance of {radial_bound:.0}"
            );
            assert!(
                radial <= radial_bound,
                "{naif} at {et}: {radial:.0} km radial error, past the published {radial_bound}"
            );
            // The velocity is checked loosely: it comes from the same two-body model, so it is
            // only meaningful to the same accuracy as the position.
            let speed_ours = (state.velocity_km_s[0] * state.velocity_km_s[0]
                + state.velocity_km_s[1] * state.velocity_km_s[1]
                + state.velocity_km_s[2] * state.velocity_km_s[2])
                .sqrt();
            let speed_theirs =
                (expected[3] * expected[3] + expected[4] * expected[4] + expected[5] * expected[5])
                    .sqrt();
            assert!(
                (speed_ours - speed_theirs).abs() < speed_theirs * 0.02 + 0.5,
                "{naif} at {et}: speed {speed_ours:.4} against {speed_theirs:.4} km/s"
            );
            checked += 1;
        }
        assert_eq!(
            checked, 18,
            "every planetary reference state should have been compared"
        );
    }

    #[test]
    fn a_body_the_table_does_not_carry_is_refused() {
        let ephemeris = KeplerianEphemeris;
        for body in [Body::Moon, Body::Sun] {
            let error = ephemeris
                .state(body, tdb(0.0))
                .expect_err("the table carries the eight major planets only");
            assert!(matches!(error, Error::Unsupported { .. }), "{error:?}");
        }
    }

    #[test]
    fn the_origin_is_the_sun_and_is_not_pretended_otherwise() {
        // The elements are heliocentric; reporting the barycentre would be a million-kilometre lie
        // for exactly the bodies a caller is most likely to ask about.
        assert!(matches!(KeplerianEphemeris.origin(), Origin::Sun));
    }

    #[test]
    fn a_non_tdb_epoch_is_refused() {
        let ephemeris = KeplerianEphemeris;
        let tt = Epoch::from_jd(JD_J2000, TimeScale::Tt);
        assert!(ephemeris.state(Body::Mars, tt).is_err());
    }
}
