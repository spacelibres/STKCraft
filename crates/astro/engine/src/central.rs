//! Orbits about a body other than the Earth.
//!
//! SGP4 is an Earth model: its element sets are fitted to Earth's gravity and its frame chain is
//! the Earth's. A satellite about Mars or Titan needs a different propagator, and the honest first
//! one is the two-body problem — an exact closed-form solution, checkable by hand, with no
//! perturbation model that would be a guess about a gravity field this repository does not carry.
//!
//! # What is modelled, and what is not
//!
//! Position and velocity for a Keplerian orbit about any [`Body`] the body crate knows, from that
//! body's own GM. Not modelled: the body's non-spherical gravity (J2 and above), third-body
//! perturbations, drag and solar radiation pressure. For a low orbit about a planet that is a
//! kilometre-a-minute error along the track; for a moon or a high orbit it is far smaller. The
//! number is not hidden — [`PlanetOrbit::state_at`] is documented as two-body, and a caller who
//! needs better must bring a force model rather than assume this one.
//!
//! # Frames
//!
//! States are returned with ICRF *axes* and the body's centre as the origin. That is the one frame
//! definition that needs no agreement about which of a planet's frames to use, and the body crate's
//! [`Body::orientation`] rotates into the body's equator and prime meridian when a caller wants a
//! planet-fixed answer.

use stkcraft_body::Body;
use stkcraft_core::{Error, Result};
use stkcraft_time::Epoch;

/// A position and velocity in kilometres and kilometres per second.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct State {
    /// Position, km, ICRF axes, origin at the central body's centre.
    pub position_km: [f64; 3],
    /// Velocity, km/s, same axes.
    pub velocity_km_s: [f64; 3],
}

/// A two-body orbit about one body.
///
/// The classical elements at an epoch, in the units a mission analyst would quote: kilometres,
/// radians and an [`Epoch`] on the TDB scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanetOrbit {
    /// The body being orbited.
    pub central: Body,
    /// Semi-major axis, km.
    pub semi_major_axis_km: f64,
    /// Eccentricity, dimensionless and in `[0, 1)` for a closed orbit.
    pub eccentricity: f64,
    /// Inclination, radians, measured from the plane whose normal is the ICRF z axis.
    pub inclination_rad: f64,
    /// Longitude of the ascending node, radians.
    pub ascending_node_rad: f64,
    /// Argument of periapsis, radians.
    pub argument_of_periapsis_rad: f64,
    /// Mean anomaly at [`Self::epoch`], radians.
    pub mean_anomaly_rad: f64,
    /// The instant those elements are quoted for.
    pub epoch: Epoch,
}

impl PlanetOrbit {
    /// A circular orbit of a given radius and inclination, at an epoch.
    ///
    /// The convenience constructor for "a satellite 400 km above Mars": everything a circular
    /// orbit needs is the radius, the plane, and where along it the satellite starts.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfRange`] when the radius is not positive and finite, or is inside the body.
    pub fn circular(
        central: Body,
        radius_km: f64,
        inclination_rad: f64,
        epoch: Epoch,
    ) -> Result<Self> {
        let equatorial = central.radii_km()[0];
        if !radius_km.is_finite() || radius_km <= equatorial {
            return Err(Error::out_of_range(
                "orbit radius",
                format!("{radius_km} km"),
                "greater than the central body's equatorial radius",
            ));
        }
        Ok(Self {
            central,
            semi_major_axis_km: radius_km,
            eccentricity: 0.0,
            inclination_rad,
            ascending_node_rad: 0.0,
            argument_of_periapsis_rad: 0.0,
            mean_anomaly_rad: 0.0,
            epoch,
        })
    }

