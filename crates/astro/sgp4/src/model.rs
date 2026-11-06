//! SGP4 initialization and propagation.
//!
//! [`Satellite`] holds everything `sgp4init` precomputes from an element set, and
//! [`Satellite::propagate`] is the `sgp4` step. The two are split the way the reference
//! implementation splits them because initialization is the expensive part: a scenario
//! propagating a satellite across a day calls `sgp4` thousands of times and `sgp4init` once.
//!
//! The deep-space half of the theory — everything under `SDP4` in the older literature — is
//! the same code path, selected by the orbital period. Keeping it in one function rather than
//! two is what the 2006 revision does, and it matters: the two versions in the field had
//! drifted, and a single path is the only way to be sure which one ran.
//!
//! Units inside this module are canonical: Earth radii, radians, minutes. The public API
//! converts at the boundary ([`StateVector`] is in km and km/s), which keeps every
//! intermediate expression comparable, term for term, with the published algorithm.

use serde::{Deserialize, Serialize};
use stkcraft_core::{Distance, Error, Result, Velocity};

use crate::constants::{
    ATMOSPHERE_Q0_KM, ATMOSPHERE_S_FLOOR_KM, DEEP_SPACE_PERIOD_MINUTES, EARTH_ROTATION_RAD_PER_MIN,
    HALF_DAY_ECCENTRICITY_MIN, HALF_DAY_MEAN_MOTION_MAX, HALF_DAY_MEAN_MOTION_MIN,
    JD_1950_JAN_0_FRACTION, LYDDANE_INCLINATION_RAD, MINUTES_PER_DAY, PERIGEE_HEIGHT_FLOOR_KM,
    PERIGEE_HEIGHT_LIMIT_KM, RESONANCE_STEP_MINUTES, RESONANCE_STEP2, SIMPLIFIED_DRAG_RADIUS,
    SYNCHRONOUS_MEAN_MOTION_MAX, SYNCHRONOUS_MEAN_MOTION_MIN,
};
use crate::elements::Elements;
use crate::error::PropagationError;

/// Radians in a full turn.
const TWO_PI: f64 = core::f64::consts::TAU;

// The variable names in this module deliberately follow the published algorithm: `argpo` is
// the argument of perigee at epoch and `argpm` its secular mean value, `eccp` is the
// *perturbed* eccentricity and `eccm` the *mean* one, `argpp`/`nodep`/`mp` are the values
// after the periodic terms. Those distinctions are the whole content of the theory, and
// renaming them to satisfy a similarity heuristic would make a transcription error much
// easier to commit and much harder to see.

/// Whether the AFSPC interpretation of the reference code is used.
///
/// The two are not mathematically identical: the AFSPC mode keeps the node in `[0, 2π)`
/// where it is used without a trigonometric function in front of it. The published
/// verification output was produced with `opsmode = 'a'`, so this crate follows that mode.
const AFSPC_MODE: bool = true;

/// An inertial state vector.
///
/// The frame is TEME — True Equator, Mean Equinox of date — which is the frame SGP4 is
/// defined in and the only frame in which its output is meaningful. Converting to ICRF or to
/// an Earth-fixed frame needs precession, nutation and Earth rotation, which live in
/// `stkcraft-astro-frames`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StateVector {
    /// Position, km, in TEME.
    pub position: [Distance; 3],
    /// Velocity, km/s, in TEME.
    pub velocity: [Velocity; 3],
}

impl StateVector {
    /// The position magnitude, km.
    #[must_use]
    pub fn radius(&self) -> Distance {
        let [x, y, z] = self.position;
        let sum = x.as_km().powi(2) + y.as_km().powi(2) + z.as_km().powi(2);
        Distance::from_km(sum.sqrt())
    }

    /// The velocity magnitude, km/s.
    #[must_use]
    pub fn speed(&self) -> Velocity {
        let [x, y, z] = self.velocity;
        let sum = x.as_km_s().powi(2) + y.as_km_s().powi(2) + z.as_km_s().powi(2);
        Velocity::from_km_s(sum.sqrt())
    }
}

/// A deep-space resonance condition.
///
/// The reference implementation carries this as the integer `irez`; naming it makes the two
/// branches in the propagator readable and makes an accidental `== 2` on the wrong variable
/// impossible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Resonance {
    /// No resonance; the near-Earth terms are the whole model.
    None,
    /// One-day (geosynchronous) resonance.
    Synchronous,
    /// Half-day (Molniya-like, 12-hour) resonance.
    HalfDay,
}

/// The resonance integrator's carried state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
struct ResonanceState {
    /// Minutes from the epoch of the last integrator step.
    atime: f64,
    /// The resonant argument at that step.
    xli: f64,
    /// The mean motion at that step.
    xni: f64,
}

/// The deep-space secular rates and periodic coefficients computed by `dscom` and `dsinit`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct DeepSpace {
    snodm: f64,
    cnodm: f64,
    sinim: f64,
    cosim: f64,
    sinomm: f64,
    cosomm: f64,
    day: f64,
    e3: f64,
    ee2: f64,
    em: f64,
    emsq: f64,
    gam: f64,
    peo: f64,
    pgho: f64,
    pho: f64,
    pinco: f64,
    plo: f64,
    rtemsq: f64,
    se2: f64,
    se3: f64,
    sgh2: f64,
    sgh3: f64,
    sgh4: f64,
    sh2: f64,
    sh3: f64,
    si2: f64,
    si3: f64,
    sl2: f64,
    sl3: f64,
    sl4: f64,
    s1: f64,
    s2: f64,
    s3: f64,
    s4: f64,
    s5: f64,
    s6: f64,
    s7: f64,
    ss1: f64,
    ss2: f64,
    ss3: f64,
    ss4: f64,
    ss5: f64,
    ss6: f64,
    ss7: f64,
    sz1: f64,
    sz2: f64,
    sz3: f64,
    sz11: f64,
    sz12: f64,
    sz13: f64,
    sz21: f64,
    sz22: f64,
    sz23: f64,
    sz31: f64,
    sz32: f64,
    sz33: f64,
    /// The `z11` term as left by the final (lunar) iteration of `dscom`.
    z11_final: f64,
    /// The `z13` term as left by the final (lunar) iteration of `dscom`.
    z13_final: f64,
    /// The `z1` term as left by the final (lunar) iteration of `dscom`.
    z1_final: f64,
    /// The `z3` term as left by the final (lunar) iteration of `dscom`.
    z3_final: f64,
    /// The `z21` term as left by the final (lunar) iteration of `dscom`.
    z21_final: f64,
    /// The `z23` term as left by the final (lunar) iteration of `dscom`.
    z23_final: f64,
    /// The `z31` term as left by the final (lunar) iteration of `dscom`.
    z31_final: f64,
    /// The `z33` term as left by the final (lunar) iteration of `dscom`.
    z33_final: f64,
    xgh2: f64,
    xgh3: f64,
    xgh4: f64,
    xh2: f64,
    xh3: f64,
    xi2: f64,
    xi3: f64,
    xl2: f64,
    xl3: f64,
    xl4: f64,
    zmol: f64,
    zmos: f64,
    // From dsinit.
    resonance: Resonance,
    dedt: f64,
    didt: f64,
    dmdt: f64,
    dnodt: f64,
    domdt: f64,
    del1: f64,
    del2: f64,
    del3: f64,
    xfact: f64,
    xlamo: f64,
    d2201: f64,
    d2211: f64,
    d3210: f64,
    d3222: f64,
    d4410: f64,
    d4422: f64,
    d5220: f64,
    d5232: f64,
    d5421: f64,
    d5433: f64,
}

/// A satellite ready to propagate.
///
/// Construction runs the whole of `sgp4init`; `sgp4init` itself calls the propagator once at
/// `t = 0` to detect a sub-orbital or already-decayed element set, and this type reports that
/// as an error rather than storing a satellite that cannot be propagated.
#[derive(Debug, Clone)]
pub struct Satellite {
    elements: Elements,
    // --- canonical elements -----------------------------------------------------------------
    ecco: f64,
    inclo: f64,
    nodeo: f64,
    argpo: f64,
    mo: f64,
    no_unkozai: f64,
    bstar: f64,
    // --- epoch ------------------------------------------------------------------------------
    /// Greenwich sidereal time at epoch, radians.
    gsto: f64,
    // --- precomputed shape terms ------------------------------------------------------------
    con41: f64,
    isimp: bool,
    // --- atmospheric and drag terms --------------------------------------------------------
    eta: f64,
    cc1: f64,
    cc4: f64,
    cc5: f64,
    d2: f64,
    d3: f64,
    d4: f64,
    delmo: f64,
    sinmao: f64,
    t2cof: f64,
    t3cof: f64,
    t4cof: f64,
    t5cof: f64,
    x1mth2: f64,
    x7thm1: f64,
    mdot: f64,
    argpdot: f64,
    nodedot: f64,
    omgcof: f64,
    xmcof: f64,
    xnodcf: f64,
    xlcof: f64,
    aycof: f64,
    // --- deep space -----------------------------------------------------------------------
    deep: Option<DeepSpace>,
    /// The resonance integrator's state, carried across calls exactly as the reference
    /// implementation carries `atime`, `xli` and `xni` in its structure.
    ///
    /// These are not a cache: the integrator steps in 720-minute increments from the epoch,
    /// and the history is what keeps a long run consistent. Recomputing them from scratch on
    /// every call gives a different answer, so they are stored.
    resonance_state: ResonanceState,
}

