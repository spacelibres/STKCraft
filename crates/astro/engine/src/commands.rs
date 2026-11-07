//! The command table: the single entry point for everything a user can do.
//!
//! Every user-visible action is a [`Command`] with an id, a label, a menu path, documented
//! parameters, an `enabled` predicate and a `run` function. The menus, keyboard shortcuts, the
//! CLI, the control channel and the MCP server will all dispatch **by id**, so an agent can do
//! anything a user can. That is a testable property rather than an aspiration, and this module
//! is where it is tested: the tests below drive commands through [`Registry::run`] exactly as a
//! menu click or an automated call will.
//!
//! # Two rules that shape the signatures
//!
//! - **Programmatic calls never open a dialog.** There is no dialog to open yet, but every
//!   command takes all its inputs as [`Arg`]uments and none of them reads the environment,
//!   the clock or a file. A future interactive variant would be a *different* command.
//! - **The engine holds no global state.** `run` takes `&mut Scenario` and returns a value; a
//!   control channel will wrap that in whatever synchronisation it needs.

use std::collections::BTreeMap;

use stkcraft_astro_access::{GroundSite, Sensor};
use stkcraft_astro_sgp4::TwoLineElement;
use stkcraft_framework_core::{Angle, Distance, Duration, Error, Result};

use crate::analysis::{SatelliteAccess, compute_all_access};
use crate::central::PlanetOrbit;
use crate::planet_access::{PlanetPass, find_passes};
use crate::scenario::{Scenario, StationId};

/// One documented parameter of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parameter {
    /// The name a caller passes.
    pub name: &'static str,
    /// What it means, in one sentence, for the parameter documentation a UI shows.
    pub description: &'static str,
    /// Whether the command refuses to run without it.
    pub required: bool,
    /// What kind of value it takes.
    ///
    /// Declared rather than inferred, because a transport has to turn whatever a caller sent into
    /// the typed value a command expects. Inferring it from the parameter's name worked until the
    /// day a model sent `latitude: 51.5` and the command answered "expected an angle": a name is
    /// not a type, and the schema a model reads has to say which parameters are degrees.
    pub kind: ParameterKind,
}

/// The kind of value a parameter takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterKind {
    /// A plain number in the unit its description names.
    Number,
    /// Text.
    Text,
    /// An angle in degrees, converted to the typed [`Angle`] before the command sees it.
    Degrees,
    /// One or more `[latitude, longitude]` pairs in degrees, converted to typed sites.
    ///
    /// A target is a region, and its corners are all angles; carrying them as text made every
    /// transport parse a private mini-language and left the MCP schema unable to say what a corner
    /// is. The array form is the schema; the command line's `lat,lon;…` spelling is still accepted
    /// by [`args_from`] for the terminal.
    DegreesPair,
}

/// The static description of a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandDescriptor {
    /// The stable identifier, e.g. `satellite.add`. Never changes once released.
    pub id: &'static str,
    /// The label a menu shows.
    pub label: &'static str,
    /// Where a menu places it, e.g. `Satellite`.
    pub menu_path: &'static str,
    /// The default keyboard shortcut, if any.
    pub shortcut: Option<&'static str>,
    /// Every parameter, documented.
    pub parameters: &'static [Parameter],
}

/// A value passed to a command.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    /// A number.
    Number(f64),
    /// Free text.
    Text(String),
    /// An angle, already in its typed form so no unit can be guessed.
    Angle(Angle),
    /// Target corners, already in their typed form.
    Corners(Vec<GroundSite>),
    /// A duration.
    Duration(Duration),
}

impl Arg {
    /// Read this argument as a number.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] naming the argument when it is not a number.
    fn as_number(&self, name: &'static str) -> Result<f64> {
        match self {
            Self::Number(value) if value.is_finite() => Ok(*value),
            Self::Number(value) => Err(Error::out_of_range(name, value, "a finite number")),
            _ => Err(Error::invalid(name, "expected a number")),
        }
    }

    /// Read this argument as text.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] naming the argument when it is not text.
    fn as_text(&self, name: &'static str) -> Result<&str> {
        match self {
            Self::Text(value) => Ok(value),
            _ => Err(Error::invalid(name, "expected text")),
        }
    }

    /// Read this argument as an angle.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] naming the argument when it is not an angle.
    fn as_angle(&self, name: &'static str) -> Result<Angle> {
        match self {
            Self::Angle(value) => Ok(*value),
            _ => Err(Error::invalid(name, "expected an angle")),
        }
    }

    /// Read this argument as target corners.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] naming the argument when it is not a corner list.
    fn as_corners(&self, name: &'static str) -> Result<&[GroundSite]> {
        match self {
            Self::Corners(values) => Ok(values),
            _ => Err(Error::invalid(
                name,
                "expected `[latitude, longitude]` pairs in degrees",
            )),
        }
    }
}

/// The arguments of one command invocation, by parameter name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Args(BTreeMap<String, Arg>);

impl Args {
    /// No arguments.
    #[must_use]
    pub const fn none() -> Self {
        Self(BTreeMap::new())
    }

    /// Add an argument.
    #[must_use]
    pub fn with(mut self, name: &str, value: Arg) -> Self {
        self.0.insert(name.to_owned(), value);
        self
    }

    /// Look up an argument.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Arg> {
        self.0.get(name)
    }

    /// A required argument, or an error naming it.
    fn required(&self, name: &'static str) -> Result<&Arg> {
        self.get(name)
            .ok_or_else(|| Error::invalid(name, "is required"))
    }
}

/// What a command did, for a status line or an automated caller.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandOutcome {
    /// A one-line summary, suitable for a status bar or a log.
    pub summary: String,
    /// Access results, when the command computed any.
    pub access: Option<Vec<SatelliteAccess>>,
    /// Passes over a station on another body, when the command computed any.
    ///
    /// Separate from [`Self::access`] because the two answer different questions: `access` is
    /// per satellite/station pair on Earth with the window list a mission analyst expects, and
    /// this is one orbit about one body over one site. A summary that said only "4 passes" left
    /// the caller to recompute the times, which is exactly the work a tool call exists to do.
    pub passes: Option<Vec<PlanetPass>>,
    /// Access to the scenario's targets, when the command computed any.
    ///
    /// A target is a region, so its windows are the union of its vertices' — separate from [`Self::access`]
    /// because a pair of an orbit and a region is a different question from a pair of an orbit and a station.
    pub targets: Option<Vec<crate::analysis::TargetAccess>>,
    /// What the answer does not model, in sentences a caller must keep with the result.
    ///
    /// A summary that hides the sampling step, the vertex-union target model or the zero Earth
    /// orientation is a more confident answer than the computation supports.
    pub limitations: Vec<String>,
}