    /// Check the elements describe a closed orbit.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfRange`] for a semi-major axis that is not positive and finite or lies inside
    /// the body, an eccentricity outside `[0, 1)`, or an inclination outside `[0, π]`.
    pub fn validate(&self) -> Result<()> {
        if !self.semi_major_axis_km.is_finite()
            || self.semi_major_axis_km <= self.central.radii_km()[0]
        {
            return Err(Error::out_of_range(
                "semi-major axis",
                format!("{} km", self.semi_major_axis_km),
                "positive, finite, and outside the central body",
            ));
        }
        if !(0.0..1.0).contains(&self.eccentricity) {
            return Err(Error::out_of_range(
                "eccentricity",
                self.eccentricity,
                "0 <= e < 1",
            ));
        }
        if !self.inclination_rad.is_finite()
            || self.inclination_rad < 0.0
            || self.inclination_rad > core::f64::consts::PI
        {
            return Err(Error::out_of_range(
                "inclination",
                format!("{} rad", self.inclination_rad),
                "0 <= i <= pi",
            ));
        }
        for (name, value) in [
            ("ascending node", self.ascending_node_rad),
            ("argument of periapsis", self.argument_of_periapsis_rad),
            ("mean anomaly", self.mean_anomaly_rad),
        ] {
            if !value.is_finite() {
                return Err(Error::out_of_range(name, format!("{value} rad"), "finite"));
            }
        }
        Ok(())
    }

    /// The orbital period, seconds.
    #[must_use]
    pub fn period_seconds(&self) -> f64 {
        let mu = self.central.gm_km3_s2();
        if mu <= 0.0 || self.semi_major_axis_km <= 0.0 {
            return f64::NAN;
        }
        core::f64::consts::TAU * (self.semi_major_axis_km.powi(3) / mu).sqrt()
    }

    /// The state at an instant, from the two-body solution.
    ///
    /// # Errors
    ///
    /// As [`Self::validate`], plus [`Error::Invalid`] for an epoch whose scale differs from the
    /// elements' — a Keplerian propagation has no leap seconds in it, so comparing a TDB epoch with
    /// a UTC one is a question about the caller's clock, not about the orbit.
    pub fn state_at(&self, at: Epoch) -> Result<State> {
        self.validate()?;
        if at.scale() != self.epoch.scale() {
            return Err(Error::invalid(
                "orbit epoch",
                format!(
                    "the elements are on the {} scale and the instant is on the {} scale",
                    self.epoch.scale().id(),
                    at.scale().id()
                ),
            ));
        }
        let mu = self.central.gm_km3_s2();
        let a = self.semi_major_axis_km;
        let e = self.eccentricity;

        // Mean motion, then the mean anomaly at the requested instant.
        let mean_motion = (mu / (a * a * a)).sqrt();
        let elapsed = at.julian().seconds_until(self.epoch.julian());
        let mean_anomaly = self.mean_anomaly_rad + mean_motion * elapsed;

        // Kepler's equation, `M = E − e sin E`, from Newton starting at `E = M` — always convergent
        // for `e < 1` — and bounded so a hostile element set cannot spin here.
        let mut eccentric_anomaly = wrap_two_pi(mean_anomaly);
        for _ in 0..48 {
            let residual = eccentric_anomaly - e * eccentric_anomaly.sin() - mean_anomaly;
            let step = residual / (1.0 - e * eccentric_anomaly.cos());
            eccentric_anomaly -= step;
            if step.abs() < 1e-13 {
                break;
            }
        }

        // Position and velocity in the orbital plane.
        let (sin_anomaly, cos_anomaly) = eccentric_anomaly.sin_cos();
        let one_minus_e_cos = 1.0 - e * cos_anomaly;
        let semi_minor = (1.0 - e * e).max(0.0).sqrt();
        let x_plane = a * (cos_anomaly - e);
        let y_plane = a * semi_minor * sin_anomaly;
        let velocity_scale = mean_motion * a / one_minus_e_cos;
        let velocity_in_plane_x = -velocity_scale * sin_anomaly;
        let velocity_in_plane_y = velocity_scale * semi_minor * cos_anomaly;

        // Rotate the orbital plane into the ICRF axes: `Rz(Ω) · Rx(i) · Rz(ω)`.
        let rotation = perifocal_to_icrf(
            self.argument_of_periapsis_rad,
            self.inclination_rad,
            self.ascending_node_rad,
        );
        let position = apply(&rotation, [x_plane, y_plane, 0.0]);
        let velocity = apply(&rotation, [velocity_in_plane_x, velocity_in_plane_y, 0.0]);
        Ok(State {
            position_km: position,
            velocity_km_s: velocity,
        })
    }
}