impl Satellite {
    /// Initialize a satellite from an element set.
    ///
    /// # Errors
    ///
    /// - [`PropagationError::SubOrbitalElements`] if the perigee is inside the Earth at
    ///   epoch, which `sgp4init` reports as code 5.
    /// - [`PropagationError::Decayed`] if the state at epoch is already below one Earth
    ///   radius, which `sgp4init` reports as code 6 by calling the propagator once.
    pub fn new(elements: Elements) -> Result<Self> {
        let g = elements.gravity.constants();
        let radius_earth_km = g.radius_earth_km;

        let ecco = elements.eccentricity;
        let inclo = elements.inclination.as_rad();
        let nodeo = elements.right_ascension.as_rad();
        let argpo = elements.argument_of_perigee.as_rad();
        let mo = elements.mean_anomaly.as_rad();
        let no_kozai = elements.mean_motion;
        let bstar = elements.bstar;
        // `dscom` is defined in days since 1950 January 0.0, so it needs the whole day count
        // *plus* the fraction. Taking only the fraction is the classic mistake: the lunar and
        // solar arguments `day` then lose their decade-scale ramp and the deep-space secular
        // rates drift by kilometres.
        //
        // `whole_days` already counts civil midnights from the 1950 January 0.0 zero, i.e.
        // from JD 2433281.5, so this sum is exactly the reference implementation's `epoch`.
        // Adding half a day here to "reach" the Julian day boundary would double-count it.
        let (fraction_of_day, whole_days) = elements.sgp4_epoch();
        let epoch_days = whole_days + fraction_of_day;

        // ---- initl: auxiliary epoch quantities -------------------------------------------
        let eccsq = ecco * ecco;
        let omeosq = 1.0 - eccsq;
        let rteosq = omeosq.sqrt();
        let cosio = inclo.cos();
        let cosio_sq = cosio * cosio;

        // Un-Kozai the mean motion. Source: AIAA 2006-6753 `initl`.
        let ak = (g.xke / no_kozai).powf(2.0 / 3.0);
        let d1 = 0.75 * g.j2 * (3.0 * cosio_sq - 1.0) / (rteosq * omeosq);
        let del = d1 / (ak * ak);
        let adel = ak * (1.0 - del * del - del * (1.0 / 3.0 + 134.0 * del * del / 81.0));
        let del = d1 / (adel * adel);
        let no_unkozai = no_kozai / (1.0 + del);

        let ao = (g.xke / no_unkozai).powf(2.0 / 3.0);
        let sinio = inclo.sin();
        let po = ao * omeosq;
        let con42 = 1.0 - 5.0 * cosio_sq;
        let con41 = -con42 - cosio_sq - cosio_sq;
        let posq = po * po;
        let rp = ao * (1.0 - ecco);

        // Greenwich sidereal time at epoch. Source: AIAA 2006-6753 `initl`, the "modern
        // approach" branch, which is what `opsmode = 'i'` selects.
        let gsto = sidereal_time_at_epoch(whole_days, fraction_of_day);

        // A perigee inside the Earth means there is no orbit to propagate.
        //
        // The reference implementation computes the epoch state first and lets the
        // radius check raise the decay code, which is why its published output for the
        // high-eccentricity `33333` case reports code 4 rather than 5. Refusing here is the
        // stronger guarantee — no state vector is invented for an orbit that does not
        // exist — so this crate reports 5 and says why.
        if rp < 1.0 {
            return Err(PropagationError::SubOrbitalElements.into());
        }
        if omeosq < 0.0 || no_unkozai < 0.0 {
            // The reference implementation skips the coefficient block in this case and
            // relies on the epoch propagation to raise the error. Returning here is the
            // same outcome without an uninitialized structure.
            return Err(PropagationError::MeanElementsOutOfFamily.into());
        }

        let isimp = rp < SIMPLIFIED_DRAG_RADIUS;

        // ---- atmospheric parameters ------------------------------------------------------
        let radius_inverse = 1.0 / radius_earth_km;
        let mut sfour = 78.0 * radius_inverse + 1.0;
        let mut qzms2t = ((ATMOSPHERE_Q0_KM - 78.0) * radius_inverse).powi(4);
        let perige = (rp - 1.0) * radius_earth_km;
        if perige < PERIGEE_HEIGHT_LIMIT_KM {
            let mut sfour_km = perige - 78.0;
            if perige <= PERIGEE_HEIGHT_FLOOR_KM {
                sfour_km = ATMOSPHERE_S_FLOOR_KM;
            }
            qzms2t = ((ATMOSPHERE_Q0_KM - sfour_km) * radius_inverse).powi(4);
            sfour = sfour_km * radius_inverse + 1.0;
        }

        let pinvsq = 1.0 / posq;
        let tsi = 1.0 / (ao - sfour);
        let eta = ao * ecco * tsi;
        let etasq = eta * eta;
        let ecc_times_eta = ecco * eta;
        let psisq = (1.0 - etasq).abs();
        let coef = qzms2t * tsi.powi(4);
        let coef1 = coef / psisq.powf(3.5);
        let cc2 = coef1
            * no_unkozai
            * (ao * (1.0 + 1.5 * etasq + ecc_times_eta * (4.0 + etasq))
                + 0.375 * g.j2 * tsi / psisq * con41 * (8.0 + 3.0 * etasq * (8.0 + etasq)));
        let cc1 = bstar * cc2;
        let mut cc3 = 0.0;
        if ecco > 1.0e-4 {
            cc3 = -2.0 * coef * tsi * g.j3_over_j2 * no_unkozai * sinio / ecco;
        }
        let x1mth2 = 1.0 - cosio_sq;
        let cc4 = 2.0
            * no_unkozai
            * coef1
            * ao
            * omeosq
            * (eta * (2.0 + 0.5 * etasq) + ecco * (0.5 + 2.0 * etasq)
                - g.j2 * tsi / (ao * psisq)
                    * (-3.0
                        * con41
                        * (1.0 - 2.0 * ecc_times_eta + etasq * (1.5 - 0.5 * ecc_times_eta))
                        + 0.75
                            * x1mth2
                            * (2.0 * etasq - ecc_times_eta * (1.0 + etasq))
                            * (2.0 * argpo).cos()));
        let cc5 = 2.0
            * coef1
            * ao
            * omeosq
            * (1.0 + 2.75 * (etasq + ecc_times_eta) + ecc_times_eta * etasq);
        let cosio4 = cosio_sq * cosio_sq;
        let temp1 = 1.5 * g.j2 * pinvsq * no_unkozai;
        let temp2 = 0.5 * temp1 * g.j2 * pinvsq;
        let temp3 = -0.46875 * g.j4 * pinvsq * pinvsq * no_unkozai;
        let mdot = no_unkozai
            + 0.5 * temp1 * rteosq * con41
            + 0.0625 * temp2 * rteosq * (13.0 - 78.0 * cosio_sq + 137.0 * cosio4);
        let argpdot = -0.5 * temp1 * con42
            + 0.0625 * temp2 * (7.0 - 114.0 * cosio_sq + 395.0 * cosio4)
            + temp3 * (3.0 - 36.0 * cosio_sq + 49.0 * cosio4);
        let xnode_dot_base = -temp1 * cosio;
        let nodedot = xnode_dot_base
            + (0.5 * temp2 * (4.0 - 19.0 * cosio_sq) + 2.0 * temp3 * (3.0 - 7.0 * cosio_sq))
                * cosio;
        let xpidot = argpdot + nodedot;
        let omgcof = bstar * cc3 * argpo.cos();
        let mut xmcof = 0.0;
        if ecco > 1.0e-4 {
            xmcof = -(2.0 / 3.0) * coef * bstar / ecc_times_eta;
        }
        let xnodcf = 3.5 * omeosq * xnode_dot_base * cc1;
        let t2cof = 1.5 * cc1;
        let aycof = -0.5 * g.j3_over_j2 * sinio;
        let temp4 = 1.5e-12;
        let xlcof = if (cosio + 1.0).abs() > temp4 {
            -0.25 * g.j3_over_j2 * sinio * (3.0 + 5.0 * cosio) / (1.0 + cosio)
        } else {
            -0.25 * g.j3_over_j2 * sinio * (3.0 + 5.0 * cosio) / temp4
        };
        let delmo = (1.0 + eta * mo.cos()).powi(3);
        let sinmao = mo.sin();
        let x7thm1 = 7.0 * cosio_sq - 1.0;

        let mut satellite = Self {
            elements,
            ecco,
            inclo,
            nodeo,
            argpo,
            mo,
            no_unkozai,
            bstar,
            gsto,
            con41,
            isimp,
            eta,
            cc1,
            cc4,
            cc5,
            d2: 0.0,
            d3: 0.0,
            d4: 0.0,
            delmo,
            sinmao,
            t2cof,
            t3cof: 0.0,
            t4cof: 0.0,
            t5cof: 0.0,
            x1mth2,
            x7thm1,
            mdot,
            argpdot,
            nodedot,
            omgcof,
            xmcof,
            xnodcf,
            xlcof,
            aycof,
            deep: None,
            resonance_state: ResonanceState::default(),
        };

        // ---- deep space initialization ---------------------------------------------------
        if TWO_PI / no_unkozai >= DEEP_SPACE_PERIOD_MINUTES {
            let dscom = dscom(epoch_days, ecco, argpo, 0.0, inclo, nodeo, no_unkozai);
            // `inclm` here is the *unperturbed* epoch inclination, which is what the reference
            // `sgp4init` assigns to it immediately before calling `dsinit`. It matters: the
            // 180-degree guard in `dsinit` compares against it and drops the node terms, so
            // recomputing it as `asin(sin(i))` would fold every orbit above 90 degrees into
            // the wrong half.
            let dsinit = dsinit(
                &dscom,
                0.0,
                cosio,
                sinio,
                inclo,
                eccsq,
                ecco,
                argpo,
                mo,
                no_unkozai,
                nodeo,
                gsto,
                mdot,
                nodedot,
                xpidot,
                elements.gravity,
            );
            satellite.isimp = true;
            satellite.deep = Some(DeepSpace {
                resonance: dsinit.resonance,
                dedt: dsinit.dedt,
                didt: dsinit.didt,
                dmdt: dsinit.dmdt,
                dnodt: dsinit.dnodt,
                domdt: dsinit.domdt,
                del1: dsinit.del1,
                del2: dsinit.del2,
                del3: dsinit.del3,
                xfact: dsinit.xfact,
                xlamo: dsinit.xlamo,
                d2201: dsinit.d2201,
                d2211: dsinit.d2211,
                d3210: dsinit.d3210,
                d3222: dsinit.d3222,
                d4410: dsinit.d4410,
                d4422: dsinit.d4422,
                d5220: dsinit.d5220,
                d5232: dsinit.d5232,
                d5421: dsinit.d5421,
                d5433: dsinit.d5433,
                ..dscom
            });
        }

        // ---- the drag series, only when the simplified path is not in use ---------------
        if !satellite.isimp {
            let cc1sq = cc1 * cc1;
            let d2 = 4.0 * ao * tsi * cc1sq;
            let temp = d2 * tsi * cc1 / 3.0;
            let d3 = (17.0 * ao + sfour) * temp;
            let d4 = 0.5 * temp * ao * tsi * (221.0 * ao + 31.0 * sfour) * cc1;
            satellite.d2 = d2;
            satellite.d3 = d3;
            satellite.d4 = d4;
            satellite.t3cof = d2 + 2.0 * cc1sq;
            satellite.t4cof = 0.25 * (3.0 * d3 + cc1 * (12.0 * d2 + 10.0 * cc1sq));
            satellite.t5cof = 0.2
                * (3.0 * d4 + 12.0 * cc1 * d3 + 6.0 * d2 * d2 + 15.0 * cc1sq * (2.0 * d2 + cc1sq));
        }

        // `sgp4init` ends by propagating at t = 0 to surface a bad element set.
        satellite.propagate_minutes(0.0)?;
        Ok(satellite)
    }

