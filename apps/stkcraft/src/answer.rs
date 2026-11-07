//! One command's answer, rendered for a person and for a program.
//!
//! The engine returns a [`CommandOutcome`] whose lists carry offsets from the scenario epoch; a
//! caller that prints only the summary has thrown the answer away. Both the command line and the
//! window's control channel use this module, so "the times the tool computed" mean the same thing
//! in both places.

use std::fmt::Write as _;

use serde_json::{Value, json};
use stkcraft_astro_engine::analysis::TargetAccess;
use stkcraft_astro_engine::{CommandOutcome, SatelliteAccess, Scenario};
use stkcraft_framework_time::Epoch;

use crate::render::{format_duration, format_epoch};

/// How many windows of one pair the text lists before it points at the JSON for the rest.
const LISTED: usize = 50;

/// The answer as text: the summary first, then every window it computed.
#[must_use]
pub fn text(outcome: &CommandOutcome, scenario: &Scenario) -> String {
    let mut text = outcome.summary.clone();
    let epoch = scenario.epoch;
    if let Some(access) = &outcome.access {
        for entry in access {
            let satellite = satellite_name(scenario, entry.satellite);
            let station = station_name(scenario, entry.station);
            let _ = write!(
                text,
                "\n{satellite} over {station}: {} window(s), {} in view",
                entry.windows.len(),
                format_duration(entry.total.as_seconds())
            );
            append_windows(&mut text, epoch, &entry.windows);
        }
    }
    if let Some(passes) = &outcome.passes {
        let _ = write!(text, "\n{} pass(es):", passes.len());
        for pass in passes {
            let _ = write!(
                text,
                "\n  {} → {} UTC ({})",
                instant(epoch, pass.start_seconds),
                instant(epoch, pass.end_seconds),
                format_duration(pass.duration_seconds())
            );
        }
    }
    if let Some(targets) = &outcome.targets {
        for entry in targets {
            let satellite = satellite_name(scenario, entry.satellite);
            let _ = write!(
                text,
                "\n{satellite} over target `{}` ({} vertices): {} window(s), {} in view",
                entry.target,
                entry.vertices,
                entry.windows.len(),
                format_duration(entry.total.as_seconds())
            );
            append_windows(&mut text, epoch, &entry.windows);
        }
    }
    if !outcome.limitations.is_empty() {
        text.push_str("\n\nLimitations:");
        for limitation in &outcome.limitations {
            let _ = write!(text, "\n- {limitation}");
        }
    }
    if !outcome.formulas.is_empty() {
        text.push_str("\n\nFormulas:");
        for formula in &outcome.formulas {
            let _ = write!(text, "\n- {} — {}", formula.id, formula.source);
        }
    }
    text
}

/// The answer as JSON: the same windows, with absolute UTC instants and offsets.
#[must_use]
pub fn json(outcome: &CommandOutcome, scenario: &Scenario) -> Value {
    let epoch = scenario.epoch;
    json!({
        "summary": outcome.summary.clone(),
        "access": outcome.access.as_ref().map(|access| access_json(access, scenario, epoch)),
        "passes": outcome.passes.as_ref().map(|passes| passes_json(passes, epoch)),
        "targets": outcome.targets.as_ref().map(|targets| targets_json(targets, scenario, epoch)),
        "limitations": outcome.limitations.clone(),
        "formulas": outcome.formulas.iter().map(|formula| json!({
            "id": formula.id,
            "source": formula.source,
        })).collect::<Vec<Value>>(),
    })
}

fn access_json(access: &[SatelliteAccess], scenario: &Scenario, epoch: Epoch) -> Vec<Value> {
    access
        .iter()
        .map(|entry| {
            json!({
                "satellite": satellite_name(scenario, entry.satellite),
                "station": station_name(scenario, entry.station),
                "total_seconds": entry.total.as_seconds(),
                "windows": windows_json(&entry.windows, epoch),
            })
        })
        .collect()
}

fn passes_json(passes: &[stkcraft_astro_engine::PlanetPass], epoch: Epoch) -> Vec<Value> {
    passes
        .iter()
        .map(|pass| {
            json!({
                "start_utc": instant(epoch, pass.start_seconds),
                "end_utc": instant(epoch, pass.end_seconds),
                "start_seconds": pass.start_seconds,
                "duration_seconds": pass.duration_seconds(),
            })
        })
        .collect()
}

