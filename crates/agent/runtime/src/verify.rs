//! Check a tool result before the platform shows it.
//!
//! A model can call a tool and then describe a number the tool did not return. This module is the
//! deterministic half of that boundary: it reads the structured content itself and checks the
//! invariants every access answer has to satisfy. It does not trust the model's prose, and it does
//! not replace the engine's own validation; it catches a result that was assembled or edited on the
//! way to the platform.
//!
//! The check is deliberately conservative. Unknown result kinds are left to their own command's
//! tests rather than rejected here, because this module knows only the access and pass shapes.

use chrono::NaiveDateTime;
use serde_json::Value;

/// How far a window's stated duration may differ from its two instants, in seconds.
///
/// The access result keeps the engine's floating-point seconds and the display strings are truncated
/// to whole seconds, so one second of difference is expected. Two seconds leaves room for the
/// truncation on both ends without accepting a wrong duration.
const DURATION_TOLERANCE_SECONDS: f64 = 2.0;

const INSTANT_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// Check every structured result, returning one message per failed invariant.
///
/// An empty error list means every known result shape passed. Unknown shapes are not errors.
#[must_use]
pub fn verify_results(results: &[Value]) -> Vec<String> {
    let mut errors = Vec::new();
    for (index, result) in results.iter().enumerate() {
        verify_result(index, result, &mut errors);
    }
    errors
}

fn verify_result(index: usize, result: &Value, errors: &mut Vec<String>) {
    let kind = result.get("kind").and_then(Value::as_str);
    match kind {
        Some("access" | "target-access") => {
            verify_named_evidence(index, result, errors);
            let Some(pairs) = result.get("pairs").and_then(Value::as_array) else {
                errors.push(format!("result {index}: access has no `pairs` array"));
                return;
            };
            for pair in pairs {
                let Some(windows) = pair.get("windows").and_then(Value::as_array) else {
                    errors.push(format!(
                        "result {index}: an access pair has no `windows` array"
                    ));
                    continue;
                };
                for window in windows {
                    verify_window(index, window, errors);
                }
            }
        }
        Some("planet-passes") => {
            verify_named_evidence(index, result, errors);
            let Some(passes) = result.get("passes").and_then(Value::as_array) else {
                errors.push(format!(
                    "result {index}: planet result has no `passes` array"
                ));
                return;
            };
            for pass in passes {
                verify_window(index, pass, errors);
            }
        }
        // A command that is not a pass computation has its own tests; this module does not know
        // its invariants and must not pretend it does.
        _ => {}
    }
}

fn verify_named_evidence(index: usize, result: &Value, errors: &mut Vec<String>) {
    let formulas = result.get("formulas").and_then(Value::as_array);
    if formulas.is_none_or(Vec::is_empty) {
        errors.push(format!("result {index}: no formula citations"));
    } else if let Some(formulas) = formulas {
        for formula in formulas {
            let id = formula
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let source = formula
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if id.trim().is_empty() || source.trim().is_empty() {
                errors.push(format!(
                    "result {index}: a formula citation has no id or source"
                ));
            }
        }
    }
    let limitations = result.get("limitations").and_then(Value::as_array);
    if limitations.is_none_or(Vec::is_empty) {
        errors.push(format!("result {index}: no limitations"));
    }
}

fn verify_window(index: usize, window: &Value, errors: &mut Vec<String>) {
    let start = window.get("start_utc").and_then(Value::as_str);
    let end = window.get("end_utc").and_then(Value::as_str);
    let duration = window.get("duration_seconds").and_then(Value::as_f64);
    let (Some(start), Some(end), Some(duration)) = (start, end, duration) else {
        errors.push(format!(
            "result {index}: a window is missing `start_utc`, `end_utc` or `duration_seconds`"
        ));
        return;
    };
    if !duration.is_finite() || duration < 0.0 {
        errors.push(format!(
            "result {index}: a window has an invalid duration `{duration}`"
        ));
        return;
    }
    let (Ok(start), Ok(end)) = (
        NaiveDateTime::parse_from_str(start, INSTANT_FORMAT),
        NaiveDateTime::parse_from_str(end, INSTANT_FORMAT),
    ) else {
        errors.push(format!(
            "result {index}: a window has an instant that is not `YYYY-MM-DD HH:MM:SS`"
        ));
        return;
    };
    let stated = end.signed_duration_since(start).num_milliseconds() as f64 / 1_000.0;
    if stated < 0.0 {
        errors.push(format!("result {index}: a window ends before it starts"));
        return;
    }
    if (stated - duration).abs() > DURATION_TOLERANCE_SECONDS {
        errors.push(format!(
            "result {index}: a window says {duration:.3} s but its instants span {stated:.3} s"
        ));
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use serde_json::json;

    use super::*;

    fn access() -> Value {
        json!({
            "kind": "access",
            "formulas": [{"id": "access.look-angles", "source": "Vallado (2013)"}],
            "limitations": ["sampled every 60 s"],
            "pairs": [{
                "satellite": "ISS",
                "station": "London",
                "windows": [{
                    "start_utc": "2023-12-31 13:28:18",
                    "end_utc": "2023-12-31 13:35:01",
                    "duration_seconds": 402.6
                }]
            }]
        })
    }

    #[test]
    fn a_consistent_access_result_passes() {
        assert_eq!(verify_results(&[access()]), Vec::<String>::new());
    }

    #[test]
    fn a_duration_that_does_not_match_its_instants_is_refused() {
        let mut result = access();
        result["pairs"][0]["windows"][0]["duration_seconds"] = json!(10.0);
        let errors = verify_results(&[result]);
        assert!(
            errors.iter().any(|error| error.contains("instants span")),
            "{errors:?}"
        );
    }

    #[test]
    fn a_missing_formula_or_limitation_is_refused() {
        let mut result = access();
        result["formulas"] = json!([]);
        let errors = verify_results(&[result]);
        assert!(
            errors.iter().any(|error| error.contains("formula")),
            "{errors:?}"
        );

        let mut result = access();
        result["limitations"] = json!([]);
        let errors = verify_results(&[result]);
        assert!(
            errors.iter().any(|error| error.contains("limitations")),
            "{errors:?}"
        );
    }

    #[test]
    fn an_unparseable_instant_is_refused() {
        let mut result = access();
        result["pairs"][0]["windows"][0]["start_utc"] = json!("yesterday");
        let errors = verify_results(&[result]);
        assert!(
            errors.iter().any(|error| error.contains("not `YYYY-MM-DD")),
            "{errors:?}"
        );
    }

    #[test]
    fn an_unknown_result_kind_is_not_this_modules_business() {
        assert_eq!(
            verify_results(&[json!({"kind": "satellite", "name": "ISS"})]),
            Vec::<String>::new()
        );
    }
}