    /// The element set this satellite was built from.
    #[must_use]
    pub fn elements(&self) -> Elements {
        self.elements
    }

    /// True when the deep-space (SDP4) branch is in use.
    #[must_use]
    pub fn is_deep_space(&self) -> bool {
        self.deep.is_some()
    }

    /// Propagate to a time offset from the element set's epoch.
    ///
    /// # Errors
    ///
    /// One of [`PropagationError`]: a decayed satellite, a mean motion that has gone
    /// non-positive, or a perturbation that has taken the elements out of family. All three
    /// are real outcomes for a stale element set and none of them has a meaningful state
    /// vector, so none of them returns one.
    pub fn propagate(&mut self, since_epoch: stkcraft_core::Duration) -> Result<StateVector> {
        self.propagate_minutes(since_epoch.as_seconds() / 60.0)
    }

    /// Propagate to a number of minutes after the epoch.
    ///
    /// This is the reference implementation's interface and the one the verification file is
    /// tabulated in, so it is public: comparing against published output needs it.
    ///
    /// # Errors
    ///
    /// As [`Self::propagate`].
    pub fn propagate_minutes(&mut self, tsince: f64) -> Result<StateVector> {
        if !tsince.is_finite() {
            return Err(Error::invalid(
                "propagation time",
                "must be a finite number of minutes",
            ));
        }
        let g = self.elements.gravity.constants();

        // ---- secular gravity and drag ---------------------------------------------------
        let xmdf = self.mo + self.mdot * tsince;
        let argpdf = self.argpo + self.argpdot * tsince;
        let nodedf = self.nodeo + self.nodedot * tsince;
        let mut argpm = argpdf;
        let mut mm = xmdf;
        let t2 = tsince * tsince;
        let mut nodem = nodedf + self.xnodcf * t2;
        let mut tempa = 1.0 - self.cc1 * tsince;
        let mut tempe = self.bstar * self.cc4 * tsince;
        let mut templ = self.t2cof * t2;

        if !self.isimp {
            let delomg = self.omgcof * tsince;
            let delmtemp = 1.0 + self.eta * xmdf.cos();
            let delm = self.xmcof * (delmtemp.powi(3) - self.delmo);
            let temp = delomg + delm;
            mm = xmdf + temp;
            argpm = argpdf - temp;
            let t3 = t2 * tsince;
            let t4 = t3 * tsince;
            tempa = tempa - self.d2 * t2 - self.d3 * t3 - self.d4 * t4;
            tempe += self.bstar * self.cc5 * (mm.sin() - self.sinmao);
            templ += self.t3cof * t3 + t4 * (self.t4cof + tsince * self.t5cof);
        }

        let mut nm = self.no_unkozai;
        let mut em = self.ecco;
        let mut inclm = self.inclo;

        if let Some(deep) = &self.deep {
            // `dspace` receives the un-Kozai'd mean motion the element set was initialized
            // with, which is what `sgp4init` passes as `no`.
            let mut resonance_state = self.resonance_state;
            let result = dspace(
                deep,
                &mut resonance_state,
                self.argpo,
                self.argpdot,
                tsince,
                self.gsto,
                self.no_unkozai,
                em,
                argpm,
                inclm,
                mm,
                nodem,
                nm,
            );
            self.resonance_state = resonance_state;
            em = result.eccm;
            argpm = result.argpm;
            inclm = result.inclm;
            mm = result.mm;
            nodem = result.nodem;
            nm = result.xn;
        }

        if nm <= 0.0 {
            return Err(PropagationError::NonPositiveMeanMotion.into());
        }

        let am = (g.xke / nm).powf(2.0 / 3.0) * tempa * tempa;
        let nm = g.xke / am.powf(1.5);
        em -= tempe;
        if !(-0.001..1.0).contains(&em) || am < 0.95 {
            return Err(PropagationError::MeanElementsOutOfFamily.into());
        }
        // `sgp4fix change test condition for eccentricity`: the mean eccentricity is floored
        // at 1e-6 so the short-period terms never divide by zero. Source: the C++ reference,
        // which applies the floor before the deep-space periodics are added.
        let em = if em < 1.0e-6 { 1.0e-6 } else { em };
        mm += self.no_unkozai * templ;
        let xlm = mm + argpm + nodem;
        let emsq = em * em;
        debug_assert!(emsq.is_finite());
        // The C++ implementation keeps `temp = 1.0 - emsq` here for later use; this crate
        // computes the short-period terms directly, so the value is not needed.
        nodem %= TWO_PI;
        argpm %= TWO_PI;
        let xlm = xlm % TWO_PI;
        mm = (xlm - argpm - nodem) % TWO_PI;

        let sinim = inclm.sin();
        let cosim = inclm.cos();

        // ---- lunar and solar periodics --------------------------------------------------
        let mut eccp = em;
        let mut xincp = inclm;
        let mut argpp = argpm;
        let mut nodep = nodem;
        let mut mp = mm;
        let mut sinip = sinim;
        let mut cosip = cosim;

        if let Some(deep) = &self.deep {
            let periodic = dpper(deep, tsince, self.inclo, eccp, xincp, nodep, argpp, mp);
            eccp = periodic.eccp;
            xincp = periodic.inclp;
            nodep = periodic.nodep;
            argpp = periodic.argpp;
            mp = periodic.mp;

            // The eccentricity check precedes the negative-inclination flip, as it does in
            // the reference; `33335` reports its error only in that order.
            if !(0.0..=1.0).contains(&eccp) {
                return Err(PropagationError::PerturbedEccentricityOutOfFamily.into());
            }
            if xincp < 0.0 {
                xincp = -xincp;
                nodep += core::f64::consts::PI;
                argpp -= core::f64::consts::PI;
            }

            sinip = xincp.sin();
            cosip = xincp.cos();
        }

        // Floor the mean eccentricity only after the deep-space periodics have been applied,
        // exactly as the reference does, so a perturbed eccentricity outside `[0, 1]` is
        // reported rather than silently rounded into range.
        eccp = if eccp < 1.0e-6 { 1.0e-6 } else { eccp };

        let temp4 = 1.5e-12;
        let (aycof, xlcof) = if self.deep.is_some() {
            let ay = -0.5 * g.j3_over_j2 * sinip;
            let xl = if (cosip + 1.0).abs() > temp4 {
                -0.25 * g.j3_over_j2 * sinip * (3.0 + 5.0 * cosip) / (1.0 + cosip)
            } else {
                -0.25 * g.j3_over_j2 * sinip * (3.0 + 5.0 * cosip) / temp4
            };
            (ay, xl)
        } else {
            (self.aycof, self.xlcof)
        };

        // ---- long-period periodics ------------------------------------------------------
        let axnl = eccp * argpp.cos();
        let temp = 1.0 / (am * (1.0 - eccp * eccp));
        let aynl = eccp * argpp.sin() + temp * aycof;
        let xl = mp + argpp + nodep + temp * xlcof * axnl;

        // ---- Kepler's equation ----------------------------------------------------------
        let u = (xl - nodep) % TWO_PI;
        let mut eo1 = u;
        let mut tem5: f64 = 9999.9;
        let mut ktr = 0;
        while tem5.abs() >= 1.0e-12 && ktr < 10 {
            ktr += 1;
            let sineo1 = eo1.sin();
            let coseo1 = eo1.cos();
            tem5 = 1.0 - coseo1 * axnl - sineo1 * aynl;
            tem5 = (u - aynl * coseo1 + axnl * sineo1 - eo1) / tem5;
            // `if (fabs(tem5) >= 0.95) tem5 = tem5 > 0.0 ? 0.95 : -0.95;` — note `>=` and a
            // three-way choice, so a correction of exactly zero stays zero.
            if tem5.abs() >= 0.95 {
                tem5 = if tem5 > 0.0 { 0.95 } else { -0.95 };
            }
            eo1 += tem5;
        }

        // ---- short-period periodics -----------------------------------------------------
        let ecose = axnl * eo1.cos() + aynl * eo1.sin();
        let esine = axnl * eo1.sin() - aynl * eo1.cos();
        let el2 = axnl * axnl + aynl * aynl;
        let pl = am * (1.0 - el2);
        if pl < 0.0 {
            return Err(PropagationError::NonPositiveSemiLatusRectum.into());
        }
        let rl = am * (1.0 - ecose);
        let rdotl = am.sqrt() * esine / rl;
        let rvdotl = pl.sqrt() / rl;
        let betal = (1.0 - el2).sqrt();
        let temp = esine / (1.0 + betal);
        let sinu = am / rl * (eo1.sin() - aynl - axnl * temp);
        let cosu = am / rl * (eo1.cos() - axnl + aynl * temp);
        let mut su = sinu.atan2(cosu);
        let sin2u = 2.0 * cosu * sinu;
        let cos2u = 1.0 - 2.0 * sinu * sinu;
        let temp = 1.0 / pl;
        let temp1 = 0.5 * g.j2 * temp;
        let temp2 = temp1 * temp;

        let (con41, x1mth2, x7thm1) = if self.deep.is_some() {
            let cosisq = cosip * cosip;
            (3.0 * cosisq - 1.0, 1.0 - cosisq, 7.0 * cosisq - 1.0)
        } else {
            (self.con41, self.x1mth2, self.x7thm1)
        };

        let mr = rl * (1.0 - 1.5 * temp2 * betal * con41) + 0.5 * temp1 * x1mth2 * cos2u;
        su -= 0.25 * temp2 * x7thm1 * sin2u;
        let xnode = nodep + 1.5 * temp2 * cosip * sin2u;
        let xinc = xincp + 1.5 * temp2 * cosip * sinip * cos2u;
        let mvt = rdotl - nm * temp1 * x1mth2 * sin2u / g.xke;
        let rvdot = rvdotl + nm * temp1 * (x1mth2 * cos2u + 1.5 * con41) / g.xke;

        // ---- orientation vectors --------------------------------------------------------
        let sinsu = su.sin();
        let cossu = su.cos();
        let snod = xnode.sin();
        let cnod = xnode.cos();
        let sini = xinc.sin();
        let cosi = xinc.cos();
        let xmx = -snod * cosi;
        let xmy = cnod * cosi;
        let ux = xmx * sinsu + cnod * cossu;
        let uy = xmy * sinsu + snod * cossu;
        let uz = sini * sinsu;
        let vx = xmx * cossu - cnod * sinsu;
        let vy = xmy * cossu - snod * sinsu;
        let vz = sini * cossu;

        // ---- position and velocity, back in kilometres ----------------------------------
        let r = [
            mr * ux * g.radius_earth_km,
            mr * uy * g.radius_earth_km,
            mr * uz * g.radius_earth_km,
        ];
        let v = [
            (mvt * ux + rvdot * vx) * g.vkmpersec,
            (mvt * uy + rvdot * vy) * g.vkmpersec,
            (mvt * uz + rvdot * vz) * g.vkmpersec,
        ];

        if mr < 1.0 {
            return Err(PropagationError::Decayed.into());
        }

        Ok(StateVector {
            position: [
                Distance::from_km(r[0]),
                Distance::from_km(r[1]),
                Distance::from_km(r[2]),
            ],
            velocity: [
                Velocity::from_km_s(v[0]),
                Velocity::from_km_s(v[1]),
                Velocity::from_km_s(v[2]),
            ],
        })
    }
}

