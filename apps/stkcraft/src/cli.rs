//! The command line: argument parsing and dispatch into the engine's command registry.
//!
//! Hand-written rather than generated, because the surface is small and a dependency that
//! cannot be fetched offline would block the build. The shape is deliberate: every subcommand
//! builds a [`Args`] and calls [`Registry::run`] by id, exactly as a menu item or a control
//! channel will. Nothing here reimplements engine behaviour.

use std::fmt::Write as _;

use stkcraft_core::Angle;
use stkcraft_engine::{Arg, Args, Registry, Scenario};
use stkcraft_sgp4::TwoLineElement;

use crate::render;

/// What the CLI can be asked to do.
pub enum Invocation {
    Help,
    Version,
    /// One satellite against one station, printed as a pass table.
    Passes(PassRequest),
}

/// The parameters of the `passes` subcommand.
pub struct PassRequest {
    line1: String,
    line2: String,
    latitude_deg: f64,
    longitude_deg: f64,
    minimum_elevation_deg: f64,
    hours: f64,
    step_seconds: f64,
    name: String,
}

/// Parse arguments and run.
///
/// # Errors
///
/// Returns a message for the user; the caller prints it and exits non-zero.
pub fn run<I: Iterator<Item = String>>(arguments: I) -> Result<(), String> {
    match parse(arguments)? {
        Invocation::Help => {
            print!("{}", usage());
            Ok(())
        }
        Invocation::Version => {
            println!("STKCraft {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Invocation::Passes(request) => passes(&request),
    }
}

/// The help text.
#[must_use]
pub fn usage() -> String {
    let mut text = String::new();
    // `write!` to a String cannot fail, and the result is ignored rather than unwrapped,
    // because the crate forbids panics even on an impossible branch.
    let _ = write!(
        text,
        "\
STKCraft {version} — space mission analysis

USAGE
    stkcraft <COMMAND> [OPTIONS]

COMMANDS
    passes      Compute satellite passes over a ground station
    version     Print the version
    help        Print this text

PASSES OPTIONS
    --tle <LINE1> <LINE2>          A two-line element set. Required.
    --lat <DEGREES>                Station geodetic latitude, positive north. Default 0.
    --lon <DEGREES>                Station longitude, positive east. Default 0.
    --mask <DEGREES>               Horizon mask. Nothing below it is visible. Default 0.
    --hours <HOURS>                How long to analyse. Default 24.
    --step <SECONDS>               Search step. Shorter is slower and more thorough.
                                   Default 30.
    --name <TEXT>                  Station name, for the output. Default \"Station\".

EXAMPLE
    stkcraft passes \\
        --tle \"1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9009\" \\
              \"2 25544  51.6400 208.9163 0006317  69.9862  25.2906 15.49569600 12343\" \\
        --lat 51.5 --lon -0.13 --mask 10

The analysis starts at the element set's own epoch, which is what makes the numbers
reproducible: the same TLE always gives the same table.
",
        version = env!("CARGO_PKG_VERSION")
    );
    text
}

/// Parse the command line.
fn parse<I: Iterator<Item = String>>(mut arguments: I) -> Result<Invocation, String> {
    let Some(command) = arguments.next() else {
        // No arguments: show help rather than an error. A person who types `stkcraft` wants to
        // know what it does.
        return Ok(Invocation::Help);
    };

    match command.as_str() {
        "help" | "--help" | "-h" => Ok(Invocation::Help),
        "version" | "--version" | "-V" => Ok(Invocation::Version),
        "passes" => parse_passes(arguments),
        other => Err(format!("unknown command `{other}`; try `stkcraft help`")),
    }
}

fn parse_passes<I: Iterator<Item = String>>(mut arguments: I) -> Result<Invocation, String> {
    let mut line1 = None;
    let mut line2 = None;
    let mut latitude_deg = 0.0;
    let mut longitude_deg = 0.0;
    let mut minimum_elevation_deg = 0.0;
    let mut hours = 24.0;
    let mut step_seconds = 30.0;
    let mut name = String::from("Station");

    while let Some(flag) = arguments.next() {
        match flag.as_str() {
            "--tle" => {
                line1 = Some(
                    arguments
                        .next()
                        .ok_or("--tle needs two lines, got neither")?,
                );
                line2 = Some(arguments.next().ok_or("--tle needs two lines, got one")?);
            }
            "--lat" => latitude_deg = next_number(&mut arguments, "--lat")?,
            "--lon" => longitude_deg = next_number(&mut arguments, "--lon")?,
            "--mask" => minimum_elevation_deg = next_number(&mut arguments, "--mask")?,
            "--hours" => hours = next_number(&mut arguments, "--hours")?,
            "--step" => step_seconds = next_number(&mut arguments, "--step")?,
            "--name" => name = arguments.next().ok_or("--name needs a value")?,
            other => return Err(format!("unknown option `{other}`; try `stkcraft help`")),
        }
    }

    let line1 = line1.ok_or("--tle is required")?;
    let line2 = line2.ok_or("--tle is required")?;
    if !hours.is_finite() || hours <= 0.0 {
        return Err("--hours must be positive".to_owned());
    }
    if !step_seconds.is_finite() || step_seconds <= 0.0 {
        return Err("--step must be positive".to_owned());
    }

    Ok(Invocation::Passes(PassRequest {
        line1,
        line2,
        latitude_deg,
        longitude_deg,
        minimum_elevation_deg,
        hours,
        step_seconds,
        name,
    }))
}

fn next_number<I: Iterator<Item = String>>(arguments: &mut I, flag: &str) -> Result<f64, String> {
    let text = arguments
        .next()
        .ok_or_else(|| format!("{flag} needs a value"))?;
    text.parse::<f64>()
        .map_err(|_| format!("{flag}: `{text}` is not a number"))
}

/// Run the `passes` subcommand through the command registry.
fn passes(request: &PassRequest) -> Result<(), String> {
    // The whole point of the exercise: build a scenario with the engine's own commands, then
    // ask the registry to compute. Nothing in this file propagates or intersects anything.
    let registry = Registry::builtin();

    let element_set = TwoLineElement::parse(&request.line1, &request.line2)
        .map_err(|error| format!("that two-line element set is not valid: {error}"))?;
    let epoch = element_set.epoch;
    let satellite_name = element_set.catalog_number.clone();

    let mut scenario = Scenario::empty("CLI");
    // Start at the element set's epoch: the same TLE then always produces the same table.
    scenario.epoch = epoch;
    scenario.duration_seconds = request.hours * 3_600.0;

    registry
        .run(
            "satellite.add",
            &mut scenario,
            &Args::none()
                .with("name", Arg::Text(satellite_name.clone()))
                .with("line1", Arg::Text(request.line1.clone()))
                .with("line2", Arg::Text(request.line2.clone())),
        )
        .map_err(|error| format!("could not add the satellite: {error}"))?;

    registry
        .run(
            "station.add",
            &mut scenario,
            &Args::none()
                .with("name", Arg::Text(request.name.clone()))
                .with(
                    "latitude",
                    Arg::Angle(Angle::from_deg(request.latitude_deg)),
                )
                .with(
                    "longitude",
                    Arg::Angle(Angle::from_deg(request.longitude_deg)),
                )
                .with(
                    "minimum-elevation",
                    Arg::Angle(Angle::from_deg(request.minimum_elevation_deg)),
                ),
        )
        .map_err(|error| format!("could not add the station: {error}"))?;

    let outcome = registry
        .run(
            "access.compute",
            &mut scenario,
            &Args::none().with("step-seconds", Arg::Number(request.step_seconds)),
        )
        .map_err(|error| format!("could not compute access: {error}"))?;

    let results = outcome.access.unwrap_or_default();

    print!(
        "{}",
        render::pass_table(
            render::Report {
                station_name: &request.name,
                satellite_name: &satellite_name,
                latitude_deg: request.latitude_deg,
                longitude_deg: request.longitude_deg,
                minimum_elevation_deg: request.minimum_elevation_deg,
                hours: request.hours,
                epoch,
            },
            &results,
        )
    );
    Ok(())
}