fn targets_json(targets: &[TargetAccess], scenario: &Scenario, epoch: Epoch) -> Vec<Value> {
    targets
        .iter()
        .map(|entry| {
            json!({
                "satellite": satellite_name(scenario, entry.satellite),
                "target": entry.target.clone(),
                "vertices": entry.vertices,
                "total_seconds": entry.total.as_seconds(),
                "windows": windows_json(&entry.windows, epoch),
            })
        })
        .collect()
}

fn windows_json(windows: &[stkcraft_astro_access::AccessWindow], epoch: Epoch) -> Vec<Value> {
    windows
        .iter()
        .map(|window| {
            json!({
                "start_utc": instant(epoch, window.start.as_seconds()),
                "end_utc": instant(epoch, window.end.as_seconds()),
                "start_seconds": window.start.as_seconds(),
                "duration_seconds": window.duration().as_seconds(),
            })
        })
        .collect()
}

fn append_windows(
    text: &mut String,
    epoch: Epoch,
    windows: &[stkcraft_astro_access::AccessWindow],
) {
    for (index, window) in windows.iter().enumerate() {
        if index >= LISTED {
            let _ = write!(
                text,
                "\n  … and {} more (all of them are in `--json`)",
                windows.len() - LISTED
            );
            break;
        }
        let _ = write!(
            text,
            "\n  {} → {} UTC ({})",
            instant(epoch, window.start.as_seconds()),
            instant(epoch, window.end.as_seconds()),
            format_duration(window.duration().as_seconds())
        );
    }
}

fn satellite_name(scenario: &Scenario, id: stkcraft_astro_engine::SatelliteId) -> &str {
    scenario
        .satellite(id)
        .map_or("?", |satellite| satellite.name.as_str())
}

fn station_name(scenario: &Scenario, id: stkcraft_astro_engine::StationId) -> &str {
    scenario
        .station(id)
        .map_or("?", |station| station.name.as_str())
}

/// An absolute instant, recovered from the scenario epoch plus an offset.
fn instant(epoch: Epoch, offset_seconds: f64) -> String {
    let at = Epoch::from_julian(epoch.julian().plus_seconds(offset_seconds), epoch.scale());
    format_epoch(at)
}

#[cfg(test)]
mod tests {
    use stkcraft_astro_engine::FormulaCitation;

    use super::*;

    #[test]
    fn a_summary_without_results_is_still_an_answer() {
        let scenario = Scenario::empty("answer test");
        let outcome = CommandOutcome::new("Scenario renamed");
        assert_eq!(text(&outcome, &scenario), "Scenario renamed");
        let value = json(&outcome, &scenario);
        assert_eq!(
            value.get("summary").and_then(Value::as_str),
            Some("Scenario renamed")
        );
        assert!(value.get("access").is_some_and(Value::is_null));
        assert!(
            value
                .get("limitations")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
        );
        assert!(
            value
                .get("formulas")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
        );
    }

    #[test]
    fn formulas_survive_the_text_and_json_answers() {
        let scenario = Scenario::empty("answer test");
        let outcome = CommandOutcome::new("Computed access").with_formulas(vec![FormulaCitation {
            id: "access.look-angles",
            source: "Vallado (2013)",
        }]);
        let text = text(&outcome, &scenario);
        assert!(text.contains("access.look-angles"), "{text}");
        assert!(text.contains("Vallado (2013)"), "{text}");
        let value = json(&outcome, &scenario);
        assert_eq!(
            value
                .get("formulas")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("id"))
                .and_then(Value::as_str),
            Some("access.look-angles")
        );
    }

    #[test]
    fn limitations_survive_the_text_and_json_answers() {
        let scenario = Scenario::empty("answer test");
        let outcome = CommandOutcome::new("Computed access")
            .with_limitations(vec!["sampled every 60 s".to_owned()]);
        let text = text(&outcome, &scenario);
        assert!(text.contains("Limitations:"), "{text}");
        assert!(text.contains("sampled every 60 s"), "{text}");
        let value = json(&outcome, &scenario);
        assert_eq!(
            value
                .get("limitations")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(Value::as_str),
            Some("sampled every 60 s")
        );
    }
}