/// Greenwich sidereal time at epoch, radians, wrapped to `[0, 2π)`.
///
/// Source: AIAA 2006-6753 `initl`, the "modern approach to finding sidereal time" branch
/// (selected by `opsmode = 'i'`). `epoch` is days from 1950 January 0.0.
fn sidereal_time_at_epoch(epoch_days: f64, fraction_of_day: f64) -> f64 {
    let _ = fraction_of_day;
    // 2433281.5 is the Julian Date of the SGP4 epoch zero, so `temp` is the Julian Date.
    let temp = epoch_days + crate::constants::JD_1950_JAN_0_DAY + JD_1950_JAN_0_FRACTION;
    // The whole-day part: the .5 boundary of the Julian Date is half a day after the SGP4
    // epoch zero, so the civil midnight of the element set's day is `temp - 0.5` truncated.
    let whole = (temp - 0.5).trunc();
    let tut1 = (whole + 0.5 - 2_451_545.0) / 36_525.0;
    // The fraction of the day is taken from the element set rather than from
    // `temp - 0.5 - whole`, which would already have lost the last digits of a Julian Date
    // near 2.45e6.
    let _ = temp;
    let rad_per_day = TWO_PI * 1.002_737_909_350_795;
    let gsto =
        1.753_368_559_233_27 + 628.331_970_688_841 * tut1 + 6.770_713_944_903_34e-6 * tut1 * tut1
            - 4.508_767_234_318_68e-10 * tut1 * tut1 * tut1
            + rad_per_day * fraction_of_day;
    let gsto = gsto % TWO_PI;
    if gsto < 0.0 { gsto + TWO_PI } else { gsto }
}

