//! The SGP4 error codes and what they mean.
//!
//! The reference implementation reports a non-zero integer from `sgp4init` and `sgp4`, and
//! callers have relied on those numbers for thirty years. This crate keeps the same
//! numbering — a caller porting filtering logic or a test file that greps for "error 6"
//! keeps working — but as an enum, so the meaning travels with the value.
//!
//! The meanings are transcribed from the `sgp4init` and `sgp4` doc comments in the
//! AIAA 2006-6753 reference source, `sgp4unit.cpp`.

use core::fmt;

use stkcraft_core::Error;

/// A failure to propagate, with the reference implementation's code where one exists.
///
/// Propagating a decayed or sub-orbital element set is a *modelling* failure, not a bug:
/// the mean elements no longer describe a state vector, and returning a plausible number
/// would be worse than returning this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PropagationError {
    /// `1` — mean elements out of family: `ecc >= 1.0`, `ecc < -0.001`, or `a < 0.95`.
    ///
    /// Usually means the element set is ancient, malformed, or the drag term has walked the
    /// orbit below the surface of the Earth.
    MeanElementsOutOfFamily,
    /// `2` — mean motion went non-positive.
    ///
    /// The drag model has removed all the orbital motion, which happens on a decaying orbit
    /// a little before it reaches the Earth.
    NonPositiveMeanMotion,
    /// `3` — a deep-space periodic perturbation pushed `ecc` outside `[0, 1]`.
    PerturbedEccentricityOutOfFamily,
    /// `4` — the semi-latus rectum came out negative, so the conic is not real.
    NonPositiveSemiLatusRectum,
    /// `5` — the element set describes a sub-orbital trajectory at epoch.
    ///
    /// The perigee radius is inside the Earth, so there is no orbit to propagate.
    SubOrbitalElements,
    /// `6` — the satellite has decayed: its radius has fallen inside one Earth radius.
    Decayed,
}

impl PropagationError {
    /// The code the reference implementation returns for this condition.
    ///
    /// Reproducing the numbering lets a caller compare against published logs and against
    /// the expected output in the verification file.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::MeanElementsOutOfFamily => 1,
            Self::NonPositiveMeanMotion => 2,
            Self::PerturbedEccentricityOutOfFamily => 3,
            Self::NonPositiveSemiLatusRectum => 4,
            Self::SubOrbitalElements => 5,
            Self::Decayed => 6,
        }
    }

    /// A short description suitable for a log line or a UI status.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::MeanElementsOutOfFamily => {
                "mean elements out of family (eccentricity or semi-major axis)"
            }
            Self::NonPositiveMeanMotion => "mean motion is not positive",
            Self::PerturbedEccentricityOutOfFamily => "perturbed eccentricity out of family",
            Self::NonPositiveSemiLatusRectum => "semi-latus rectum is not positive",
            Self::SubOrbitalElements => "element set is sub-orbital at epoch",
            Self::Decayed => "satellite has decayed",
        }
    }
}

impl fmt::Display for PropagationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SGP4 error {}: {}", self.code(), self.description())
    }
}

impl std::error::Error for PropagationError {}

impl From<PropagationError> for Error {
    fn from(value: PropagationError) -> Self {
        Self::Invalid {
            what: "orbit state",
            reason: value.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_match_the_reference_implementation() {
        // The numbering is part of the interface; see the `sgp4init` doc comment in the
        // AIAA 2006-6753 source.
        assert_eq!(PropagationError::MeanElementsOutOfFamily.code(), 1);
        assert_eq!(PropagationError::NonPositiveMeanMotion.code(), 2);
        assert_eq!(PropagationError::PerturbedEccentricityOutOfFamily.code(), 3);
        assert_eq!(PropagationError::NonPositiveSemiLatusRectum.code(), 4);
        assert_eq!(PropagationError::SubOrbitalElements.code(), 5);
        assert_eq!(PropagationError::Decayed.code(), 6);
    }

    #[test]
    fn the_message_names_the_code_and_the_condition() {
        assert_eq!(
            PropagationError::Decayed.to_string(),
            "SGP4 error 6: satellite has decayed"
        );
    }
}