/// Reduce an angle to `[0, 2π)`.
fn wrap_two_pi(angle: f64) -> f64 {
    let turn = core::f64::consts::TAU;
    let wrapped = angle % turn;
    if wrapped < 0.0 {
        wrapped + turn
    } else {
        wrapped
    }
}

/// The rotation from the orbital plane to the reference axes: `Rz(Ω) · Rx(i) · Rz(ω)`.
fn perifocal_to_icrf(
    argument_of_periapsis: f64,
    inclination: f64,
    ascending_node: f64,
) -> [[f64; 3]; 3] {
    let (sin_node, cos_node) = ascending_node.sin_cos();
    let (sin_i, cos_i) = inclination.sin_cos();
    let (sin_w, cos_w) = argument_of_periapsis.sin_cos();
    [
        [
            cos_node * cos_w - sin_node * sin_w * cos_i,
            -cos_node * sin_w - sin_node * cos_w * cos_i,
            sin_node * sin_i,
        ],
        [
            sin_node * cos_w + cos_node * sin_w * cos_i,
            -sin_node * sin_w + cos_node * cos_w * cos_i,
            -cos_node * sin_i,
        ],
        [sin_w * sin_i, cos_w * sin_i, cos_i],
    ]
}

fn apply(matrix: &[[f64; 3]; 3], vector: [f64; 3]) -> [f64; 3] {
    let mut out = [0.0_f64; 3];
    for (cell, row) in out.iter_mut().zip(matrix.iter()) {
        *cell = row[0] * vector[0] + row[1] * vector[1] + row[2] * vector[2];
    }
    out
}

#[cfg(test)]
#[expect(clippy::expect_used)]
#[allow(clippy::indexing_slicing)] // the tests index three-component vectors by axis
mod tests {
    use super::*;
    use stkcraft_time::{JD_J2000, TimeScale};

    fn tdb(seconds: f64) -> Epoch {
        Epoch::from_julian(
            stkcraft_time::JulianDate::from_parts(JD_J2000, seconds / 86_400.0),
            TimeScale::Tdb,
        )
    }

    #[test]
    fn a_circular_orbit_has_the_speed_the_vis_viva_equation_predicts() {
        // v = sqrt(mu / r), straight from the definition of a circular orbit, with Mars's own GM.
        let radius = 4_000.0;
        let orbit =
            PlanetOrbit::circular(Body::Mars, radius, 0.0, tdb(0.0)).expect("a valid orbit");
        let state = orbit.state_at(tdb(0.0)).expect("a valid epoch");
        let speed = (state.velocity_km_s[0].powi(2)
            + state.velocity_km_s[1].powi(2)
            + state.velocity_km_s[2].powi(2))
        .sqrt();
        let expected = (Body::Mars.gm_km3_s2() / radius).sqrt();
        assert!(
            (speed - expected).abs() < 1e-9,
            "speed {speed} km/s against vis-viva {expected}"
        );
        let distance = (state.position_km[0].powi(2)
            + state.position_km[1].powi(2)
            + state.position_km[2].powi(2))
        .sqrt();
        assert!((distance - radius).abs() < 1e-9);
    }