/// The `dscom` deep-space common terms.
///
/// Source: AIAA 2006-6753 `dscom`. The lunar and solar directions are low-precision series
/// from the 1980 report; the constants are transcribed exactly because the deep-space
/// answer depends on their last digits.
#[allow(clippy::too_many_arguments)]
fn dscom(epoch: f64, eccp: f64, argpp: f64, tc: f64, inclp: f64, nodep: f64, np: f64) -> DeepSpace {
    // Source constants, `dscom`.
    const ZES: f64 = 0.016_75;
    const ZEL: f64 = 0.054_90;
    const C1SS: f64 = 2.986_479_7e-6;
    const C1L: f64 = 4.796_806_5e-7;
    const ZSINIS: f64 = 0.397_854_16;
    const ZCOSIS: f64 = 0.917_448_67;
    const ZCOSGS: f64 = 0.194_590_5;
    const ZSINGS: f64 = -0.980_884_58;

    let em = eccp;
    let snodm = nodep.sin();
    let cnodm = nodep.cos();
    let sinomm = argpp.sin();
    let cosomm = argpp.cos();
    let sinim = inclp.sin();
    let cosim = inclp.cos();
    let emsq = em * em;
    let betasq = 1.0 - emsq;
    let rtemsq = betasq.sqrt();

    // Days since the deep-space lunar/solar epoch, 1900 January 0.5.
    let day = epoch + 18_261.5 + tc / MINUTES_PER_DAY;
    let xnodce = (4.523_602_0 - 9.242_202_9e-4 * day) % TWO_PI;
    let stem = xnodce.sin();
    let ctem = xnodce.cos();
    let zcosil = 0.913_751_64 - 0.035_680_96 * ctem;
    let zsinil = (1.0 - zcosil * zcosil).sqrt();
    let zsinhl = 0.089_683_511 * stem / zsinil;
    let zcoshl = (1.0 - zsinhl * zsinhl).sqrt();
    let gam = 5.835_151_4 + 0.001_944_368_0 * day;
    let zx = 0.397_854_16 * stem / zsinil;
    let zy = zcoshl * ctem + 0.917_448_67 * zsinhl * stem;
    let zx = zx.atan2(zy);
    let zx = gam + zx - xnodce;
    let zcosgl = zx.cos();
    let zsingl = zx.sin();

    let mut zcosg = ZCOSGS;
    let mut zsing = ZSINGS;
    let mut zcosi = ZCOSIS;
    let mut zsini = ZSINIS;
    let mut zcosh = cnodm;
    let mut zsinh = snodm;
    let mut cc = C1SS;
    let xnoi = 1.0 / np;

    let mut ss = [0.0_f64; 8]; // 1..=7
    // The `sz` array holds the *solar* `z` terms. They must be captured at the end of the
    // solar iteration and then left alone: the lunar iteration overwrites the scalar `z`
    // variables, and the solar coefficients `si2`, `si3`, `sh2`, `sh3`, `sl2`, `sl3` and
    // `sgh2`, `sgh3` are formed from the solar values afterwards. Reducing against the lunar
    // values instead is a small-looking mistake that moves an argument of perigee rate by
    // about 6% and a position by kilometres after a few days.
    let mut sz = [0.0_f64; 34];
    let (mut z1, mut z2, mut z3) = (0.0, 0.0, 0.0);
    let (mut z11, mut z12, mut z13) = (0.0, 0.0, 0.0);
    let (mut z21, mut z22, mut z23) = (0.0, 0.0, 0.0);
    let (mut z31, mut z32, mut z33) = (0.0, 0.0, 0.0);
    let mut s = [0.0_f64; 8];

    for lsflg in 1..=2 {
        let a1 = zcosg * zcosh + zsing * zcosi * zsinh;
        let a3 = -zsing * zcosh + zcosg * zcosi * zsinh;
        let a7 = -zcosg * zsinh + zsing * zcosi * zcosh;
        let a8 = zsing * zsini;
        let a9 = zsing * zsinh + zcosg * zcosi * zcosh;
        let a10 = zcosg * zsini;
        let a2 = cosim * a7 + sinim * a8;
        let a4 = cosim * a9 + sinim * a10;
        let a5 = -sinim * a7 + cosim * a8;
        let a6 = -sinim * a9 + cosim * a10;

        let x1 = a1 * cosomm + a2 * sinomm;
        let x2 = a3 * cosomm + a4 * sinomm;
        let x3 = -a1 * sinomm + a2 * cosomm;
        let x4 = -a3 * sinomm + a4 * cosomm;
        let x5 = a5 * sinomm;
        let x6 = a6 * sinomm;
        let x7 = a5 * cosomm;
        let x8 = a6 * cosomm;

        z31 = 12.0 * x1 * x1 - 3.0 * x3 * x3;
        z32 = 24.0 * x1 * x2 - 6.0 * x3 * x4;
        z33 = 12.0 * x2 * x2 - 3.0 * x4 * x4;
        z1 = 3.0 * (a1 * a1 + a2 * a2) + z31 * emsq;
        z2 = 6.0 * (a1 * a3 + a2 * a4) + z32 * emsq;
        z3 = 3.0 * (a3 * a3 + a4 * a4) + z33 * emsq;
        z11 = -6.0 * a1 * a5 + emsq * (-24.0 * x1 * x7 - 6.0 * x3 * x5);
        z12 = -6.0 * (a1 * a6 + a3 * a5)
            + emsq * (-24.0 * (x2 * x7 + x1 * x8) - 6.0 * (x3 * x6 + x4 * x5));
        z13 = -6.0 * a3 * a6 + emsq * (-24.0 * x2 * x8 - 6.0 * x4 * x6);
        z21 = 6.0 * a2 * a5 + emsq * (24.0 * x1 * x5 - 6.0 * x3 * x7);
        z22 = 6.0 * (a4 * a5 + a2 * a6)
            + emsq * (24.0 * (x2 * x5 + x1 * x6) - 6.0 * (x4 * x7 + x3 * x8));
        z23 = 6.0 * a4 * a6 + emsq * (24.0 * x2 * x6 - 6.0 * x4 * x8);
        z1 = z1 + z1 + betasq * z31;
        z2 = z2 + z2 + betasq * z32;
        z3 = z3 + z3 + betasq * z33;
        s[3] = cc * xnoi;
        s[2] = -0.5 * s[3] / rtemsq;
        s[4] = s[3] * rtemsq;
        s[1] = -15.0 * em * s[4];
        s[5] = x1 * x3 + x2 * x4;
        s[6] = x2 * x3 + x1 * x4;
        s[7] = x2 * x4 - x1 * x3;

        if lsflg == 1 {
            ss.clone_from(&s);
            // Copy the solar `z` set into `sz`; the lunar iteration overwrites the scalars.
            sz[1] = z1;
            sz[2] = z2;
            sz[3] = z3;
            sz[11] = z11;
            sz[12] = z12;
            sz[13] = z13;
            sz[21] = z21;
            sz[22] = z22;
            sz[23] = z23;
            sz[31] = z31;
            sz[32] = z32;
            sz[33] = z33;
            zcosg = zcosgl;
            zsing = zsingl;
            zcosi = zcosil;
            zsini = zsinil;
            zcosh = zcoshl * cnodm + zsinhl * snodm;
            zsinh = snodm * zcoshl - cnodm * zsinhl;
            cc = C1L;
        }
    }

    let zmol = (4.719_967_2 + 0.229_971_50 * day - gam) % TWO_PI;
    let zmos = (6.256_583_7 + 0.017_201_977 * day) % TWO_PI;

    DeepSpace {
        snodm,
        cnodm,
        sinim,
        cosim,
        sinomm,
        cosomm,
        day,
        e3: 2.0 * s[1] * s[7],
        ee2: 2.0 * s[1] * s[6],
        em,
        emsq,
        gam,
        peo: 0.0,
        pgho: 0.0,
        pho: 0.0,
        pinco: 0.0,
        plo: 0.0,
        rtemsq,
        se2: 2.0 * ss[1] * ss[6],
        se3: 2.0 * ss[1] * ss[7],
        sgh2: 2.0 * ss[4] * sz[32],
        sgh3: 2.0 * ss[4] * (sz[33] - sz[31]),
        sgh4: -18.0 * ss[4] * ZES,
        sh2: -2.0 * ss[2] * sz[22],
        sh3: -2.0 * ss[2] * (sz[23] - sz[21]),
        si2: 2.0 * ss[2] * sz[12],
        si3: 2.0 * ss[2] * (sz[13] - sz[11]),
        sl2: -2.0 * ss[3] * sz[2],
        sl3: -2.0 * ss[3] * (sz[3] - sz[1]),
        sl4: -2.0 * ss[3] * (-21.0 - 9.0 * emsq) * ZES,
        s1: s[1],
        s2: s[2],
        s3: s[3],
        s4: s[4],
        s5: s[5],
        s6: s[6],
        s7: s[7],
        ss1: ss[1],
        ss2: ss[2],
        ss3: ss[3],
        ss4: ss[4],
        ss5: ss[5],
        ss6: ss[6],
        ss7: ss[7],
        sz1: sz[1],
        sz2: sz[2],
        sz3: sz[3],
        sz11: sz[11],
        sz12: sz[12],
        sz13: sz[13],
        sz21: sz[21],
        sz22: sz[22],
        sz23: sz[23],
        sz31: sz[31],
        sz32: sz[32],
        sz33: sz[33],
        z11_final: z11,
        z13_final: z13,
        z1_final: z1,
        z3_final: z3,
        z21_final: z21,
        z23_final: z23,
        z31_final: z31,
        z33_final: z33,
        xgh2: 2.0 * s[4] * z32,
        xgh3: 2.0 * s[4] * (z33 - z31),
        xgh4: -18.0 * s[4] * ZEL,
        xh2: -2.0 * s[2] * z22,
        xh3: -2.0 * s[2] * (z23 - z21),
        xi2: 2.0 * s[2] * z12,
        xi3: 2.0 * s[2] * (z13 - z11),
        xl2: -2.0 * s[3] * z2,
        xl3: -2.0 * s[3] * (z3 - z1),
        xl4: -2.0 * s[3] * (-21.0 - 9.0 * emsq) * ZEL,
        zmol,
        zmos,
        // Filled in by `dsinit`.
        resonance: Resonance::None,
        dedt: 0.0,
        didt: 0.0,
        dmdt: 0.0,
        dnodt: 0.0,
        domdt: 0.0,
        del1: 0.0,
        del2: 0.0,
        del3: 0.0,
        xfact: 0.0,
        xlamo: 0.0,
        d2201: 0.0,
        d2211: 0.0,
        d3210: 0.0,
        d3222: 0.0,
        d4410: 0.0,
        d4422: 0.0,
        d5220: 0.0,
        d5232: 0.0,
        d5421: 0.0,
        d5433: 0.0,
    }
}

