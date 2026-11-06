//! The reference time scales used by the analysis layers.

/// A reference time scale.
///
/// These are the scales a scenario can be expressed in. `UTC` is the only one with
/// discontinuities (leap seconds); every other scale advances uniformly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum TimeScale {
    /// Coordinated Universal Time. Follows atomic time but inserts leap seconds so that
    /// it stays within 0.9 s of UT1. Not uniform.
    Utc,
    /// International Atomic Time. The uniform atomic scale that UTC is derived from.
    Tai,
    /// Terrestrial Time. The scale of geocentric ephemerides; `TT = TAI + 32.184 s`.
    Tt,
    /// Barycentric Dynamical Time. The independent argument of barycentric ephemerides;
    /// differs from TT by a periodic term smaller than 2 ms.
    Tdb,
    /// Barycentric Coordinate Time. Uniform barycentric scale, `TDB + L_B * (JD - T0)`.
    Tcb,
    /// Geocentric Coordinate Time. Uniform geocentric scale, `TT + L_G * (JD - T0)`.
    Tcg,
    /// Universal Time UT1. Earth-rotation angle; derived from UTC through `DUT1`, which
    /// IERS publishes and which stays within 0.9 s of UTC.
    Ut1,
    /// GPS time. Uniform, `GPS = TAI - 19 s` with no leap seconds.
    Gps,
}

impl TimeScale {
    /// The name used in files, logs and the command interface.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Utc => "UTC",
            Self::Tai => "TAI",
            Self::Tt => "TT",
            Self::Tdb => "TDB",
            Self::Tcb => "TCB",
            Self::Tcg => "TCG",
            Self::Ut1 => "UT1",
            Self::Gps => "GPS",
        }
    }

    /// True when the scale advances in uniform SI seconds with no discontinuities.
    #[must_use]
    pub const fn is_uniform(self) -> bool {
        !matches!(self, Self::Utc | Self::Ut1)
    }

    /// Parse a scale name, case-insensitively.
    #[must_use]
    pub fn from_id(text: &str) -> Option<Self> {
        match text.trim().to_ascii_uppercase().as_str() {
            "UTC" => Some(Self::Utc),
            "TAI" => Some(Self::Tai),
            "TT" => Some(Self::Tt),
            "TDB" => Some(Self::Tdb),
            "TCB" => Some(Self::Tcb),
            "TCG" => Some(Self::Tcg),
            "UT1" => Some(Self::Ut1),
            "GPS" => Some(Self::Gps),
            _ => None,
        }
    }
}

/// Which conversion a caller is asking for, and what extra data it needs.
///
/// UT1 needs `DUT1 = UT1 - UTC`, which only the IERS publishes; it cannot be derived.
/// Carrying it as a parameter keeps the omission visible at the call site rather than
/// letting a default slip in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeScaleConversion {
    /// The scale being converted from.
    pub from: TimeScale,
    /// The scale being converted to.
    pub to: TimeScale,
    /// `UT1 - UTC` in seconds. Required only when either side is UT1.
    pub dut1: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for scale in [
            TimeScale::Utc,
            TimeScale::Tai,
            TimeScale::Tt,
            TimeScale::Tdb,
            TimeScale::Tcb,
            TimeScale::Tcg,
            TimeScale::Ut1,
            TimeScale::Gps,
        ] {
            assert_eq!(TimeScale::from_id(scale.id()), Some(scale));
        }
    }

    #[test]
    fn ids_parse_case_insensitively() {
        assert_eq!(TimeScale::from_id("utc"), Some(TimeScale::Utc));
        assert_eq!(TimeScale::from_id("  Tdb "), Some(TimeScale::Tdb));
        assert_eq!(TimeScale::from_id("nonsense"), None);
    }

    #[test]
    fn utc_and_ut1_are_not_uniform() {
        assert!(!TimeScale::Utc.is_uniform());
        assert!(!TimeScale::Ut1.is_uniform());
        assert!(TimeScale::Tai.is_uniform());
        assert!(TimeScale::Tt.is_uniform());
    }
}