impl CommandOutcome {
    /// An outcome with only a summary.
    #[must_use]
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            access: None,
            passes: None,
            targets: None,
            limitations: Vec::new(),
        }
    }

    /// The same outcome with the limitations the answer must carry.
    #[must_use]
    pub fn with_limitations(mut self, limitations: Vec<String>) -> Self {
        self.limitations = limitations;
        self
    }

    /// The same outcome with target access attached.
    #[must_use]
    pub fn with_targets(mut self, targets: Vec<crate::analysis::TargetAccess>) -> Self {
        self.targets = Some(targets);
        self
    }

    /// The same outcome with passes attached.
    #[must_use]
    pub fn with_passes(mut self, passes: Vec<PlanetPass>) -> Self {
        self.passes = Some(passes);
        self
    }
}

/// A command: a description plus the two functions every transport needs.
pub struct Command {
    /// The static description.
    pub descriptor: CommandDescriptor,
    /// Whether the command can run in this scenario.
    pub enabled: fn(&Scenario) -> bool,
    /// Run it.
    pub run: fn(&mut Scenario, &Args) -> Result<CommandOutcome>,
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Command")
            .field("id", &self.descriptor.id)
            .finish_non_exhaustive()
    }
}

/// Every command the engine knows, by id.
#[derive(Debug, Default)]
pub struct Registry {
    commands: BTreeMap<&'static str, Command>,
}

impl Registry {
    /// The built-in command set.
    #[must_use]
    pub fn builtin() -> Self {
        let mut registry = Self::default();
        for (descriptor, enabled, run) in BUILTIN {
            registry.commands.insert(
                descriptor.id,
                Command {
                    descriptor,
                    enabled,
                    run,
                },
            );
        }
        registry
    }

    /// Every command, in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Command> {
        self.commands.values()
    }

    /// Find a command by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Command> {
        self.commands.get(id)
    }

    /// Whether a command could run right now.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] for an unknown id, so a caller can never mistake "no such
    /// command" for "disabled".
    pub fn enabled(&self, id: &str, scenario: &Scenario) -> Result<bool> {
        let command = self.get(id).ok_or_else(|| Error::NotFound {
            what: format!("command `{id}`"),
        })?;
        Ok((command.enabled)(scenario))
    }

    /// Run a command by id.
    ///
    /// This is the single dispatch path: a menu click, a shortcut, a CLI invocation and an
    /// agent all arrive here.
    ///
    /// # Errors
    ///
    /// - [`Error::NotFound`] for an unknown id.
    /// - [`Error::Unsupported`] if the command is disabled in this scenario.
    /// - Whatever the command's own validation returns.
    pub fn run(&self, id: &str, scenario: &mut Scenario, args: &Args) -> Result<CommandOutcome> {
        let command = self.get(id).ok_or_else(|| Error::NotFound {
            what: format!("command `{id}`"),
        })?;
        if !(command.enabled)(scenario) {
            return Err(Error::Unsupported {
                what: "this command in the current scenario",
            });
        }
        (command.run)(scenario, args)
    }
}

/// Parse `lat,lon;lat,lon;…` into vertices.
///
/// A text parameter rather than one parameter per corner, because a polygon has as many corners as the person
/// drew: repeating parameters would fix the count, and a second command just to append one would make a
/// triangle four calls.
///
/// # Errors
///
/// [`Error::Invalid`] naming the pair that failed: a coordinate that is not a number, or a place no latitude
/// or longitude reaches.
fn parse_vertices(points: &str) -> Result<Vec<GroundSite>> {
    let mut vertices = Vec::new();
    for (index, pair) in points
        .split(';')
        .filter(|pair| !pair.trim().is_empty())
        .enumerate()
    {
        let mut coordinates = pair.split(',');
        let latitude = coordinates
            .next()
            .map(str::trim)
            .ok_or_else(|| Error::invalid("points", format!("pair {index} has no latitude")))?;
        let longitude = coordinates
            .next()
            .map(str::trim)
            .ok_or_else(|| Error::invalid("points", format!("pair {index} has no longitude")))?;
        let latitude: f64 = latitude
            .parse()
            .map_err(|_| Error::invalid("points", format!("`{latitude}` is not a number")))?;
        let longitude: f64 = longitude
            .parse()
            .map_err(|_| Error::invalid("points", format!("`{longitude}` is not a number")))?;
        if vertices.len() >= crate::scenario::MAX_TARGET_VERTICES {
            return Err(Error::invalid(
                "points",
                format!(
                    "corner {} is past the maximum of {}",
                    index + 1,
                    crate::scenario::MAX_TARGET_VERTICES
                ),
            ));
        }
        vertices.push(GroundSite::new(
            Angle::from_deg(latitude),
            Angle::from_deg(longitude),
            0.0,
        )?);
    }
    if vertices.is_empty() {
        return Err(Error::invalid(
            "points",
            "a target needs at least one corner",
        ));
    }
    Ok(vertices)
}

/// Build a command's arguments from a JSON object, through the parameter kinds it declares.
///
/// Every transport calls this — the MCP server, the control channel, and the window's own socket —
/// so the schema a caller reads and the typed value a command receives cannot disagree. The kind is
/// the authority: a `Degrees` parameter becomes an [`Angle`] here, before any command sees it, which
/// is what stops a latitude arriving as a bare number and a command rejecting the right value.
///
/// A parameter the caller did not send is absent rather than zero, so a command's own default
/// applies. A number may arrive as a JSON number or as a string holding one: a caller occasionally
/// quotes it, and every command validates its range anyway.
#[must_use]
pub fn args_from(descriptor: &CommandDescriptor, values: &serde_json::Value) -> Args {
    let mut args = Args::none();
    for parameter in descriptor.parameters {
        let Some(value) = values.get(parameter.name) else {
            continue;
        };
        let argument = match parameter.kind {
            ParameterKind::Text => Arg::Text(match value {
                serde_json::Value::String(text) => text.clone(),
                other => other.to_string(),
            }),
            ParameterKind::Number => match number_from(value) {
                Some(number) => Arg::Number(number),
                None => Arg::Text(value.to_string()),
            },
            ParameterKind::Degrees => match number_from(value) {
                Some(degrees) => Arg::Angle(Angle::from_deg(degrees)),
                None => Arg::Text(value.to_string()),
            },
            ParameterKind::DegreesPair => match corners_from(value) {
                Some(corners) => Arg::Corners(corners),
                None => Arg::Text(value.to_string()),
            },
        };
        args = args.with(parameter.name, argument);
    }
    args
}