/// The output of `dsinit`: the deep-space secular rates and resonance coefficients.
struct DeepSpaceInit {
    resonance: Resonance,
    dedt: f64,
    didt: f64,
    dmdt: f64,
    dnodt: f64,
    domdt: f64,
    del1: f64,
    del2: f64,
    del3: f64,
    xfact: f64,
    xlamo: f64,
    d2201: f64,
    d2211: f64,
    d3210: f64,
    d3222: f64,
    d4410: f64,
    d4422: f64,
    d5220: f64,
    d5232: f64,
    d5421: f64,
    d5433: f64,
}

/// The `dsinit` deep-space secular rates and geopotential resonance terms.
///
/// Source: AIAA 2006-6753 `dsinit`. The geopotential coefficients (`q22`, `q31`, `q33` and
/// the `root*` functions of them) belong to the WGS-72 field and are part of the theory, not
/// tuning parameters.
#[allow(clippy::too_many_arguments)]
fn dsinit(
    ds: &DeepSpace,
    tc: f64,
    cosim: f64,
    sinim: f64,
    inclm: f64,
    eccsq: f64,
    ecco: f64,
    argpo: f64,
    mo: f64,
    no_unkozai: f64,
    nodeo: f64,
    gsto: f64,
    mdot: f64,
    nodedot: f64,
    xpidot: f64,
    gravity: crate::constants::GravityField,
) -> DeepSpaceInit {
    // Source constants, `dsinit`.
    const Q22: f64 = 1.789_167_9e-6;
    const Q31: f64 = 2.146_074_8e-6;
    const Q33: f64 = 2.212_301_5e-7;
    const ROOT22: f64 = 1.789_167_9e-6;
    const ROOT44: f64 = 7.363_695_3e-9;
    const ROOT54: f64 = 2.176_580_3e-9;
    const ROOT32: f64 = 3.739_379_2e-7;
    const ROOT52: f64 = 1.142_863_9e-7;
    const ZNL: f64 = 1.583_521_8e-4;
    const ZNS: f64 = 1.194_59e-5;

    let emsq = ds.emsq;

    let mut resonance = Resonance::None;
    if no_unkozai < SYNCHRONOUS_MEAN_MOTION_MAX && no_unkozai > SYNCHRONOUS_MEAN_MOTION_MIN {
        resonance = Resonance::Synchronous;
    }
    if (HALF_DAY_MEAN_MOTION_MIN..=HALF_DAY_MEAN_MOTION_MAX).contains(&no_unkozai)
        && ecco >= HALF_DAY_ECCENTRICITY_MIN
    {
        resonance = Resonance::HalfDay;
    }

    // Solar secular terms.
    //
    // The arguments follow the C++ reference's `sgp4init` call to `dsinit`: each `sz`/`ss`
    // value is the solar capture from the first `dscom` iteration, and `z11`/`z13` are the
    // values `dscom` left behind, which by then belong to the lunar iteration. The published
    // verification output was produced by exactly that argument list, so it is reproduced
    // here rather than reinterpreted.
    let ses = ds.ss1 * ZNS * ds.ss5;
    // `sis` reads the *solar* `sz` capture, unlike the lunar block below.
    let sis = ds.ss2 * ZNS * (ds.sz11 + ds.sz13);
    let sls = -ZNS * ds.ss3 * (ds.sz1 + ds.sz3 - 14.0 - 6.0 * emsq);
    let sghs = ds.ss4 * ZNS * (ds.sz31 + ds.sz33 - 6.0);
    let mut shs = -ZNS * ds.ss2 * (ds.sz21 + ds.sz23);
    // `sgp4fix for 180 deg incl`: near 0 or 180 degrees the node terms are singular, and the
    // reference implementation drops them rather than dividing by sin(i) ≈ 0.
    if inclm < 5.235_987_7e-2 || inclm > core::f64::consts::PI - 5.235_987_7e-2 {
        shs = 0.0;
    }
    if sinim != 0.0 {
        shs /= sinim;
    }
    let sgs = sghs - cosim * shs;

    // Lunar secular terms.
    //
    // These use the `s` and `z` values `dscom` leaves after its second iteration — the lunar
    // set — while the solar block above uses the `ss`/`sz` capture from its first. Mixing the
    // two up is easy because the reference implementation's `dsinit` prototype names its
    // parameters `sz11`, `z11` and so on in the opposite order to how it uses them; the
    // values below are the ones that reproduce its published output.
    let dedt = ses + ds.s1 * ZNL * ds.s5;
    let didt = sis + ds.s2 * ZNL * (ds.z11_final + ds.z13_final);
    let dmdt = sls - ZNL * ds.s3 * (ds.z1_final + ds.z3_final - 14.0 - 6.0 * emsq);
    let sghl = ds.s4 * ZNL * (ds.z31_final + ds.z33_final - 6.0);
    let mut shl = -ZNL * ds.s2 * (ds.z21_final + ds.z23_final);
    if inclm < 5.235_987_7e-2 || inclm > core::f64::consts::PI - 5.235_987_7e-2 {
        shl = 0.0;
    }
    let mut domdt = sgs + sghl;
    let mut dnodt = shs;
    if sinim != 0.0 {
        domdt -= cosim / sinim * shl;
        dnodt += shl / sinim;
    }

    let mut result = DeepSpaceInit {
        resonance,
        dedt,
        didt,
        dmdt,
        dnodt,
        domdt,
        del1: 0.0,
        del2: 0.0,
        del3: 0.0,
        xfact: 0.0,
        xlamo: 0.0,
        d2201: 0.0,
        d2211: 0.0,
        d3210: 0.0,
        d3222: 0.0,
        d4410: 0.0,
        d4422: 0.0,
        d5220: 0.0,
        d5232: 0.0,
        d5421: 0.0,
        d5433: 0.0,
    };

    let g = gravity.constants();
    let theta = (gsto + tc * EARTH_ROTATION_RAD_PER_MIN) % TWO_PI;
    let aonv = (no_unkozai / g.xke).powf(2.0 / 3.0);

    match resonance {
        Resonance::None => {}
        Resonance::HalfDay => {
            let cosisq = cosim * cosim;
            let em = ecco;
            let emsqo = eccsq;
            let eoc = em * emsqo;
            let g201 = -0.306 - (em - 0.64) * 0.440;
            let (g211, g310, g322, g410, g422, g520) = if em <= 0.65 {
                (
                    3.616 - 13.2470 * em + 16.2900 * emsqo,
                    -19.302 + 117.3900 * em - 228.4190 * emsqo + 156.591 * eoc,
                    -18.9068 + 109.7927 * em - 214.6334 * emsqo + 146.5816 * eoc,
                    -41.122 + 242.6940 * em - 471.0940 * emsqo + 313.953 * eoc,
                    -146.407 + 841.8800 * em - 1629.014 * emsqo + 1083.435 * eoc,
                    -532.114 + 3017.977 * em - 5740.032 * emsqo + 3708.276 * eoc,
                )
            } else {
                let g520 = if em > 0.715 {
                    -5149.66 + 29_936.92 * em - 54_087.36 * emsqo + 31_324.56 * eoc
                } else {
                    1464.74 - 4664.75 * em + 3763.64 * emsqo
                };
                (
                    -72.099 + 331.819 * em - 508.738 * emsqo + 266.724 * eoc,
                    -346.844 + 1582.851 * em - 2415.925 * emsqo + 1246.113 * eoc,
                    -342.585 + 1554.908 * em - 2366.899 * emsqo + 1215.972 * eoc,
                    -1052.797 + 4758.686 * em - 7193.992 * emsqo + 3651.957 * eoc,
                    -3581.690 + 16_178.11 * em - 24_462.77 * emsqo + 12_422.52 * eoc,
                    g520,
                )
            };
            let (g533, g521, g532) = if em < 0.7 {
                (
                    -919.22770 + 4988.6100 * em - 9064.7700 * emsqo + 5542.21 * eoc,
                    -822.71072 + 4568.6173 * em - 8491.4146 * emsqo + 5337.524 * eoc,
                    -853.66600 + 4690.2500 * em - 8624.7700 * emsqo + 5341.4 * eoc,
                )
            } else {
                (
                    -37_995.780 + 161_616.52 * em - 229_838.20 * emsqo + 109_377.94 * eoc,
                    -51_752.104 + 218_913.95 * em - 309_468.16 * emsqo + 146_349.42 * eoc,
                    -40_023.880 + 170_470.89 * em - 242_699.48 * emsqo + 115_605.82 * eoc,
                )
            };
            let sini2 = sinim * sinim;
            let f220 = 0.75 * (1.0 + 2.0 * cosim + cosisq);
            let f221 = 1.5 * sini2;
            let f321 = 1.875 * sinim * (1.0 - 2.0 * cosim - 3.0 * cosisq);
            let f322 = -1.875 * sinim * (1.0 + 2.0 * cosim - 3.0 * cosisq);
            let f441 = 35.0 * sini2 * f220;
            let f442 = 39.3750 * sini2 * sini2;
            let f522 = 9.84375
                * sinim
                * (sini2 * (1.0 - 2.0 * cosim - 5.0 * cosisq)
                    + 0.333_333_33 * (-2.0 + 4.0 * cosim + 6.0 * cosisq));
            let f523 = sinim
                * (4.921_875_12 * sini2 * (-2.0 - 4.0 * cosim + 10.0 * cosisq)
                    + 6.562_500_12 * (1.0 + 2.0 * cosim - 3.0 * cosisq));
            let f542 = 29.53125
                * sinim
                * (2.0 - 8.0 * cosim + cosisq * (-12.0 + 8.0 * cosim + 10.0 * cosisq));
            let f543 = 29.53125
                * sinim
                * (-2.0 - 8.0 * cosim + cosisq * (12.0 + 8.0 * cosim - 10.0 * cosisq));

            let xno2 = no_unkozai * no_unkozai;
            let ainv2 = aonv * aonv;
            let mut temp1 = 3.0 * xno2 * ainv2;
            let temp = temp1 * ROOT22;
            result.d2201 = temp * f220 * g201;
            result.d2211 = temp * f221 * g211;
            temp1 *= aonv;
            let temp = temp1 * ROOT32;
            result.d3210 = temp * f321 * g310;
            result.d3222 = temp * f322 * g322;
            temp1 *= aonv;
            let temp = 2.0 * temp1 * ROOT44;
            result.d4410 = temp * f441 * g410;
            result.d4422 = temp * f442 * g422;
            temp1 *= aonv;
            let temp = temp1 * ROOT52;
            result.d5220 = temp * f522 * g520;
            result.d5232 = temp * f523 * g532;
            let temp = 2.0 * temp1 * ROOT54;
            result.d5421 = temp * f542 * g521;
            result.d5433 = temp * f543 * g533;
            result.xlamo = (mo + nodeo + nodeo - theta - theta) % TWO_PI;
            result.xfact =
                mdot + dmdt + 2.0 * (nodedot + dnodt - EARTH_ROTATION_RAD_PER_MIN) - no_unkozai;
        }
        Resonance::Synchronous => {
            let g200 = 1.0 + emsq * (-2.5 + 0.8125 * emsq);
            let g310 = 1.0 + 2.0 * emsq;
            let g300 = 1.0 + emsq * (-6.0 + 6.60937 * emsq);
            let f220 = 0.75 * (1.0 + cosim) * (1.0 + cosim);
            let f311 = 0.9375 * sinim * sinim * (1.0 + 3.0 * cosim) - 0.75 * (1.0 + cosim);
            let f330 = 1.0 + cosim;
            let f330 = 1.875 * f330 * f330 * f330;
            let del1 = 3.0 * no_unkozai * no_unkozai * aonv * aonv;
            result.del2 = 2.0 * del1 * f220 * g200 * Q22;
            result.del3 = 3.0 * del1 * f330 * g300 * Q33 * aonv;
            result.del1 = del1 * f311 * g310 * Q31 * aonv;
            result.xlamo = (mo + nodeo + argpo - theta) % TWO_PI;
            result.xfact =
                mdot + xpidot - EARTH_ROTATION_RAD_PER_MIN + dmdt + domdt + dnodt - no_unkozai;
        }
    }

    result
}