    #[test]
    fn half_a_period_puts_the_satellite_opposite_where_it_started() {
        // For a *circular* orbit half a period is exactly the antipode, whatever the plane: a
        // property no wrong rotation or sign survives. (For an ellipse it is not — the opposite
        // point of the orbit is not at `r + T/2` — so the orbit here is circular on purpose.)
        let orbit = PlanetOrbit {
            inclination_rad: 0.6,
            ascending_node_rad: 1.1,
            argument_of_periapsis_rad: 0.3,
            ..PlanetOrbit::circular(Body::Mars, 5_000.0, 0.0, tdb(0.0)).expect("a valid orbit")
        };
        let period = orbit.period_seconds();
        let start = orbit.state_at(tdb(0.0)).expect("valid");
        let half = orbit.state_at(tdb(period * 0.5)).expect("valid");
        for axis in 0..3 {
            assert!(
                (start.position_km[axis] + half.position_km[axis]).abs() < 1e-6,
                "axis {axis}: {} and {} are not opposite",
                start.position_km[axis],
                half.position_km[axis]
            );
        }
    }

    #[test]
    fn the_energy_and_angular_momentum_are_the_constants_the_elements_imply() {
        // For a two-body orbit `h = sqrt(mu a (1 - e²))` and `E = -mu / 2a`; both must hold at every
        // point of the orbit, which is what makes this propagator checkable rather than plausible.
        let orbit = PlanetOrbit {
            semi_major_axis_km: 90_000.0,
            eccentricity: 0.2,
            inclination_rad: 0.4,
            ascending_node_rad: 2.0,
            argument_of_periapsis_rad: 0.9,
            mean_anomaly_rad: 0.5,
            ..PlanetOrbit::circular(Body::Jupiter, 90_000.0, 0.0, tdb(0.0)).expect("a valid orbit")
        };
        let mu = Body::Jupiter.gm_km3_s2();
        let expected_h = (mu * orbit.semi_major_axis_km * (1.0 - 0.04)).sqrt();
        let expected_energy = -mu / (2.0 * orbit.semi_major_axis_km);
        let period = orbit.period_seconds();
        for fraction in [0.0, 0.17, 0.5, 0.83] {
            let state = orbit.state_at(tdb(period * fraction)).expect("valid");
            let r = state.position_km;
            let v = state.velocity_km_s;
            let h = [
                r[1] * v[2] - r[2] * v[1],
                r[2] * v[0] - r[0] * v[2],
                r[0] * v[1] - r[1] * v[0],
            ];
            let h_norm = (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt();
            assert!(
                (h_norm - expected_h).abs() < 1e-6 * expected_h,
                "at {fraction}: angular momentum {h_norm} against {expected_h}"
            );
            let radius = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
            let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let energy = 0.5 * speed * speed - mu / radius;
            assert!(
                (energy - expected_energy).abs() < 1e-6 * expected_energy.abs(),
                "at {fraction}: specific energy {energy} against {expected_energy}"
            );
        }
    }

    #[test]
    fn an_orbit_inside_the_body_is_refused() {
        let error = PlanetOrbit::circular(Body::Mars, 3_000.0, 0.0, tdb(0.0))
            .expect_err("Mars's radius is 3 396 km");
        assert!(matches!(error, Error::OutOfRange { .. }), "{error:?}");
    }

    #[test]
    fn a_hostile_element_set_is_refused_rather_than_propagated() {
        let base = PlanetOrbit::circular(Body::Mars, 5_000.0, 0.0, tdb(0.0)).expect("valid");
        for (label, orbit) in [
            (
                "eccentricity above one",
                PlanetOrbit {
                    eccentricity: 1.2,
                    ..base
                },
            ),
            (
                "non-finite node",
                PlanetOrbit {
                    ascending_node_rad: f64::NAN,
                    ..base
                },
            ),
            (
                "inclination past pi",
                PlanetOrbit {
                    inclination_rad: 4.0,
                    ..base
                },
            ),
        ] {
            assert!(orbit.validate().is_err(), "{label} was accepted");
        }
    }

    #[test]
    fn mixing_time_scales_is_refused() {
        // A Keplerian propagation has no leap seconds in it: comparing a TDB epoch with a TT one
        // would quietly shift the satellite by kilometres.
        let orbit = PlanetOrbit::circular(Body::Mars, 5_000.0, 0.0, tdb(0.0)).expect("valid");
        let tt = Epoch::from_jd(JD_J2000, TimeScale::Tt);
        assert!(orbit.state_at(tt).is_err());
    }
}