/// A number from a JSON number or from a string that holds one.
fn number_from(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

/// Target corners from the schema's array form or the command line's `lat,lon;…` form.
///
/// The bound is checked before the allocation: `items.len()` comes from the caller.
fn corners_from(value: &serde_json::Value) -> Option<Vec<GroundSite>> {
    match value {
        serde_json::Value::String(text) => parse_vertices(text).ok(),
        serde_json::Value::Array(items) => {
            if items.is_empty() || items.len() > crate::scenario::MAX_TARGET_VERTICES {
                return None;
            }
            let mut corners = Vec::with_capacity(items.len());
            for item in items {
                let pair = item.as_array()?;
                if pair.len() != 2 {
                    return None;
                }
                let latitude = number_from(pair.first()?)?;
                let longitude = number_from(pair.get(1)?)?;
                corners.push(
                    GroundSite::new(Angle::from_deg(latitude), Angle::from_deg(longitude), 0.0)
                        .ok()?,
                );
            }
            Some(corners)
        }
        _ => None,
    }
}

/// One built-in command: its description, its `enabled` predicate and its implementation.
type Builtin = (
    CommandDescriptor,
    fn(&Scenario) -> bool,
    fn(&mut Scenario, &Args) -> Result<CommandOutcome>,
);

/// What an Earth access answer does not model, in the sentences the answer carries.
fn access_limitations(step_seconds: f64, targets: bool) -> Vec<String> {
    let mut limitations = vec![
        format!(
            "Windows are sampled every {step_seconds} s; a window that opens and closes between \
             samples can be missed."
        ),
        "The line of sight is geometric: no atmospheric refraction, light time or aberration."
            .to_owned(),
        "Earth orientation is zero: UT1 - UTC and polar motion are not modelled, so ground \
         positions can be off by hundreds of metres."
            .to_owned(),
    ];
    if targets {
        limitations.push(
            "Target access is the union of its vertices' windows, not coverage of the polygon \
             interior."
                .to_owned(),
        );
    }
    limitations
}

/// What a planet-centred pass answer does not model.
fn planet_access_limitations(step_seconds: f64) -> Vec<String> {
    vec![
        format!(
            "Passes are sampled every {step_seconds} s; a pass that opens and closes between \
             samples can be missed."
        ),
        "The orbit is a circular Keplerian orbit about the body; no perturbations are modelled."
            .to_owned(),
        "The line of sight is geometric: no atmospheric refraction, light time or aberration."
            .to_owned(),
    ]
}

const ALWAYS: fn(&Scenario) -> bool = |_| true;

const BUILTIN: [Builtin; 11] = [
    (
        CommandDescriptor {
            id: "scenario.rename",
            label: "Rename Scenario",
            menu_path: "Scenario",
            shortcut: None,
            parameters: &[Parameter {
                name: "name",
                kind: ParameterKind::Text,
                description: "The new scenario name.",
                required: true,
            }],
        },
        ALWAYS,
        |scenario, args| {
            let name = args.required("name")?.as_text("name")?.to_owned();
            scenario.name.clone_from(&name);
            Ok(CommandOutcome::new(format!("Scenario renamed to `{name}`")))
        },
    ),
    (
        CommandDescriptor {
            id: "scenario.set-duration",
            label: "Set Timeline Duration",
            menu_path: "Scenario",
            shortcut: None,
            parameters: &[Parameter {
                name: "seconds",
                kind: ParameterKind::Number,
                description: "How long the timeline runs, in seconds. Must be positive.",
                required: true,
            }],
        },
        ALWAYS,
        |scenario, args| {
            let seconds = args.required("seconds")?.as_number("seconds")?;
            if seconds <= 0.0 {
                return Err(Error::out_of_range(
                    "seconds",
                    seconds,
                    "strictly positive seconds",
                ));
            }
            scenario.duration_seconds = seconds;
            Ok(CommandOutcome::new(format!("Timeline set to {seconds} s")))
        },
    ),
    (
        CommandDescriptor {
            id: "satellite.add",
            label: "Add Satellite",
            menu_path: "Satellite",
            shortcut: None,
            parameters: &[
                Parameter {
                    name: "name",
                    kind: ParameterKind::Text,
                    description: "A display name; need not be unique.",
                    required: true,
                },
                Parameter {
                    name: "line1",
                    kind: ParameterKind::Text,
                    description: "The first line of the two-line element set.",
                    required: true,
                },
                Parameter {
                    name: "line2",
                    kind: ParameterKind::Text,
                    description: "The second line of the two-line element set.",
                    required: true,
                },
            ],
        },
        ALWAYS,
        |scenario, args| {
            let name = args.required("name")?.as_text("name")?.to_owned();
            let line1 = args.required("line1")?.as_text("line1")?;
            let line2 = args.required("line2")?.as_text("line2")?;
            let element_set = TwoLineElement::parse(line1, line2)?;
            let id = scenario.add_satellite(name.clone(), element_set, [255, 255, 255])?;
            Ok(CommandOutcome::new(format!(
                "Added satellite `{name}` as {id:?}"
            )))
        },
    ),
    (
        CommandDescriptor {
            id: "satellite.remove",
            label: "Remove Satellite",
            menu_path: "Satellite",
            shortcut: None,
            parameters: &[Parameter {
                name: "id",
                kind: ParameterKind::Number,
                description: "The numeric id of the satellite to remove.",
                required: true,
            }],
        },
        ALWAYS,
        |scenario, args| {
            let id = args.required("id")?.as_number("id")?;
            let id = crate::scenario::SatelliteId(
                u32::try_from(id as i64).map_err(|_| Error::invalid("id", "out of range"))?,
            );
            if scenario.remove_satellite(id) {
                Ok(CommandOutcome::new(format!("Removed satellite {id:?}")))
            } else {
                Err(Error::NotFound {
                    what: format!("satellite {id:?}"),
                })
            }
        },
    ),
    (
        CommandDescriptor {
            id: "station.add",
            label: "Add Ground Station",
            menu_path: "Station",
            shortcut: None,
            parameters: &[
                Parameter {
                    name: "name",
                    kind: ParameterKind::Text,
                    description: "A display name; need not be unique.",
                    required: true,
                },
                Parameter {
                    name: "latitude",
                    kind: ParameterKind::Degrees,
                    description: "Geodetic latitude, positive north.",
                    required: true,
                },
                Parameter {
                    name: "longitude",
                    kind: ParameterKind::Degrees,
                    description: "Longitude, positive east.",
                    required: true,
                },
                Parameter {
                    name: "minimum-elevation",
                    kind: ParameterKind::Degrees,
                    description: "The horizon mask; nothing below it is visible.",
                    required: true,
                },
            ],
        },
        ALWAYS,
        |scenario, args| {
            let name = args.required("name")?.as_text("name")?.to_owned();
            let latitude = args.required("latitude")?.as_angle("latitude")?;
            let longitude = args.required("longitude")?.as_angle("longitude")?;
            let mask = args
                .required("minimum-elevation")?
                .as_angle("minimum-elevation")?;
            let site = stkcraft_astro_access::GroundSite::new(latitude, longitude, 0.0)?;
            let sensor = Sensor::above_horizon(mask);
            let id = scenario.add_station(name.clone(), site, sensor)?;
            Ok(CommandOutcome::new(format!(
                "Added station `{name}` as {id:?}"
            )))
        },
    ),
    (
        CommandDescriptor {
            id: "station.remove",
            label: "Remove Ground Station",
            menu_path: "Station",
            shortcut: None,
            parameters: &[Parameter {
                name: "id",
                kind: ParameterKind::Number,
                description: "The numeric id of the station to remove.",
                required: true,
            }],
        },
        ALWAYS,
        |scenario, args| {
            let id = args.required("id")?.as_number("id")?;
            let id = StationId(
                u32::try_from(id as i64).map_err(|_| Error::invalid("id", "out of range"))?,
            );
            if scenario.remove_station(id) {
                Ok(CommandOutcome::new(format!("Removed station {id:?}")))
            } else {
                Err(Error::NotFound {
                    what: format!("station {id:?}"),
                })
            }
        },
    ),
    (
        CommandDescriptor {
            id: "station.sensor",
            label: "Set Station Sensor",
            menu_path: "Station",
            shortcut: None,
            parameters: &[
                Parameter {
                    name: "id",
                    kind: ParameterKind::Number,
                    description: "The numeric id of the station.",
                    required: true,
                },
                Parameter {
                    name: "half-angle",
                    kind: ParameterKind::Degrees,
                    description: "Half-angle of the field-of-view cone about the boresight, \
                                  in degrees. 180 is the whole upper hemisphere.",
                    required: true,
                },
                Parameter {
                    name: "max-range-km",
                    kind: ParameterKind::Number,
                    description: "Maximum slant range in kilometres; omit for unlimited.",
                    required: false,
                },
                Parameter {
                    name: "azimuth",
                    kind: ParameterKind::Degrees,
                    description: "Boresight azimuth, clockwise from north. Default 0.",
                    required: false,
                },
                Parameter {
                    name: "elevation",
                    kind: ParameterKind::Degrees,
                    description: "Boresight elevation above the horizon. Default 90 (zenith).",
                    required: false,
                },
            ],
        },
        ALWAYS,
        |scenario, args| {
            let id = args.required("id")?.as_number("id")?;
            let id = StationId(
                u32::try_from(id as i64).map_err(|_| Error::invalid("id", "out of range"))?,
            );
            let half_angle = args.required("half-angle")?.as_angle("half-angle")?;
            if half_angle.as_deg() <= 0.0 || half_angle.as_deg() > 180.0 {
                return Err(Error::invalid(
                    "half-angle",
                    "a field-of-view half-angle is greater than 0 and at most 180 degrees",
                ));
            }
            let azimuth = args
                .get("azimuth")
                .map(|argument| argument.as_angle("azimuth"))
                .transpose()?
                .unwrap_or(Angle::ZERO);
            let elevation = args
                .get("elevation")
                .map(|argument| argument.as_angle("elevation"))
                .transpose()?
                .unwrap_or(Angle::from_deg(90.0));
            let boresight = (azimuth, elevation);
            let maximum_range = args
                .get("max-range-km")
                .map(|argument| argument.as_number("max-range-km"))
                .transpose()?
                .map(Distance::from_km);
            let sensor = scenario
                .station_sensor_mut(id)
                .ok_or_else(|| Error::NotFound {
                    what: format!("station {id:?}"),
                })?;
            *sensor = Sensor {
                minimum_elevation: sensor.minimum_elevation,
                half_angle,
                boresight,
                maximum_range,
            };
            Ok(CommandOutcome::new(format!(
                "Station {id:?} now has a {} degree cone{}",
                half_angle.as_deg(),
                maximum_range.map_or_else(String::new, |range| format!(
                    ", maximum range {} km",
                    range.as_km()
                ))
            )))
        },
    ),
    (
        CommandDescriptor {
            id: "target.add",
            label: "Add Target",
            menu_path: "Target",
            shortcut: None,
            parameters: &[
                Parameter {
                    name: "name",
                    kind: ParameterKind::Text,
                    description: "A display name; need not be unique.",
                    required: true,
                },
                Parameter {
                    name: "points",
                    kind: ParameterKind::DegreesPair,
                    description: "The target's corners as `[latitude, longitude]` pairs in degrees; \
                                  the command line also accepts `lat,lon;…`, e.g. `40.4,-3.7` or \
                                  `0,0;0,1;1,1`.",
                    required: true,
                },
                Parameter {
                    name: "half-angle",
                    kind: ParameterKind::Degrees,
                    description: "Half-angle of the sensor's field-of-view cone. Default 180.",
                    required: false,
                },
                Parameter {
                    name: "minimum-elevation",
                    kind: ParameterKind::Degrees,
                    description: "The observer's horizon mask. Default 0.",
                    required: false,
                },
            ],
        },
        ALWAYS,
        |scenario, args| {
            let name = args.required("name")?.as_text("name")?.to_owned();
            let vertices = args.required("points")?.as_corners("points")?.to_vec();
            let half_angle = args
                .get("half-angle")
                .map(|argument| argument.as_angle("half-angle"))
                .transpose()?
                .unwrap_or(Angle::from_deg(180.0));
            let mask = args
                .get("minimum-elevation")
                .map(|argument| argument.as_angle("minimum-elevation"))
                .transpose()?
                .unwrap_or(Angle::ZERO);
            let mut sensor = Sensor::above_horizon(mask);
            sensor.half_angle = half_angle;
            let id = scenario.add_target(name.clone(), vertices.clone(), sensor)?;
            Ok(CommandOutcome::new(format!(
                "Added target `{name}` as {id:?} with {} vertex/vertices",
                vertices.len()
            )))
        },
    ),
    (
        CommandDescriptor {
            id: "access.targets",
            label: "Compute Target Access",
            menu_path: "Access",
            shortcut: None,
            parameters: &[Parameter {
                name: "step-seconds",
                kind: ParameterKind::Number,
                description: "Search step. Shorter is more thorough and slower.",
                required: false,
            }],
        },
        |scenario| !scenario.satellites.is_empty() && !scenario.targets.is_empty(),
        |scenario, args| {
            let step = match args.get("step-seconds") {
                Some(argument) => argument.as_number("step-seconds")?,
                None => stkcraft_astro_access::DEFAULT_STEP.as_seconds(),
            };
            if step <= 0.0 {
                return Err(Error::out_of_range(
                    "step-seconds",
                    step,
                    "strictly positive seconds",
                ));
            }
            let step = Duration::from_seconds(step);
            let mut results = Vec::new();
            for satellite in &scenario.satellites {
                for target in &scenario.targets {
                    results.push(crate::analysis::compute_target_access(
                        scenario,
                        satellite.id,
                        &target.name,
                        &target.vertices,
                        &target.sensor,
                        step,
                    )?);
                }
            }
            let windows: usize = results.iter().map(|result| result.windows.len()).sum();
            let summary = format!(
                "Computed target access: {} pair(s), {windows} window(s)",
                results.len()
            );
            Ok(CommandOutcome::new(summary)
                .with_targets(results)
                .with_limitations(access_limitations(step.as_seconds(), true)))
        },
    ),
    (
        CommandDescriptor {
            id: "access.compute",
            label: "Compute Access",
            menu_path: "Access",
            shortcut: Some("Ctrl+Shift+A"),
            parameters: &[Parameter {
                name: "step-seconds",
                kind: ParameterKind::Number,
                description: "Search step. Shorter is more thorough and slower.",
                required: false,
            }],
        },
        |scenario| !scenario.satellites.is_empty() && !scenario.stations.is_empty(),
        |scenario, args| {
            let step = match args.get("step-seconds") {
                Some(argument) => argument.as_number("step-seconds")?,
                // The default is the access crate's own: one minute.
                None => stkcraft_astro_access::DEFAULT_STEP.as_seconds(),
            };
            if step <= 0.0 {
                return Err(Error::out_of_range(
                    "step-seconds",
                    step,
                    "strictly positive seconds",
                ));
            }
            let results = compute_all_access(scenario, Duration::from_seconds(step))?;
            let windows: usize = results.iter().map(|r| r.windows.len()).sum();
            Ok(CommandOutcome {
                summary: format!(
                    "Computed access: {} pair(s), {windows} window(s)",
                    results.len()
                ),
                access: Some(results),
                passes: None,
                targets: None,
                limitations: access_limitations(step, false),
            })
        },
    ),
    (
        CommandDescriptor {
            id: "planet.access",
            label: "Planet-Centred Access",
            menu_path: "Access",
            shortcut: None,
            parameters: &[
                Parameter {
                    name: "body",
                    kind: ParameterKind::Text,
                    description: "The body the station stands on and the orbit is about, by IAU name (e.g. Mars, Titan).",
                    required: true,
                },
                Parameter {
                    name: "radius-km",
                    kind: ParameterKind::Number,
                    description: "Circular orbit radius from the body's centre, km.",
                    required: true,
                },
                Parameter {
                    name: "inclination-deg",
                    kind: ParameterKind::Degrees,
                    description: "Orbit inclination from the body's ICRF equator, degrees.",
                    required: true,
                },
                Parameter {
                    name: "latitude-deg",
                    kind: ParameterKind::Degrees,
                    description: "Station planetodetic latitude on that body, degrees.",
                    required: true,
                },
                Parameter {
                    name: "longitude-deg",
                    kind: ParameterKind::Degrees,
                    description: "Station longitude east of the body's prime meridian, degrees.",
                    required: true,
                },
                Parameter {
                    name: "mask-deg",
                    kind: ParameterKind::Degrees,
                    description: "Sensor horizon mask, degrees above the local horizon.",
                    required: false,
                },
                Parameter {
                    name: "hours",
                    kind: ParameterKind::Number,
                    description: "How long to search, hours.",
                    required: false,
                },
                Parameter {
                    name: "step-seconds",
                    kind: ParameterKind::Number,
                    description: "Search step: shorter is more thorough and slower.",
                    required: false,
                },
            ],
        },
        ALWAYS,
        |scenario, args| {
            let body_name = args
                .get("body")
                .ok_or_else(|| Error::invalid("body", "required: the IAU name of a body"))?
                .as_text("body")?;
            let body = stkcraft_astro_body::Body::ALL
                .into_iter()
                .find(|candidate| candidate.name().eq_ignore_ascii_case(body_name))
                .ok_or_else(|| Error::NotFound {
                    what: format!("a body named `{body_name}`"),
                })?;
            let radius_km = args
                .get("radius-km")
                .ok_or_else(|| Error::invalid("radius-km", "required"))?
                .as_number("radius-km")?;
            let inclination = args
                .get("inclination-deg")
                .ok_or_else(|| Error::invalid("inclination-deg", "required"))?
                .as_angle("inclination-deg")?;
            let latitude = args
                .get("latitude-deg")
                .ok_or_else(|| Error::invalid("latitude-deg", "required"))?
                .as_angle("latitude-deg")?;
            let longitude = args
                .get("longitude-deg")
                .ok_or_else(|| Error::invalid("longitude-deg", "required"))?
                .as_angle("longitude-deg")?;
            let mask = match args.get("mask-deg") {
                Some(argument) => argument.as_angle("mask-deg")?,
                None => Angle::ZERO,
            };
            let hours = match args.get("hours") {
                Some(argument) => argument.as_number("hours")?,
                None => 24.0,
            };
            let step_seconds = match args.get("step-seconds") {
                Some(argument) => argument.as_number("step-seconds")?,
                None => 30.0,
            };

            // The body's rotation model takes TDB, and the scenario's epoch is on whatever scale
            // its document used; converting once here keeps the conversion out of the sampler.
            let epoch_tdb = scenario
                .epoch
                .to_scale(stkcraft_framework_time::TimeScale::Tdb, None)?;
            let orbit = PlanetOrbit::circular(body, radius_km, inclination.as_rad(), epoch_tdb)?;
            let access = stkcraft_astro_access::PlanetAccess {
                body,
                site: stkcraft_astro_access::GroundSite::new(latitude, longitude, 0.0)?,
                sensor: Sensor::above_horizon(mask),
            };
            let passes = find_passes(&orbit, &access, epoch_tdb, hours, step_seconds)?;
            let total: f64 = passes
                .iter()
                .map(crate::planet_access::PlanetPass::duration_seconds)
                .sum();
            Ok(CommandOutcome::new(format!(
                "{body}: {} pass(es) in {hours:.1} h, {} min in view",
                passes.len(),
                (total / 60.0).round() as i64
            ))
            .with_passes(passes)
            .with_limitations(planet_access_limitations(step_seconds)))
        },
    ),
];

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    const LEO_L1: &str = "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9009";
    const LEO_L2: &str = "2 25544  51.6400 208.9163 0006317  69.9862  25.2906 15.49569600 12343";

    fn add_satellite_args() -> Args {
        Args::none()
            .with("name", Arg::Text("ISS".to_owned()))
            .with("line1", Arg::Text(LEO_L1.to_owned()))
            .with("line2", Arg::Text(LEO_L2.to_owned()))
    }

    #[test]
    fn every_command_id_is_unique_and_dotted() {
        let registry = Registry::builtin();
        let mut seen = std::collections::BTreeSet::new();
        for command in registry.iter() {
            let id = command.descriptor.id;
            assert!(seen.insert(id), "duplicate command id {id}");
            assert!(
                id.contains('.'),
                "command id `{id}` is not `namespace.verb` shaped"
            );
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '.' || c == '-'),
                "command id `{id}` has characters a caller has to quote"
            );
        }
        assert_eq!(seen.len(), BUILTIN.len());
    }

    #[test]
    fn every_command_has_a_label_a_menu_path_and_documented_parameters() {
        let registry = Registry::builtin();
        for command in registry.iter() {
            let d = &command.descriptor;
            assert!(!d.label.is_empty(), "{} has no label", d.id);
            assert!(!d.menu_path.is_empty(), "{} has no menu path", d.id);
            for parameter in d.parameters {
                assert!(
                    !parameter.name.is_empty(),
                    "{} has an unnamed parameter",
                    d.id
                );
                assert!(
                    parameter.description.len() > 10,
                    "{}::{} has no real description",
                    d.id,
                    parameter.name
                );
            }
        }
    }

    #[test]
    fn commands_dispatch_by_id() {
        // The whole point: run through the registry by string id, exactly as a menu item or an
        // automated caller would, and assert the scenario actually changed.
        let registry = Registry::builtin();
        let mut scenario = Scenario::empty("test");

        let outcome = registry
            .run("satellite.add", &mut scenario, &add_satellite_args())
            .expect("adds");
        assert!(outcome.summary.contains("ISS"));
        assert_eq!(scenario.satellites.len(), 1);

        let outcome = registry
            .run(
                "station.add",
                &mut scenario,
                &Args::none()
                    .with("name", Arg::Text("Equator".to_owned()))
                    .with("latitude", Arg::Angle(Angle::from_deg(0.0)))
                    .with("longitude", Arg::Angle(Angle::from_deg(0.0)))
                    .with("minimum-elevation", Arg::Angle(Angle::from_deg(0.0))),
            )
            .expect("adds");
        assert!(outcome.summary.contains("Equator"));
        assert_eq!(scenario.stations.len(), 1);
    }

    #[test]
    fn an_unknown_command_is_not_found_rather_than_disabled() {
        // The distinction matters: "no such command" and "this command cannot run now" are
        // different errors and a caller must be able to tell them apart.
        let registry = Registry::builtin();
        let mut scenario = Scenario::empty("test");
        let error = registry
            .run("no.such.command", &mut scenario, &Args::none())
            .expect_err("must fail");
        assert!(matches!(error, Error::NotFound { .. }), "got {error:?}");

        let error = registry
            .enabled("no.such.command", &scenario)
            .expect_err("must fail");
        assert!(matches!(error, Error::NotFound { .. }), "got {error:?}");
    }

    #[test]
    fn a_disabled_command_reports_unsupported() {
        // `access.compute` needs at least one satellite and one station.
        let registry = Registry::builtin();
        let mut scenario = Scenario::empty("test");
        assert_eq!(registry.enabled("access.compute", &scenario), Ok(false));
        let error = registry
            .run("access.compute", &mut scenario, &Args::none())
            .expect_err("must fail");
        assert!(matches!(error, Error::Unsupported { .. }), "got {error:?}");
    }

    #[test]
    fn every_command_runs_without_reading_anything_but_its_arguments() {
        // The guarantee behind "programmatic calls never open a dialog": each command must be a
        // pure function of (scenario, arguments). This checks the mechanical part of that --
        // every command is runnable from a cold scenario with only explicit arguments, and
        // never blocks or reads ambient state.
        //
        // Commands whose inputs are real data (a TLE, an id that must exist) are exercised by
        // their own tests; here the property is that the registry has an entry that can be
        // called by id at all, and that calling it with nothing yields a typed error rather
        // than a hang or a panic.
        let registry = Registry::builtin();
        let mut scenario = Scenario::empty("automation");

        for command in registry.iter() {
            let id = command.descriptor.id;
            let outcome = registry.run(id, &mut scenario, &Args::none());
            // Either it succeeded, or it failed with one of our typed errors. Never a panic.
            match outcome {
                Ok(_) => {}
                Err(error) => {
                    assert!(
                        matches!(
                            error,
                            Error::Invalid { .. }
                                | Error::NotFound { .. }
                                | Error::OutOfRange { .. }
                                | Error::Unsupported { .. }
                        ),
                        "`{id}` failed with an unexpected error: {error:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_missing_required_argument_is_named() {
        let registry = Registry::builtin();
        let mut scenario = Scenario::empty("test");
        let error = registry
            .run(
                "satellite.add",
                &mut scenario,
                &Args::none().with("name", Arg::Text("ISS".to_owned())),
            )
            .expect_err("must fail");
        let text = error.to_string();
        assert!(text.contains("line1"), "error did not name line1: {text}");
    }

    #[test]
    fn an_argument_of_the_wrong_type_is_refused() {
        let registry = Registry::builtin();
        let mut scenario = Scenario::empty("test");
        let error = registry
            .run(
                "station.add",
                &mut scenario,
                &Args::none()
                    .with("name", Arg::Text("E".to_owned()))
                    // A number where an angle belongs: degrees and radians must not be guessed.
                    .with("latitude", Arg::Number(0.0))
                    .with("longitude", Arg::Angle(Angle::from_deg(0.0)))
                    .with("minimum-elevation", Arg::Angle(Angle::from_deg(0.0))),
            )
            .expect_err("must fail");
        assert!(error.to_string().contains("latitude"), "got {error}");
    }

    #[test]
    fn an_invalid_element_set_is_refused_by_the_command() {
        let registry = Registry::builtin();
        let mut scenario = Scenario::empty("test");
        let error = registry
            .run(
                "satellite.add",
                &mut scenario,
                &Args::none()
                    .with("name", Arg::Text("BROKEN".to_owned()))
                    .with("line1", Arg::Text(LEO_L1.to_owned()))
                    .with("line2", Arg::Text("2 not a tle line".to_owned())),
            )
            .expect_err("must fail");
        assert!(
            error.to_string().contains("TLE"),
            "error did not mention the TLE: {error}"
        );
        assert_eq!(scenario.satellites.len(), 0, "a broken TLE got added");
    }

    #[test]
    fn access_compute_runs_end_to_end_through_the_registry() {
        let registry = Registry::builtin();
        let mut scenario = Scenario::empty("automation");
        scenario.epoch = TwoLineElement::parse(LEO_L1, LEO_L2)
            .expect("valid TLE")
            .epoch;
        scenario.duration_seconds = 3_600.0;

        registry
            .run("satellite.add", &mut scenario, &add_satellite_args())
            .expect("adds satellite");
        registry
            .run(
                "station.add",
                &mut scenario,
                &Args::none()
                    .with("name", Arg::Text("Equator".to_owned()))
                    .with("latitude", Arg::Angle(Angle::from_deg(0.0)))
                    .with("longitude", Arg::Angle(Angle::from_deg(0.0)))
                    .with("minimum-elevation", Arg::Angle(Angle::from_deg(0.0))),
            )
            .expect("adds station");

        assert_eq!(registry.enabled("access.compute", &scenario), Ok(true));
        let outcome = registry
            .run(
                "access.compute",
                &mut scenario,
                &Args::none().with("step-seconds", Arg::Number(30.0)),
            )
            .expect("computes");
        assert!(
            outcome
                .limitations
                .iter()
                .any(|line| line.contains("sampled every 30 s")),
            "{:?}",
            outcome.limitations
        );
        assert!(
            outcome
                .limitations
                .iter()
                .any(|line| line.contains("Earth orientation is zero")),
            "{:?}",
            outcome.limitations
        );
        let access = outcome.access.expect("access results");
        assert_eq!(access.len(), 1);
    }

    #[test]
    fn removing_an_object_that_is_not_there_is_not_found() {
        let registry = Registry::builtin();
        let mut scenario = Scenario::empty("test");
        let error = registry
            .run(
                "satellite.remove",
                &mut scenario,
                &Args::none().with("id", Arg::Number(42.0)),
            )
            .expect_err("must fail");
        assert!(matches!(error, Error::NotFound { .. }), "got {error:?}");
    }
    #[test]
    fn a_planet_centred_access_run_goes_through_the_registry() {
        // The property `AGENTS.md` asks for: a user-visible capability is a command, so an agent
        // and a menu click take the same path. Mars, a 4 000 km orbit, a station on its equator.
        let registry = Registry::builtin();
        let mut scenario = Scenario::empty("Mars");
        let outcome = registry
            .run(
                "planet.access",
                &mut scenario,
                &Args::none()
                    .with("body", Arg::Text("Mars".to_owned()))
                    .with("radius-km", Arg::Number(4_000.0))
                    // Angles are typed: a plain number here is the mix-up the type exists to
                    // prevent, and the command refuses it rather than guessing the unit.
                    .with("inclination-deg", Arg::Angle(Angle::from_deg(0.0)))
                    .with("latitude-deg", Arg::Angle(Angle::from_deg(0.0)))
                    .with("longitude-deg", Arg::Angle(Angle::from_deg(0.0)))
                    .with("mask-deg", Arg::Angle(Angle::from_deg(10.0)))
                    .with("hours", Arg::Number(6.0))
                    .with("step-seconds", Arg::Number(10.0)),
            )
            .expect("the command runs");
        assert!(
            outcome.summary.contains("pass(es)"),
            "the summary does not report passes: {}",
            outcome.summary
        );
        assert!(outcome.summary.starts_with("Mars:"), "{}", outcome.summary);
        assert!(
            outcome
                .limitations
                .iter()
                .any(|line| line.contains("circular Keplerian orbit")),
            "{:?}",
            outcome.limitations
        );
    }

    #[test]
    fn an_unknown_body_name_is_refused_by_name() {
        let registry = Registry::builtin();
        let mut scenario = Scenario::empty("nowhere");
        let error = registry
            .run(
                "planet.access",
                &mut scenario,
                &Args::none()
                    .with("body", Arg::Text("Vulcan".to_owned()))
                    .with("radius-km", Arg::Number(4_000.0))
                    .with("inclination-deg", Arg::Angle(Angle::from_deg(0.0)))
                    .with("latitude-deg", Arg::Angle(Angle::from_deg(0.0)))
                    .with("longitude-deg", Arg::Angle(Angle::from_deg(0.0))),
            )
            .expect_err("Vulcan is not a body");
        assert!(error.to_string().contains("Vulcan"), "{error}");
    }

    /// A near-equatorial orbit whose ascending node sits on the epoch's prime meridian, so the satellite is
    /// overhead of `(0°, 0°)` at the epoch: the station below it must see it, and a cone narrow enough
    /// cannot contain it.
    const OVERHEAD_L1: &str =
        "1 90000U 24001A   24001.50000000  .00000000  00000-0  00000-0 0  9000";
    const OVERHEAD_L2: &str =
        "2 90000   0.0100   0.0000 0001000   0.0000   0.0000 15.50000000 00004";

    #[test]
    fn narrowing_a_stations_cone_removes_the_windows_it_cannot_see() {
        // The hand-computable case F5 asks for: the same orbit and the same station, with a sensor that sees
        // the whole sky and then with a cone a hundredth of a degree wide. The satellite passes over the
        // station and cannot be inside a 0.01-degree cone, so the windows must vanish — a sensor that is
        // wired in but ignored would leave them.
        let registry = Registry::builtin();
        let mut scenario = Scenario::empty("sensor");
        scenario.epoch = TwoLineElement::parse(OVERHEAD_L1, OVERHEAD_L2)
            .expect("valid TLE")
            .epoch;
        // A day: a low Earth orbit crosses the equator several times, which is what gives the wide sensor
        // something to see before the cone is narrowed.
        scenario.duration_seconds = 86_400.0;
        registry
            .run(
                "satellite.add",
                &mut scenario,
                &Args::none()
                    .with("name", Arg::Text("Overhead".to_owned()))
                    .with("line1", Arg::Text(OVERHEAD_L1.to_owned()))
                    .with("line2", Arg::Text(OVERHEAD_L2.to_owned())),
            )
            .expect("adds satellite");
        registry
            .run(
                "station.add",
                &mut scenario,
                &Args::none()
                    .with("name", Arg::Text("Equator".to_owned()))
                    .with("latitude", Arg::Angle(Angle::from_deg(0.0)))
                    .with("longitude", Arg::Angle(Angle::from_deg(0.0)))
                    .with("minimum-elevation", Arg::Angle(Angle::from_deg(0.0))),
            )
            .expect("adds station");
        let station = scenario.stations.first().expect("the station was added").id;
        let compute = |scenario: &mut Scenario| {
            registry
                .run(
                    "access.compute",
                    scenario,
                    &Args::none().with("step-seconds", Arg::Number(30.0)),
                )
                .expect("computes")
                .access
                .unwrap_or_default()
                .iter()
                .map(|entry| entry.windows.len())
                .sum::<usize>()
        };
        let wide = compute(&mut scenario);
        assert!(
            wide > 0,
            "the station directly under the satellite saw nothing"
        );

        // A cone 0.01 degrees wide about the zenith: nothing can be inside it by accident.
        registry
            .run(
                "station.sensor",
                &mut scenario,
                &Args::none()
                    .with("id", Arg::Number(f64::from(station.0)))
                    .with("half-angle", Arg::Angle(Angle::from_deg(0.01))),
            )
            .expect("sets the cone");
        let narrow = compute(&mut scenario);
        assert_eq!(narrow, 0, "{narrow} window(s) survived a 0.01 degree cone");

        // And a cone that is invalid is refused rather than stored: a half-angle of 0 or 200 degrees is not
        // a sensor, and a command that accepted it would fail later as a missing window.
        for bad in [0.0, 200.0] {
            let answer = registry.run(
                "station.sensor",
                &mut scenario,
                &Args::none()
                    .with("id", Arg::Number(f64::from(station.0)))
                    .with("half-angle", Arg::Angle(Angle::from_deg(bad))),
            );
            assert!(answer.is_err(), "{bad} degrees was accepted");
        }
    }

    #[test]
    fn target_corners_are_typed_from_both_forms() {
        let registry = Registry::builtin();
        let command = registry.get("target.add").expect("target.add");
        let array = args_from(
            &command.descriptor,
            &serde_json::json!({"name": "x", "points": [[51.5, -0.13], [51.6, -0.2]]}),
        );
        assert!(matches!(
            array.get("points"),
            Some(Arg::Corners(corners)) if corners.len() == 2
        ));
        let text = args_from(
            &command.descriptor,
            &serde_json::json!({"name": "x", "points": "51.5,-0.13"}),
        );
        assert!(matches!(
            text.get("points"),
            Some(Arg::Corners(corners)) if corners.len() == 1
        ));
    }

    #[test]
    fn a_target_is_added_from_points_and_its_windows_are_the_union_of_its_vertices() {
        // The command-level version of the geometry test: a two-vertex target must see exactly what the two
        // point targets see together, merged — so "the region is visible while any corner is" holds through
        // the registry and not only inside `analysis`.
        let registry = Registry::builtin();
        let mut scenario = Scenario::empty("targets");
        scenario.epoch = TwoLineElement::parse(LEO_L1, LEO_L2)
            .expect("valid TLE")
            .epoch;
        scenario.duration_seconds = 86_400.0;
        registry
            .run("satellite.add", &mut scenario, &add_satellite_args())
            .expect("adds satellite");

        for (name, points) in [
            ("west", "0,-0.5"),
            ("east", "0,0.5"),
            ("both", "0,-0.5;0,0.5"),
        ] {
            registry
                .run(
                    "target.add",
                    &mut scenario,
                    &Args::none().with("name", Arg::Text(name.to_owned())).with(
                        "points",
                        Arg::Corners(parse_vertices(points).expect("the corners")),
                    ),
                )
                .expect("adds target");
        }
        assert_eq!(scenario.targets.len(), 3);
        assert_eq!(
            scenario.targets.last().expect("the polygon").vertices.len(),
            2
        );

        let outcome = registry
            .run(
                "access.targets",
                &mut scenario,
                &Args::none().with("step-seconds", Arg::Number(30.0)),
            )
            .expect("computes");
        assert!(
            outcome
                .limitations
                .iter()
                .any(|line| line.contains("union of its vertices")),
            "{:?}",
            outcome.limitations
        );
        let targets = outcome.targets.expect("target access");
        assert_eq!(targets.len(), 3, "one per satellite/target pair");
        let find = |name: &str| {
            targets
                .iter()
                .find(|entry| entry.target == name)
                .expect("the target")
        };
        let west = find("west").windows.clone();
        let east = find("east").windows.clone();
        let both = find("both").windows.clone();
        // The region's windows are exactly the union of its corners' — computed the same way here, from the
        // two point targets the command just produced.
        let expected = crate::analysis::union(&[west.clone(), east.clone()]);
        assert_eq!(
            both, expected,
            "the region's windows are not the union of its corners'"
        );
        // And the union cannot be shorter than either corner.
        assert!(
            both.len() <= west.len() + east.len(),
            "the union did not merge anything at all"
        );
        assert!(
            both.len() >= west.len().max(east.len()),
            "the union lost a window"
        );

        // A target with no points is refused, and so is one with a bad coordinate.
        for points in ["", "  ", "40.4", "north,-3.7"] {
            let answer = registry.run(
                "target.add",
                &mut scenario,
                &Args::none()
                    .with("name", Arg::Text("bad".to_owned()))
                    .with("points", Arg::Text(points.to_owned())),
            );
            assert!(answer.is_err(), "`{points}` was accepted as a target");
        }
    }
}