/// The state of the deep-space periodic terms after `dpper`.
struct DeepSpacePeriodics {
    eccp: f64,
    inclp: f64,
    nodep: f64,
    argpp: f64,
    mp: f64,
}

/// The `dpper` lunar and solar long-period periodic perturbations.
///
/// Source: AIAA 2006-6753 `dpper`. The periodic terms are zero at epoch by construction, so
/// they are built from coefficients that are already reduced against the epoch values. The
/// Lyddane modification below 0.2 rad of inclination exists because the plain substitution
/// has a singularity there.
#[allow(clippy::too_many_arguments)]
fn dpper(
    ds: &DeepSpace,
    t: f64,
    inclo: f64,
    eccp: f64,
    inclp: f64,
    nodep: f64,
    argpp: f64,
    mp: f64,
) -> DeepSpacePeriodics {
    const ZES: f64 = 0.016_75;
    const ZEL: f64 = 0.054_90;
    const ZNS: f64 = 1.194_59e-5;
    const ZNL: f64 = 1.583_521_8e-4;

    let mut eccp = eccp;
    let mut inclp = inclp;
    let mut nodep = nodep;
    let mut argpp = argpp;
    let mut mp = mp;

    // Solar periodics.
    let zm = ds.zmos + ZNS * t;
    let zf = zm + 2.0 * ZES * zm.sin();
    let sinzf = zf.sin();
    let f2 = 0.5 * sinzf * sinzf - 0.25;
    let f3 = -0.5 * sinzf * zf.cos();
    let ses = ds.se2 * f2 + ds.se3 * f3;
    let sis = ds.si2 * f2 + ds.si3 * f3;
    let sls = ds.sl2 * f2 + ds.sl3 * f3 + ds.sl4 * sinzf;
    let sghs = ds.sgh2 * f2 + ds.sgh3 * f3 + ds.sgh4 * sinzf;
    let shs = ds.sh2 * f2 + ds.sh3 * f3;

    // Lunar periodics.
    let zm = ds.zmol + ZNL * t;
    let zf = zm + 2.0 * ZEL * zm.sin();
    let sinzf = zf.sin();
    let f2 = 0.5 * sinzf * sinzf - 0.25;
    let f3 = -0.5 * sinzf * zf.cos();
    let sel = ds.ee2 * f2 + ds.e3 * f3;
    let sil = ds.xi2 * f2 + ds.xi3 * f3;
    let sll = ds.xl2 * f2 + ds.xl3 * f3 + ds.xl4 * sinzf;
    let sghl = ds.xgh2 * f2 + ds.xgh3 * f3 + ds.xgh4 * sinzf;
    let shl = ds.xh2 * f2 + ds.xh3 * f3;

    let pe = ses + sel;
    let pinc = sis + sil;
    let pl = sls + sll;
    let mut pgh = sghs + sghl;
    let mut ph = shs + shl;

    // Subtract the epoch values, then apply.
    let pe = pe - ds.peo;
    let pinc = pinc - ds.pinco;
    let pl = pl - ds.plo;
    pgh -= ds.pgho;
    ph -= ds.pho;
    inclp += pinc;
    eccp += pe;
    let sinip = inclp.sin();
    let cosip = inclp.cos();

    if inclp >= LYDDANE_INCLINATION_RAD {
        ph /= sinip;
        pgh -= cosip * ph;
        argpp += pgh;
        nodep += ph;
        mp += pl;
    } else {
        // Lyddane modification. Source: AIAA 2006-6753 `dpper`, the `else` branch.
        let sinop = nodep.sin();
        let cosop = nodep.cos();
        let alfdp = sinip * sinop + (ph * cosop + pinc * cosip * sinop);
        let betdp = sinip * cosop + (-ph * sinop + pinc * cosip * cosop);
        let nodep_mod = nodep % TWO_PI;
        // `sgp4fix for afspc written intrinsic functions`: the AFSPC mode keeps the node in
        // [0, 2pi) before it is used without a trigonometric function ahead of it.
        let nodep_mod = if nodep_mod < 0.0 && AFSPC_MODE {
            nodep_mod + TWO_PI
        } else {
            nodep_mod
        };
        let xls = (mp + argpp + cosip * nodep_mod) + (pl + pgh - pinc * nodep_mod * sinip);
        let xnoh = nodep_mod;
        let mut nodep_new = alfdp.atan2(betdp);
        if nodep_new < 0.0 && AFSPC_MODE {
            nodep_new += TWO_PI;
        }
        // `atan2` returns a value in (-pi, pi]; the node must stay on the same branch it was
        // on, or the mean anomaly below jumps by a full turn.
        if (xnoh - nodep_new).abs() > core::f64::consts::PI {
            if nodep_new < xnoh {
                nodep_new += TWO_PI;
            } else {
                nodep_new -= TWO_PI;
            }
        }
        mp += pl;
        argpp = xls - mp - cosip * nodep_new;
        nodep = nodep_new;
    }

    let _ = inclo;
    DeepSpacePeriodics {
        eccp,
        inclp,
        nodep,
        argpp,
        mp,
    }
}

