//! The `STKCraft` error type.
//!
//! Every crate in the workspace returns [`Result`] and threads errors with `?`. There is
//! no panic path: an unfinished feature returns [`Error::Unsupported`], an out-of-range
//! input returns a validation error, and the caller decides what to show.

use std::fmt;

/// The result type used throughout `STKCraft`.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong in the analysis layers.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Error {
    /// A caller supplied a value outside its permitted range.
    ///
    /// `what` names the quantity, `value` is the offending value as text (so the error
    /// stays comparable without boxing floats), and `expected` describes the range.
    OutOfRange {
        /// The quantity that was out of range, e.g. `"semi-major axis"`.
        what: &'static str,
        /// The offending value, formatted for display.
        value: String,
        /// A human-readable description of the permitted range.
        expected: &'static str,
    },
    /// A value was mathematically undefined for the operation requested.
    Undefined {
        /// What was undefined, e.g. `"eccentricity of a circular orbit"`.
        what: &'static str,
    },
    /// A required input was missing or could not be interpreted.
    Invalid {
        /// What was invalid.
        what: &'static str,
        /// Why it was rejected.
        reason: String,
    },
    /// The requested capability exists in the API but is not implemented yet.
    ///
    /// This is the *only* sanctioned way to represent an unfinished feature; it must
    /// never be modelled as a panic.
    Unsupported {
        /// The capability that is not available.
        what: &'static str,
    },
    /// A resource the request needed was not found.
    NotFound {
        /// What was being looked for.
        what: String,
    },
}

impl Error {
    /// Build an [`Error::OutOfRange`] from any displayable value.
    #[must_use]
    pub fn out_of_range(
        what: &'static str,
        value: impl fmt::Display,
        expected: &'static str,
    ) -> Self {
        Self::OutOfRange {
            what,
            value: value.to_string(),
            expected,
        }
    }

    /// Build an [`Error::Invalid`] from any displayable reason.
    #[must_use]
    pub fn invalid(what: &'static str, reason: impl fmt::Display) -> Self {
        Self::Invalid {
            what,
            reason: reason.to_string(),
        }
    }

    /// Build an [`Error::Unsupported`] for a capability that is not built yet.
    #[must_use]
    pub const fn unsupported(what: &'static str) -> Self {
        Self::Unsupported { what }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange {
                what,
                value,
                expected,
            } => write!(f, "{what} = {value} is out of range ({expected})"),
            Self::Undefined { what } => write!(f, "{what} is undefined"),
            Self::Invalid { what, reason } => write!(f, "invalid {what}: {reason}"),
            Self::Unsupported { what } => write!(f, "{what} is not available yet"),
            Self::NotFound { what } => write!(f, "{what} was not found"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_name_the_quantity_and_the_range() {
        let e = Error::out_of_range("eccentricity", 1.5, "0 <= e < 1");
        assert_eq!(
            e.to_string(),
            "eccentricity = 1.5 is out of range (0 <= e < 1)"
        );
    }

    #[test]
    fn unsupported_reads_as_not_available_rather_than_broken() {
        assert_eq!(
            Error::unsupported("coverage").to_string(),
            "coverage is not available yet"
        );
    }
}