/// The result of the `dspace` resonance integration.
struct DeepSpaceSecular {
    eccm: f64,
    argpm: f64,
    inclm: f64,
    mm: f64,
    nodem: f64,
    xn: f64,
}

/// The `dspace` deep-space secular update, including the geopotential resonance integrator.
///
/// Source: AIAA 2006-6753 `dspace`. The resonance is integrated with an Euler-Maclaurin
/// predictor over 720-minute steps, which is why this is a loop rather than a closed form.
#[allow(clippy::too_many_arguments)]
fn dspace(
    ds: &DeepSpace,
    state: &mut ResonanceState,
    argpo: f64,
    argpdot: f64,
    t: f64,
    gsto: f64,
    no_unkozai: f64,
    em: f64,
    argpm: f64,
    inclm: f64,
    mm: f64,
    nodem: f64,
    xn: f64,
) -> DeepSpaceSecular {
    // Source constants, `dspace`.
    const FASX2: f64 = 0.131_309_08;
    const FASX4: f64 = 2.884_319_8;
    const FASX6: f64 = 0.374_480_87;
    const G22: f64 = 5.768_639_6;
    const G32: f64 = 0.952_408_98;
    const G44: f64 = 1.801_499_8;
    const G52: f64 = 1.050_833_0;
    const G54: f64 = 4.410_889_8;

    let eccm = em + ds.dedt * t;
    // `sgp4fix for negative inclinations` is deliberately *not* applied here: the reference
    // keeps it commented out, and the flip belongs after the periodics instead.
    let inclm = inclm + ds.didt * t;
    let argpm = argpm + ds.domdt * t;
    // `t` is `tc`, which `sgp4` sets to the elapsed time before calling this.
    let theta = (gsto + t * EARTH_ROTATION_RAD_PER_MIN) % TWO_PI;
    let nodem = nodem + ds.dnodt * t;
    let mut mm = mm + ds.dmdt * t;
    let mut xn = xn;

    if ds.resonance != Resonance::None {
        // Epoch restart. The reference implementation keeps `atime`, `xli` and `xni` in its
        // satellite structure, so a run that steps forward keeps the integrator history.
        // Restarting from the epoch while stepping past the epoch, or closing on it, resets
        // all three; otherwise the stored state is the starting point.
        if state.atime == 0.0 || t * state.atime <= 0.0 || t.abs() < state.atime.abs() {
            state.atime = 0.0;
            state.xni = no_unkozai;
            state.xli = ds.xlamo;
        }
        let mut xni = state.xni;
        let mut xli = state.xli;
        let mut atime = state.atime;
        let delt = if t > 0.0 {
            RESONANCE_STEP_MINUTES
        } else {
            -RESONANCE_STEP_MINUTES
        };
        let step2 = RESONANCE_STEP2;

        #[allow(unused_assignments)]
        let (mut xndt, mut xldot, mut xnddt) = (0.0, 0.0, 0.0);
        loop {
            // The half-day branch is taken only for the 12-hour resonance; every other case,
            // including the one-day one, uses the near-synchronous terms.
            #[allow(clippy::if_not_else)]
            if ds.resonance != Resonance::HalfDay {
                // Near-synchronous resonance terms.
                xndt = ds.del1 * (xli - FASX2).sin()
                    + ds.del2 * (2.0 * (xli - FASX4)).sin()
                    + ds.del3 * (3.0 * (xli - FASX6)).sin();
                xldot = xni + ds.xfact;
                xnddt = ds.del1 * (xli - FASX2).cos()
                    + 2.0 * ds.del2 * (2.0 * (xli - FASX4)).cos()
                    + 3.0 * ds.del3 * (3.0 * (xli - FASX6)).cos();
                xnddt *= xldot;
            } else {
                // Near half-day resonance terms.
                let xomi = argpo + argpdot * atime;
                let x2omi = xomi + xomi;
                let x2li = xli + xli;
                xndt = ds.d2201 * (x2omi + xli - G22).sin()
                    + ds.d2211 * (xli - G22).sin()
                    + ds.d3210 * (xomi + xli - G32).sin()
                    + ds.d3222 * (-xomi + xli - G32).sin()
                    + ds.d4410 * (x2omi + x2li - G44).sin()
                    + ds.d4422 * (x2li - G44).sin()
                    + ds.d5220 * (xomi + xli - G52).sin()
                    + ds.d5232 * (-xomi + xli - G52).sin()
                    + ds.d5421 * (xomi + x2li - G54).sin()
                    + ds.d5433 * (-xomi + x2li - G54).sin();
                xldot = xni + ds.xfact;
                xnddt = ds.d2201 * (x2omi + xli - G22).cos()
                    + ds.d2211 * (xli - G22).cos()
                    + ds.d3210 * (xomi + xli - G32).cos()
                    + ds.d3222 * (-xomi + xli - G32).cos()
                    + ds.d5220 * (xomi + xli - G52).cos()
                    + ds.d5232 * (-xomi + xli - G52).cos()
                    + 2.0
                        * (ds.d4410 * (x2omi + x2li - G44).cos()
                            + ds.d4422 * (x2li - G44).cos()
                            + ds.d5421 * (xomi + x2li - G54).cos()
                            + ds.d5433 * (-xomi + x2li - G54).cos());
                xnddt *= xldot;
            }

            if (t - atime).abs() >= RESONANCE_STEP_MINUTES {
                xli += xldot * delt + xndt * step2;
                xni += xndt * delt + xnddt * step2;
                atime += delt;
            } else {
                break;
            }
        }

        let ft = t - atime;
        xn = xni + xndt * ft + xnddt * ft * ft * 0.5;
        let xl = xli + xldot * ft + xndt * ft * ft * 0.5;
        if ds.resonance == Resonance::Synchronous {
            mm = xl - nodem - argpm + theta;
        } else {
            mm = xl - 2.0 * nodem + 2.0 * theta;
        }
        let dndt = xn - no_unkozai;
        xn = no_unkozai + dndt;
        state.atime = atime;
        state.xli = xli;
        state.xni = xni;
    }

    DeepSpaceSecular {
        eccm,
        argpm,
        inclm,
        mm,
        nodem,
        xn,
    }
}
