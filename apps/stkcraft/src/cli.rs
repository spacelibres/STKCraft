//! The command line: argument parsing and dispatch into the engine's command registry.
//!
//! Hand-written rather than generated, because the surface is small and a dependency that
//! cannot be fetched offline would block the build. The shape is deliberate: every subcommand
//! builds a [`Args`] and calls [`Registry::run`] by id, exactly as a menu item or a control
//! channel will. Nothing here reimplements engine behaviour.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use stkcraft_astro_engine::{Arg, Args, Registry, Scenario};
use stkcraft_astro_sgp4::TwoLineElement;
use stkcraft_framework_core::Angle;

use crate::knowledge::{library as knowledge_library, opened as knowledge_opened};
use crate::render;

/// What the CLI can be asked to do.
pub enum Invocation {
    Help,
    Version,
    /// One satellite against one station, printed as a pass table.
    Passes(PassRequest),
    /// One satellite against a station on another body, printed as a pass table.
    Planet(PlanetRequest),
    /// Serve the command registry to an agent over standard input and output.
    Mcp(Option<String>),
    /// Ask a model a question; it answers by running the engine's commands.
    Ask(AskRequest),
    /// Ask questions one after another, in one conversation, on standard input.
    Chat(ChatRequest),
    /// List the extension packages installed for this person.
    Extensions,
    /// Show what this installation has computed, newest first.
    History,
    /// Serve the control channel on a loopback port.
    Control(ControlRequest),
    /// Show or change the model this installation uses.
    Model(ModelRequest),
    /// List or search the person's knowledge base.
    Knowledge(KnowledgeRequest),
    /// Run one registry command by id, optionally against a scenario document.
    Run(RunRequest),
    /// Run the fixed agent evaluation cases against the configured model.
    AgentEval(AgentEvalRequest),
    /// Run a deterministic task graph.
    Workflow(WorkflowRequest),
    /// Ask the model for a task graph, validate it, and run it.
    Plan(PlanRequest),
    /// List every command id, its parameters and its menu.
    Commands,
}

/// One generic command invocation: the transport that makes every id reachable from a terminal.
pub struct RunRequest {
    id: String,
    /// A scenario document to load before the command runs.
    scenario: Option<String>,
    /// Where to write the scenario after the command, when the person asked for it.
    write: Option<String>,
    /// Print the structured answer instead of the readable one.
    json: bool,
    /// An IERS finals2000A file, when the command needs Earth orientation.
    eop: Option<String>,
    /// The parameters, as the strings the caller typed; `args_from` converts them by kind.
    params: serde_json::Value,
}

/// One question for the model planner.
pub struct PlanRequest {
    /// The question the graph must answer.
    question: String,
}

/// One deterministic task graph to run.
pub struct WorkflowRequest {
    /// The JSON graph file.
    path: String,
}

/// The fixed agent evaluation cases to run.
pub struct AgentEvalRequest {
    /// The JSONL file; `share/evals/agent/basic.jsonl` by default.
    path: Option<String>,
}

/// One question for the model-backed agent.
pub struct AskRequest {
    /// The question.
    question: String,
    /// Print the machine-readable `ResultEnvelope` instead of the model text.
    json: bool,
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
    /// An IERS `finals2000A.all` file, when the caller supplied one.
    eop: Option<String>,
}

/// What a planet-centred request carries, in the units the command line used.
pub struct PlanetRequest {
    body: String,
    radius_km: f64,
    inclination_deg: f64,
    latitude_deg: f64,
    longitude_deg: f64,
    mask_deg: f64,
    hours: f64,
    step_seconds: f64,
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
        Invocation::Planet(request) => planet(&request),
        Invocation::Mcp(expert) => mcp(expert.as_deref()),
        Invocation::Ask(request) => ask(&request),
        Invocation::Chat(request) => chat(&request),
        Invocation::Extensions => crate::extensions::list(),
        Invocation::History => history(),
        Invocation::Control(request) => control(&request),
        Invocation::Model(request) => model(&request),
        Invocation::Knowledge(request) => knowledge(&request),
        Invocation::Run(request) => run_command(&request),
        Invocation::AgentEval(request) => agent_eval(&request),
        Invocation::Workflow(request) => workflow(&request),
        Invocation::Plan(request) => plan(&request),
        Invocation::Commands => {
            print!("{}", command_list());
            Ok(())
        }
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
    planet      Compute passes over a station on another body
    mcp         Serve the engine's commands to an agent over stdio (Model Context Protocol)
                (`mcp --expert time-frames` restricts it to that expert's allowlist)
    ask         Ask DeepSeek a question; it answers by running the engine's commands
                (`ask --json <question>` prints the ResultEnvelope, not the model text)
    agent-eval  Run the fixed agent cases against the configured model (needs a key)
    workflow    Run a deterministic task graph (`workflow --file graph.json`)
    plan        Ask the model for a graph, validate it, and run it
    chat        Ask questions in one conversation, reading them from standard input
                (`chat --session <id>` resumes one; `chat --sessions` lists them)
    extensions  List the extension packages installed for this person
    history     Show what this installation has computed, newest first
    control     Serve the control channel on loopback (`control --port 7979`)
    model       Show or change the model this installation uses:
                `model --key <KEY>`, `--endpoint <URL>`, `--name <MODEL>`, `--clear`
                `--protocol deepseek|compatible`, `--thinking enabled|disabled`
                `--max-output-tokens <POSITIVE INTEGER>` (includes thinking tokens)
    knowledge   List the documents in your knowledge base, or search them:
                `stkcraft knowledge [<query>]`
    run         Run any registry command by id, against a scenario document:
                `stkcraft run <id> [--scenario FILE] [--write FILE] [--json] [--eop FILE] [--<param> VALUE]`
    commands    List every command id, its parameters and the menu it belongs to
    version     Print the version
    help        Print this text

WINDOW OPTIONS
    --control <PORT>               Serve the control channel from the window, so a launcher can
                                   drive it and photograph it (also STKCRAFT_CONTROL_PORT).
                                   `stkcraft control --port <PORT>` serves the same protocol
                                   with no window at all.

PASSES OPTIONS
    --tle <LINE1> <LINE2>          A two-line element set. Required.
    --lat <DEGREES>                Station geodetic latitude, positive north. Default 0.
    --lon <DEGREES>                Station longitude, positive east. Default 0.
    --mask <DEGREES>               Horizon mask. Nothing below it is visible. Default 0.
    --hours <HOURS>                How long to analyse. Default 24.
    --step <SECONDS>               Search step. Shorter is slower and more thorough.
                                   Default 30.
    --name <TEXT>                  Station name, for the output. Default \"Station\".
    --eop <FILE>                   IERS finals2000A file. Without it, Earth orientation is zero
                                   and the answer says so.

PLANET OPTIONS
    --body <NAME>                  The body, by IAU name (Mars, Titan, ...). Required.
    --radius <KM>                  Circular orbit radius from the body's centre. Required.
    --inclination <DEGREES>        Orbit inclination from the body's ICRF equator. Default 0.
    --lat <DEGREES>                Station planetodetic latitude. Default 0.
    --lon <DEGREES>                Station longitude east of the prime meridian. Default 0.
    --mask <DEGREES>               Horizon mask. Default 0.
    --hours <HOURS>                How long to analyse. Default 6.
    --step <SECONDS>               Search step. Default 10.

EXAMPLE
    stkcraft planet --body Mars --radius 4000 --inclination 30 --lat 0 --lon 0 --mask 10

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
        "planet" => parse_planet(arguments),
        "mcp" => parse_mcp(arguments),
        "chat" => parse_chat(arguments),
        "extensions" => Ok(Invocation::Extensions),
        "history" => Ok(Invocation::History),
        "control" => parse_control(arguments),
        "model" => parse_model(arguments),
        "knowledge" => Ok(Invocation::Knowledge(KnowledgeRequest {
            query: arguments.next(),
        })),
        "ask" => {
            let mut json = false;
            let mut question = None;
            for argument in arguments {
                if argument == "--json" {
                    json = true;
                } else if question.is_none() {
                    question = Some(argument);
                } else {
                    return Err(
                        "`ask` takes one question; quote a question containing spaces".to_owned(),
                    );
                }
            }
            let question = question.ok_or_else(|| {
                "`ask` needs a question, e.g. `stkcraft ask --json \"when does the ISS pass over London?\"`"
                    .to_owned()
            })?;
            Ok(Invocation::Ask(AskRequest { question, json }))
        }
        "run" => parse_run(arguments),
        "agent-eval" => parse_agent_eval(arguments),
        "workflow" => parse_workflow(arguments),
        "plan" => {
            let question = arguments.next().ok_or_else(|| {
                "`plan` needs a question, e.g. `stkcraft plan \"add the ISS ... and compute passes\"`"
                    .to_owned()
            })?;
            Ok(Invocation::Plan(PlanRequest { question }))
        }
        "commands" => Ok(Invocation::Commands),
        other => Err(format!("unknown command `{other}`; try `stkcraft help`")),
    }
}

/// Parse `run <id> [--scenario FILE] [--write FILE] [--json] [--param VALUE]`.
fn parse_run<I: Iterator<Item = String>>(mut arguments: I) -> Result<Invocation, String> {
    let id = arguments.next().ok_or_else(|| {
        "`run` needs a command id, e.g. `stkcraft run access.compute`; try `stkcraft commands`"
            .to_owned()
    })?;
    let registry = Registry::builtin();
    let command = registry
        .get(&id)
        .ok_or_else(|| format!("no command `{id}`; try `stkcraft commands`"))?;
    let mut scenario = None;
    let mut write = None;
    let mut json = false;
    let mut eop = None;
    let mut params = serde_json::Map::new();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--json" => json = true,
            "--scenario" => {
                scenario = Some(arguments.next().ok_or_else(|| {
                    "`--scenario` needs a file, e.g. `--scenario mission.json`".to_owned()
                })?);
            }
            "--write" => {
                write = Some(arguments.next().ok_or_else(|| {
                    "`--write` needs a file, e.g. `--write mission.json`".to_owned()
                })?);
            }
            "--eop" => {
                eop = Some(
                    arguments
                        .next()
                        .ok_or_else(|| "`--eop` needs an IERS finals2000A file".to_owned())?,
                );
            }
            other => {
                let Some(name) = other.strip_prefix("--") else {
                    return Err(format!(
                        "unexpected argument `{other}`; parameters are `--<name> <value>`"
                    ));
                };
                let known = command
                    .descriptor
                    .parameters
                    .iter()
                    .any(|parameter| parameter.name == name);
                if !known {
                    let names = command
                        .descriptor
                        .parameters
                        .iter()
                        .map(|parameter| format!("--{}", parameter.name))
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(format!(
                        "`{id}` has no parameter `{name}`; it takes {}",
                        if names.is_empty() {
                            "no parameters".to_owned()
                        } else {
                            names
                        }
                    ));
                }
                let value = arguments
                    .next()
                    .ok_or_else(|| format!("`--{name}` needs a value"))?;
                params.insert(name.to_owned(), serde_json::Value::String(value));
            }
        }
    }
    Ok(Invocation::Run(RunRequest {
        id,
        scenario,
        write,
        json,
        eop,
        params: serde_json::Value::Object(params),
    }))
}

/// The largest scenario document this command will read, bytes.
///
/// A document is input from outside the repository; a path that names a device or a multi-gigabyte
/// file must fail before the allocation, not after it.
const MAX_SCENARIO_BYTES: u64 = 16 * 1024 * 1024;

/// Run one command against an empty or loaded scenario, then print and optionally save it.
fn run_command(request: &RunRequest) -> Result<(), String> {
    let registry = Registry::builtin();
    let command = registry
        .get(&request.id)
        .ok_or_else(|| format!("no command `{}`; try `stkcraft commands`", request.id))?;
    let mut scenario = match &request.scenario {
        Some(path) => load_scenario(path)?,
        None => Scenario::for_now("command line")
            .map_err(|error| format!("could not read the system clock: {error}"))?,
    };
    scenario
        .validate()
        .map_err(|error| format!("scenario: {error}"))?;
    if let Some(path) = &request.eop {
        scenario.eop_model = Some(load_eop(path)?);
    }
    let args = stkcraft_astro_engine::args_from(&command.descriptor, &request.params);
    let enabled = registry
        .enabled(&request.id, &scenario)
        .map_err(|error| format!("{}: {error}", request.id))?;
    if !enabled {
        return Err(format!(
            "`{}` is not available in this scenario: it needs objects the document does not have \
             (for example a satellite and a station)",
            request.id
        ));
    }
    let outcome = registry
        .run(&request.id, &mut scenario, &args)
        .map_err(|error| format!("{}: {error}", request.id))?;
    if request.json {
        let value = crate::answer::json(&outcome, &scenario);
        let text = serde_json::to_string_pretty(&value)
            .map_err(|error| format!("could not print the answer: {error}"))?;
        println!("{text}");
    } else {
        println!("{}", crate::answer::text(&outcome, &scenario));
    }
    if let Some(path) = &request.write {
        save_scenario(path, &scenario)?;
    }
    Ok(())
}

fn load_eop(path: &str) -> Result<Arc<stkcraft_astro_frames::Finals2000A>, String> {
    stkcraft_astro_frames::Finals2000A::from_path(Path::new(path))
        .map(Arc::new)
        .map_err(|error| format!("could not read the EOP file `{path}`: {error}"))
}

fn load_scenario(path: &str) -> Result<Scenario, String> {
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("could not read `{path}`: {error}"))?;
    if metadata.len() > MAX_SCENARIO_BYTES {
        return Err(format!(
            "`{path}` is {} bytes; a scenario document is limited to {MAX_SCENARIO_BYTES}",
            metadata.len()
        ));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("could not read `{path}`: {error}"))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("`{path}` is not a scenario document: {error}"))
}

fn save_scenario(path: &str, scenario: &Scenario) -> Result<(), String> {
    let text = serde_json::to_string_pretty(scenario)
        .map_err(|error| format!("could not write the scenario: {error}"))?;
    std::fs::write(path, text).map_err(|error| format!("could not write `{path}`: {error}"))
}

/// Every command id, its parameters and its menu, for a caller that has no other catalogue.
fn command_list() -> String {
    let mut text = String::from(
        "COMMAND REGISTRY\nRun one with `stkcraft run <id> [--scenario FILE] [--write FILE] [--json] [--eop FILE] [--<param> VALUE]`.\n\n",
    );
    for command in Registry::builtin().iter() {
        let _ = writeln!(
            text,
            "{:<22} {:<28} {}",
            command.descriptor.id, command.descriptor.label, command.descriptor.menu_path
        );
        for parameter in command.descriptor.parameters {
            let kind = match parameter.kind {
                stkcraft_astro_engine::commands::ParameterKind::Text => "text",
                stkcraft_astro_engine::commands::ParameterKind::Number => "number",
                stkcraft_astro_engine::commands::ParameterKind::Degrees => "degrees",
                stkcraft_astro_engine::commands::ParameterKind::DegreesPair => "corners",
            };
            let _ = writeln!(
                text,
                "    --{:<20} {:<8} {}{}",
                parameter.name,
                kind,
                if parameter.required { "required; " } else { "" },
                parameter.description
            );
        }
    }
    text
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
    let mut eop = None;

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
            "--eop" => eop = Some(arguments.next().ok_or("--eop needs a file path")?),
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
        eop,
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
/// Record what a run computed, so the number can be traced back to its inputs.
///
/// A failure to record is reported on stderr and never fails the run: the analysis already happened,
/// and refusing to print it because a database was busy would be the tail wagging the dog. What is
/// *not* optional is that the record carries the inputs' digest — a row without it cannot be
/// reproduced, and the storage layer refuses to pretend otherwise.
/// Open the computation record for this installation, when there is one.
fn open_ledger() -> Option<stkcraft_framework_storage::Store> {
    let paths = stkcraft_framework_storage::Paths::for_user().ok()?;
    stkcraft_framework_storage::Store::open(&paths.record()).ok()
}

struct RunRecord<'a> {
    command: &'a str,
    parameters: &'a str,
    inputs: &'a str,
    summary: &'a str,
    result: &'a str,
    errors: &'a [String],
    windows: &'a [stkcraft_framework_storage::AccessWindowRecord],
}

fn record_run(record: &RunRecord<'_>) {
    let Ok(paths) = stkcraft_framework_storage::Paths::for_user() else {
        eprintln!("stkcraft: the run was not recorded: no data directory");
        return;
    };
    if let Err(message) = record_run_at(&paths, record) {
        eprintln!("stkcraft: the run was not recorded: {message}");
    }
}

/// [`record_run`] against an explicit data directory, which is what a test uses.
fn record_run_at(
    paths: &stkcraft_framework_storage::Paths,
    record: &RunRecord<'_>,
) -> Result<(), String> {
    let mut store = stkcraft_framework_storage::Store::open(&paths.record())
        .map_err(|error| error.to_string())?;
    let errors_json = serde_json::to_string(record.errors)
        .map_err(|error| format!("could not record the verifier errors: {error}"))?;
    let id = store
        .record_computation(&stkcraft_framework_storage::Computation {
            scenario_name: "CLI".to_owned(),
            scenario_revision: None,
            command: record.command.to_owned(),
            parameters: record.parameters.to_owned(),
            engine_version: env!("CARGO_PKG_VERSION").to_owned(),
            inputs_digest: stkcraft_framework_storage::digest(record.inputs.as_bytes()),
            summary: record.summary.to_owned(),
            result: record.result.to_owned(),
            errors: errors_json,
        })
        .map_err(|error| error.to_string())?;
    for window in record.windows {
        store
            .record_window(id, window)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// The UTC text of an instant, for a stored window.
fn utc_text(epoch: stkcraft_framework_time::Epoch, offset_seconds: f64) -> String {
    let instant = stkcraft_framework_time::Epoch::from_julian(
        epoch.julian().plus_seconds(offset_seconds),
        epoch.scale(),
    );
    instant
        .to_utc()
        .ok()
        .and_then(|utc| stkcraft_framework_time::julian_to_utc_naive(utc.julian()))
        .map_or_else(
            || format!("JD {:.6}", instant.julian().sum()),
            |datetime| datetime.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        )
}

fn passes(request: &PassRequest) -> Result<(), String> {
    // The whole point of the exercise: build a scenario with the engine's own commands, then
    // ask the registry to compute. Nothing in this file propagates or intersects anything.
    let registry = Registry::builtin();

    let element_set = TwoLineElement::parse(&request.line1, &request.line2)
        .map_err(|error| format!("that two-line element set is not valid: {error}"))?;
    let epoch = element_set.epoch;
    let satellite_name = element_set.catalog_number.clone();

    // Start at the element set's epoch: the same TLE then always produces the same table, and the
    // clock cannot move the analysis.
    let mut scenario = Scenario::empty_at("CLI", epoch);
    scenario.duration_seconds = request.hours * 3_600.0;
    if let Some(path) = &request.eop {
        scenario.eop_model = Some(load_eop(path)?);
    }

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

    let ledger_result = serde_json::to_string(&crate::answer::json(&outcome, &scenario))
        .unwrap_or_else(|_| "{}".to_owned());
    let ledger_errors: Vec<String> = Vec::new();
    let results = outcome.access.unwrap_or_default();

    // The provenance: what ran, on what, with which parameters.
    let mut windows = Vec::new();
    for access in &results {
        for window in &access.windows {
            windows.push(stkcraft_framework_storage::AccessWindowRecord {
                satellite: satellite_name.clone(),
                station: request.name.clone(),
                start_utc: utc_text(epoch, window.start.as_seconds()),
                end_utc: utc_text(epoch, window.end.as_seconds()),
                start_seconds: window.start.as_seconds(),
                duration_seconds: window.duration().as_seconds(),
                minimum_elevation_deg: request.minimum_elevation_deg,
            });
        }
    }
    record_run(&RunRecord {
        command: "access.compute",
        parameters: &format!(
            "{{\"step-seconds\":{},\"hours\":{}}}",
            request.step_seconds, request.hours
        ),
        inputs: &format!("{}{}{}", request.line1, request.line2, request.name),
        summary: &outcome.summary,
        result: &ledger_result,
        errors: &ledger_errors,
        windows: &windows,
    });

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
    for limitation in &outcome.limitations {
        println!("note: {limitation}");
    }
    for formula in &outcome.formulas {
        println!("formula: {} — {}", formula.id, formula.source);
    }
    Ok(())
}

/// Parse the `planet` subcommand.
fn parse_planet<I: Iterator<Item = String>>(mut arguments: I) -> Result<Invocation, String> {
    let mut request = PlanetRequest {
        body: String::new(),
        radius_km: 0.0,
        inclination_deg: 0.0,
        latitude_deg: 0.0,
        longitude_deg: 0.0,
        mask_deg: 0.0,
        hours: 6.0,
        step_seconds: 10.0,
    };
    let mut have_radius = false;
    while let Some(flag) = arguments.next() {
        let number = |text: Option<String>, flag: &str| -> Result<f64, String> {
            let text = text.ok_or_else(|| format!("`{flag}` needs a value"))?;
            text.parse::<f64>()
                .map_err(|_| format!("`{text}` is not a number for `{flag}`"))
        };
        match flag.as_str() {
            "--body" => {
                request.body = arguments
                    .next()
                    .ok_or_else(|| "`--body` needs a value".to_owned())?;
            }
            "--radius" => {
                request.radius_km = number(arguments.next(), "--radius")?;
                have_radius = true;
            }
            "--inclination" => {
                request.inclination_deg = number(arguments.next(), "--inclination")?;
            }
            "--lat" => request.latitude_deg = number(arguments.next(), "--lat")?,
            "--lon" => request.longitude_deg = number(arguments.next(), "--lon")?,
            "--mask" => request.mask_deg = number(arguments.next(), "--mask")?,
            "--hours" => request.hours = number(arguments.next(), "--hours")?,
            "--step" => request.step_seconds = number(arguments.next(), "--step")?,
            other => return Err(format!("unknown option `{other}`; try `stkcraft help`")),
        }
    }
    if request.body.is_empty() {
        return Err("`--body` is required; it names the body, e.g. `Mars`".to_owned());
    }
    if !have_radius {
        return Err("`--radius` is required: the circular orbit radius in km".to_owned());
    }
    Ok(Invocation::Planet(request))
}

/// Run the `planet` subcommand: build the scenario, run the command, print the result.
fn planet(request: &PlanetRequest) -> Result<(), String> {
    let registry = Registry::builtin();
    let mut scenario = Scenario::for_now("CLI")
        .map_err(|error| format!("could not read the system clock: {error}"))?;
    let outcome = registry
        .run(
            "planet.access",
            &mut scenario,
            &Args::none()
                .with("body", Arg::Text(request.body.clone()))
                .with("radius-km", Arg::Number(request.radius_km))
                // Angles go in as angles: the command declares them as degrees, and the typed
                // value is what stops a radians/degrees mix-up reaching the geometry.
                .with(
                    "inclination-deg",
                    Arg::Angle(Angle::from_deg(request.inclination_deg)),
                )
                .with(
                    "latitude-deg",
                    Arg::Angle(Angle::from_deg(request.latitude_deg)),
                )
                .with(
                    "longitude-deg",
                    Arg::Angle(Angle::from_deg(request.longitude_deg)),
                )
                .with("mask-deg", Arg::Angle(Angle::from_deg(request.mask_deg)))
                .with("hours", Arg::Number(request.hours))
                .with("step-seconds", Arg::Number(request.step_seconds)),
        )
        .map_err(|error| format!("{error}"))?;
    println!("STKCraft {}", env!("CARGO_PKG_VERSION"));
    println!("{}", outcome.summary);
    // The passes themselves, not only how many: the times are the answer, and a summary that
    // counts them leaves the reader to compute what they asked for.
    for pass in outcome.passes.iter().flatten() {
        let start = instant_at(scenario.epoch, pass.start_seconds);
        let end = instant_at(scenario.epoch, pass.end_seconds);
        println!(
            "  {} → {} UTC   {}",
            crate::render::format_epoch(start),
            crate::render::format_epoch(end),
            crate::render::format_duration(pass.duration_seconds())
        );
    }
    for limitation in &outcome.limitations {
        println!("note: {limitation}");
    }
    for formula in &outcome.formulas {
        println!("formula: {} — {}", formula.id, formula.source);
    }
    Ok(())
}

/// An absolute instant from a scenario epoch and an offset.
fn instant_at(
    epoch: stkcraft_framework_time::Epoch,
    offset_seconds: f64,
) -> stkcraft_framework_time::Epoch {
    stkcraft_framework_time::Epoch::from_julian(
        epoch.julian().plus_seconds(offset_seconds),
        epoch.scale(),
    )
}

/// Serve the engine's commands over the Model Context Protocol on stdin/stdout.
///
/// The skills directory is looked for beside the executable first and then in the working
/// directory, because a deployment copies the binary and its documents together while a developer
/// runs from the repository root. Absent skills are not an error: the tools still work.
fn mcp(expert: Option<&str>) -> Result<(), String> {
    let skills = skills_directory();
    let loaded = installed_extensions();
    let mut server = stkcraft_agent_mcp::Server::new(
        app_demo_scenario()?,
        stkcraft_agent_mcp::load_skills(&skills),
    )
    .with_knowledge(knowledge_library()?)
    .with_capabilities(load_capabilities()?)
    .with_limits(load_limits()?)
    .with_extensions(&loaded);
    if let Some(id) = expert {
        let role = load_expert_role(id)?;
        server = server.with_role(role);
    }
    server.serve()
}

/// Parse `mcp [--expert ID]`.
fn parse_mcp<I: Iterator<Item = String>>(mut arguments: I) -> Result<Invocation, String> {
    let mut expert = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--expert" => {
                expert = Some(
                    arguments
                        .next()
                        .ok_or("`--expert` needs an expert id, e.g. `--expert time-frames`")?,
                );
            }
            other => return Err(format!("unknown option `{other}` for `mcp`")),
        }
    }
    Ok(Invocation::Mcp(expert))
}

fn load_capabilities()
-> Result<Vec<stkcraft_agent_knowledge::capability::CapabilityManifest>, String> {
    let beside_executable = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let root = beside_executable
        .filter(|root| root.join("share/capabilities").is_dir())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    if !root.join("share/capabilities").is_dir() {
        return Ok(Vec::new());
    }
    stkcraft_agent_knowledge::capability::load_all(&root)
        .map_err(|errors| format!("capabilities: {}", errors.join("; ")))
}

fn load_limits() -> Result<Vec<stkcraft_agent_knowledge::capability::CapabilityLimit>, String> {
    let beside_executable = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let root = beside_executable
        .filter(|root| root.join("share/capabilities/limits.toml").is_file())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    stkcraft_agent_knowledge::capability::load_limits(&root)
        .map_err(|errors| format!("capability limits: {}", errors.join("; ")))
}

fn load_expert_role(id: &str) -> Result<stkcraft_agent_knowledge::expert::ExpertRole, String> {
    let beside_executable = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let root = beside_executable
        .filter(|root| {
            root.join("share/experts")
                .join(id)
                .join("role.toml")
                .is_file()
        })
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    stkcraft_agent_knowledge::expert::load_role(&root, id)
        .map_err(|error| format!("expert `{id}`: {error}"))
}

/// Ask `DeepSeek` a question, with the engine's commands as its tools and the skills as its library.
///
/// The model needs `DEEPSEEK_API_KEY`; without one the message says so instead of failing inside a
/// request. Every tool call it makes is printed as it happens, because an analysis whose steps a
/// person cannot see is an analysis nobody can check.
fn ask(request: &AskRequest) -> Result<(), String> {
    let model = crate::prefs::model_provider()?;
    let server = stkcraft_agent_mcp::Server::new(
        app_demo_scenario()?,
        stkcraft_agent_mcp::load_skills(&skills_directory()),
    )
    .with_knowledge(knowledge_library()?)
    .with_capabilities(load_capabilities()?)
    .with_limits(load_limits()?);
    let mut agent = stkcraft_agent_runtime::Agent::new(model, server);
    if let Some(store) = open_ledger() {
        agent = agent.with_ledger(store);
    }
    // Printed as they happen rather than collected and printed at the end: a question can take a
    // minute, and a step nobody can see while it runs is a step nobody can interrupt. In JSON mode
    // the trace goes to stderr so stdout stays one parseable envelope.
    let mut print = |line: &str| {
        if request.json {
            eprintln!("  · {line}");
        } else {
            println!("  · {line}");
        }
    };
    match agent.ask_observing(&request.question, &mut print) {
        Ok(answer) if request.json => {
            let envelope = answer.envelope(&request.question);
            let text = serde_json::to_string_pretty(&envelope)
                .map_err(|error| format!("could not print the result envelope: {error}"))?;
            println!("{text}");
            Ok(())
        }
        Ok(answer) => {
            println!("{}", answer.text);
            Ok(())
        }
        Err(message) => Err(message),
    }
}

/// Ask questions one after another, in one conversation, on standard input.
///
/// The conversation is the point: a follow-up can say "and the day after?" because the agent still
/// holds the scenario, the tool calls and the answers of the questions before it. One agent is built
/// for the session, and `exit`, `quit` or end of input ends it — a model that forgot everything
/// between questions would make every question restate the whole job.
///
/// # Errors
///
/// A message when the model cannot be configured, which is the same one `ask` gives: the difference
/// between the two commands is the number of questions, not what they need to work.
/// What the `chat` subcommand was asked for.
pub struct ChatRequest {
    /// The session to resume, if any.
    session: Option<String>,
    /// Whether to list the stored sessions instead of chatting.
    list: bool,
}

/// Parse `chat [--session <id>] [--sessions]`.
fn parse_chat<I: Iterator<Item = String>>(mut arguments: I) -> Result<Invocation, String> {
    let mut request = ChatRequest {
        session: None,
        list: false,
    };
    while let Some(flag) = arguments.next() {
        match flag.as_str() {
            "--session" => {
                request.session = Some(arguments.next().ok_or_else(|| {
                    "`--session` needs an id; `--sessions` lists them".to_owned()
                })?);
            }
            "--sessions" => request.list = true,
            other => return Err(format!("unknown option `{other}`; try `stkcraft help`")),
        }
    }
    Ok(Invocation::Chat(request))
}

/// Where a conversation is stored: the data directory's `sessions/`.
fn sessions_directory() -> Result<std::path::PathBuf, String> {
    let paths = stkcraft_framework_storage::Paths::for_user().map_err(|error| error.to_string())?;
    Ok(paths.root().join("sessions"))
}

fn chat(request: &ChatRequest) -> Result<(), String> {
    let directory = sessions_directory()?;
    if request.list {
        println!(
            "STKCraft {} — stored conversations in {}",
            env!("CARGO_PKG_VERSION"),
            directory.display()
        );
        let sessions = stkcraft_agent_runtime::session::Session::list(&directory);
        if sessions.is_empty() {
            println!("  none yet: `stkcraft chat` starts one");
        }
        for id in sessions {
            println!("  {id}");
        }
        return Ok(());
    }
    let model = crate::prefs::model_provider()?;
    let server = stkcraft_agent_mcp::Server::new(
        app_demo_scenario()?,
        stkcraft_agent_mcp::load_skills(&skills_directory()),
    )
    .with_knowledge(knowledge_library()?)
    .with_capabilities(load_capabilities()?)
    .with_limits(load_limits()?);
    // A conversation is a session on disk, so closing the terminal does not lose it: `--session`
    // resumes one and the id is printed for next time. This is the same shape a harness keeps its own
    // sessions in — a transcript that outlives the process.
    let session = match &request.session {
        Some(id) => stkcraft_agent_runtime::session::Session::open(&directory, id)?,
        None => stkcraft_agent_runtime::session::Session::create(&directory)?,
    };
    let id = session.id.clone();
    let mut agent = stkcraft_agent_runtime::Agent::new(model, server).with_session(session);
    if let Some(store) = open_ledger() {
        agent = agent.with_ledger(store);
    }
    println!(
        "STKCraft {} — session {id}; `exit` ends it, `stkcraft chat --session {id}` resumes it.",
        env!("CARGO_PKG_VERSION")
    );
    let stdin = std::io::stdin();
    loop {
        print!("> ");
        // Flushing by hand is what makes the prompt appear before the read on a pipe as well as on
        // a terminal; without it the prompt arrives after the answer.
        if let Err(error) = std::io::Write::flush(&mut std::io::stdout()) {
            return Err(format!("could not write the prompt: {error}"));
        }
        let mut line = String::new();
        match stdin.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => return Err(format!("could not read a question: {error}")),
        }
        let question = line.trim();
        if question.is_empty() {
            continue;
        }
        if matches!(question, "exit" | "quit") {
            break;
        }
        let mut print = |line: &str| println!("  · {line}");
        match agent.ask_observing(question, &mut print) {
            Ok(answer) => println!("{}", answer.text),
            // A failed question does not end the session: the next one may be the user correcting
            // the parameter that failed, which is exactly when the conversation is worth keeping.
            Err(message) => eprintln!("error: {message}"),
        }
    }
    Ok(())
}

/// Where the skill documents live, if they are anywhere this build can find them.
///
/// `share/skills/` beside the executable first (a deployment copies the binary and its `share/`
/// together), then `share/skills/` in the working directory (a developer runs from the repository
/// root). Content is not scattered beside the source: `share/` holds what the app loads, and the
/// repository layout is `docs/standards.md` §9.
pub fn skills_directory() -> std::path::PathBuf {
    let beside_executable = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("share/skills")));
    if let Some(path) = beside_executable.filter(|path| path.is_dir()) {
        return path;
    }
    std::path::PathBuf::from("share/skills")
}

/// What the `control` subcommand was asked for.
pub struct ControlRequest {
    /// The loopback port to listen on.
    port: u16,
}

/// What `model` was asked to do.
pub struct ModelRequest {
    /// A credential to store.
    key: Option<String>,
    /// An endpoint to store.
    endpoint: Option<String>,
    /// A model name to store.
    name: Option<String>,
    /// An explicitly selected protocol.
    protocol: Option<String>,
    /// An explicitly selected thinking mode.
    thinking: Option<String>,
    /// An explicit total output-token cap.
    max_output_tokens: Option<String>,
    /// Remove the stored credential.
    clear: bool,
}

/// Parse model connection, protocol and thinking settings without making a model request.
fn parse_model<I: Iterator<Item = String>>(mut arguments: I) -> Result<Invocation, String> {
    let mut request = ModelRequest {
        key: None,
        endpoint: None,
        name: None,
        protocol: None,
        thinking: None,
        max_output_tokens: None,
        clear: false,
    };
    while let Some(flag) = arguments.next() {
        let mut value = |what: &str| {
            arguments
                .next()
                .ok_or_else(|| format!("`{what}` needs a value"))
        };
        match flag.as_str() {
            "--key" => request.key = Some(value("--key")?),
            "--endpoint" => request.endpoint = Some(value("--endpoint")?),
            "--name" => request.name = Some(value("--name")?),
            "--protocol" => {
                let selected = value("--protocol")?;
                if !matches!(selected.as_str(), "deepseek" | "compatible") {
                    return Err("--protocol must be deepseek or compatible".to_owned());
                }
                request.protocol = Some(selected);
            }
            "--thinking" => {
                let selected = value("--thinking")?;
                if !matches!(selected.as_str(), "enabled" | "disabled") {
                    return Err("--thinking must be enabled or disabled".to_owned());
                }
                request.thinking = Some(selected);
            }
            "--clear" => request.clear = true,
            "--max-output-tokens" => {
                request.max_output_tokens = Some(value("--max-output-tokens")?);
            }
            other => return Err(format!("unknown option `{other}`; try `stkcraft help`")),
        }
    }
    Ok(Invocation::Model(request))
}

/// Show or change the model this installation uses.
///
/// Three states matter to a person: which endpoint and model are in force, whether a credential is
/// stored (never the credential itself), and where it is stored. Changing one writes the settings or
/// the credential file and says which.
fn model(request: &ModelRequest) -> Result<(), String> {
    let mut prefs = crate::prefs::Prefs::try_load()?;
    let mut changed = Vec::new();
    if let Some(endpoint) = &request.endpoint {
        prefs.model_endpoint = Some(endpoint.trim().to_owned());
        changed.push(format!("endpoint {endpoint}"));
    }
    if let Some(name) = &request.name {
        prefs.model = Some(name.trim().to_owned());
        changed.push(format!("model {name}"));
    }
    if let Some(protocol) = &request.protocol {
        prefs.model_protocol = Some(protocol.clone());
        changed.push(format!("protocol {protocol}"));
    }
    if let Some(thinking) = &request.thinking {
        prefs.model_thinking = Some(thinking.clone());
        changed.push(format!("thinking {thinking}"));
    }
    // Validate before any preference or credential mutation, including key-only/clear-only calls.
    if let Some(cap) = &request.max_output_tokens {
        prefs.model_max_output_tokens = Some(cap.clone());
        changed.push(format!("max output tokens {cap}"));
    }
    crate::prefs::validate_model_options(&prefs)?;
    if !changed.is_empty() {
        prefs.save()?;
    }
    if let Some(key) = &request.key {
        crate::prefs::store_credential(key)?;
        changed.push("credential stored".to_owned());
    }
    if request.clear {
        if let Some(path) = crate::prefs::credential_path() {
            let _ = std::fs::remove_file(&path);
        }
        changed.push("credential removed".to_owned());
    }

    // Report the state *after* the change, resolved the same way the agent resolves it, so the
    // answer to "what will be used" is the answer the next command gets.
    match crate::prefs::model_provider() {
        Ok(provider) => {
            println!(
                "STKCraft {} — model {} at {}",
                env!("CARGO_PKG_VERSION"),
                provider.model,
                provider.base_url
            );
            println!(
                "  protocol: {:?}; thinking: {:?}; max output tokens: {:?}",
                provider.options.protocol,
                provider.options.thinking,
                provider.options.max_output_tokens
            );
            println!(
                "  credential: {} ({}-byte stored or environment key, timeout {} s)",
                if provider.has_credential() {
                    "present"
                } else {
                    "absent"
                },
                provider.api_key.len(),
                provider.timeout_seconds
            );
        }
        Err(message) => println!("STKCraft {} — {message}", env!("CARGO_PKG_VERSION")),
    }
    if !changed.is_empty() {
        println!("  changed: {}", changed.join(", "));
    }
    for (label, path) in [
        ("settings", crate::prefs::Prefs::path()),
        ("credential", crate::prefs::credential_path()),
    ] {
        if let Some(path) = path {
            println!("  {label}: {}", path.display());
        }
    }
    Ok(())
}

/// Parse `control --port <p>`.
fn parse_control<I: Iterator<Item = String>>(mut arguments: I) -> Result<Invocation, String> {
    let mut port = 7979_u16;
    while let Some(flag) = arguments.next() {
        match flag.as_str() {
            "--port" => {
                let text = arguments
                    .next()
                    .ok_or_else(|| "`--port` needs a number".to_owned())?;
                port = text
                    .parse::<u16>()
                    .map_err(|_| format!("`{text}` is not a port number"))?;
                if port == 0 {
                    return Err("`--port 0` would not be reachable; give a real port".to_owned());
                }
            }
            other => return Err(format!("unknown option `{other}`; try `stkcraft help`")),
        }
    }
    Ok(Invocation::Control(ControlRequest { port }))
}

/// Serve the control channel: the same registry, driven over a loopback socket.
///
/// The token is written where a client looks (`$XDG_RUNTIME_DIR/stkcraft/control-<port>.token`) and
/// printed nowhere else: a token on standard output ends up in a terminal scrollback and a log.
fn control(request: &ControlRequest) -> Result<(), String> {
    let session = stkcraft_agent_control::Control::new(
        control_session()?,
        stkcraft_agent_control::new_token(),
    );
    stkcraft_agent_control::serve(request.port, session)
}

/// The session a control channel serves: the engine's commands, the skills, and the person's
/// documents.
///
/// Shared by the CLI's headless channel and by the window's, so the two answer `engine.commands` and
/// `knowledge.*` identically — the difference between them is only the window-owned half.
///
/// # Errors
///
/// A message when the demo scenario or the knowledge base cannot be opened.
pub(crate) fn control_session() -> Result<stkcraft_agent_mcp::Server, String> {
    let loaded = installed_extensions();
    Ok(stkcraft_agent_mcp::Server::new(
        app_demo_scenario()?,
        stkcraft_agent_mcp::load_skills(&skills_directory()),
    )
    .with_knowledge(knowledge_library()?)
    .with_capabilities(load_capabilities()?)
    .with_limits(load_limits()?)
    .with_extensions(&loaded))
}

/// The packages installed for this person, contributed commands included.
///
/// The tier is `Internal` because everything in that directory was put there by the person using the app;
/// nothing in this program reads a tier from a package.
pub(crate) fn installed_extensions() -> stkcraft_framework_extensions::Loaded {
    match crate::prefs::extensions_dir() {
        Some(directory) => stkcraft_framework_extensions::load(
            &directory,
            stkcraft_framework_extensions::TrustTier::Internal,
        ),
        None => stkcraft_framework_extensions::Loaded::default(),
    }
}

/// Print what this installation has computed, newest first.
///
/// The provenance a stored run carries — command, parameters, engine version and the digest of its
/// inputs — is what makes a number in a report checkable months later. A record that cannot be opened
/// is reported rather than hidden: an empty history and an unreadable one are different facts.
fn history() -> Result<(), String> {
    let paths = stkcraft_framework_storage::Paths::for_user().map_err(|error| error.to_string())?;
    println!(
        "STKCraft {} — history from {}",
        env!("CARGO_PKG_VERSION"),
        paths.record().display()
    );
    let store = stkcraft_framework_storage::Store::open(&paths.record())
        .map_err(|error| error.to_string())?;
    let computations = store.computations(20).map_err(|error| error.to_string())?;
    if computations.is_empty() {
        println!("  nothing recorded yet");
        return Ok(());
    }
    for computation in computations {
        println!(
            "  {}  {}  {}  {} window(s)  {} ms",
            computation.started_at,
            computation.record.command,
            computation.record.scenario_name,
            computation.windows,
            computation.duration.as_millis()
        );
        println!(
            "      inputs {}…  engine {}  {}",
            computation
                .record
                .inputs_digest
                .get(..12)
                .unwrap_or_default(),
            computation.record.engine_version,
            computation.record.summary
        );
    }
    Ok(())
}

/// Say which files in the directory are not in the library, and why.
///
/// A person who dropped a paper in the folder and cannot search it is owed the reason rather than a
/// listing that quietly leaves it out.
fn knowledge_skipped(opened: &stkcraft_agent_knowledge::Opened) {
    if opened.skipped.is_empty() {
        return;
    }
    println!("  {} file(s) not in the library:", opened.skipped.len());
    for skipped in &opened.skipped {
        println!("    {skipped}");
    }
}

/// Run the `knowledge` subcommand: list the library, or search it.
fn knowledge(request: &KnowledgeRequest) -> Result<(), String> {
    let opened = knowledge_opened()?;
    let library = &opened.library;
    match &request.query {
        None => {
            println!(
                "STKCraft {} — knowledge base at {}",
                env!("CARGO_PKG_VERSION"),
                library.root().display()
            );
            if library.is_empty() {
                println!("  no documents: drop text, markdown, CSV or PDF files in that directory");
                knowledge_skipped(&opened);
                return Ok(());
            }
            println!(
                "  {} document(s), {} passage(s), {} parsed / {} reused from the index cache",
                library.documents().len(),
                library.passages(),
                opened.reuse.parsed,
                opened.reuse.reused
            );
            for document in library.documents() {
                // A paper is cited by page, so its page count is part of what the listing says; a text
                // file has none, and saying "0 page(s)" would be a lie about a document without pages.
                match document.pages {
                    Some(pages) => println!(
                        "  {} ({} byte(s), {} page(s), {} passage(s){})",
                        document.path,
                        document.bytes,
                        pages,
                        document.chunks.len(),
                        if document.limited {
                            ", read in part: a bound was reached"
                        } else {
                            ""
                        }
                    ),
                    None => println!(
                        "  {} ({} byte(s), {} passage(s))",
                        document.path,
                        document.bytes,
                        document.chunks.len()
                    ),
                }
                if document.ocr_pages > 0 {
                    // How much of the document rests on a recognised page is a fact a person needs when
                    // they weigh an answer, so it is printed rather than kept in a log.
                    println!(
                        "      {} page(s) had no text layer and were read by OCR",
                        document.ocr_pages
                    );
                }
                if let Some(first) = document.formulas.first() {
                    // The first place, and the count: a listing that said "34 formulas" without one place
                    // to look would be a claim nobody can check in a second.
                    println!(
                        "      {} formula(s); first at {}",
                        document.formulas.len(),
                        first.cite(&document.path)
                    );
                }
            }
            knowledge_skipped(&opened);
            Ok(())
        }
        Some(query) => {
            let search = library.search(query, 5);
            if search.missed {
                println!(
                    "no passage in the knowledge base matches `{query}` \
                     ({} document(s) searched)",
                    search.documents
                );
                return Ok(());
            }
            for hit in &search.formulas {
                println!(
                    "[{}] score {:.2} (formula)\n    {}{}",
                    hit.cite(),
                    hit.score,
                    hit.formula.unicode,
                    hit.formula
                        .problem
                        .as_deref()
                        .map(|problem| format!("\n    introduced by: {problem}"))
                        .unwrap_or_default()
                );
            }
            for hit in &search.hits {
                println!("[{}] score {:.2}", hit.cite(), hit.score);
                for line in hit.text.lines() {
                    println!("    {line}");
                }
                println!();
            }
            Ok(())
        }
    }
}

/// What the `knowledge` subcommand was asked for.
pub struct KnowledgeRequest {
    /// The query, or nothing to list the library.
    query: Option<String>,
}

/// The scenario an MCP session starts with.
fn app_demo_scenario() -> Result<stkcraft_astro_engine::Scenario, String> {
    crate::app::demo_scenario()
}

/// Ask the model for a graph, validate it and execute it against a fresh scenario.
fn plan(request: &PlanRequest) -> Result<(), String> {
    let model = crate::prefs::model_provider()?;
    let server = stkcraft_agent_mcp::Server::new(Scenario::empty("planning"), Vec::new())
        .with_capabilities(load_capabilities()?)
        .with_limits(load_limits()?);
    let tools: Vec<serde_json::Value> = server
        .tools_json()
        .into_iter()
        .filter(|tool| {
            tool.get("name")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|name| !name.starts_with("knowledge.") && name != "agent.report")
        })
        .collect();
    let planner = stkcraft_agent_runtime::task::Planner::new(model);
    let graph = planner.plan_corrected(&request.question, &tools, &Registry::builtin())?;
    let graph_text = serde_json::to_string_pretty(&graph)
        .map_err(|error| format!("could not print the plan: {error}"))?;
    eprintln!("plan:\n{graph_text}");
    let mut scenario = Scenario::empty("planned");
    let outcomes = graph
        .execute(&Registry::builtin(), &mut scenario)
        .map_err(|errors| errors.join("\n"))?;
    let text = serde_json::to_string_pretty(&outcomes)
        .map_err(|error| format!("could not print the outcomes: {error}"))?;
    println!("{text}");
    let failed: Vec<&str> = outcomes
        .iter()
        .filter(|outcome| {
            !matches!(
                outcome.status,
                stkcraft_agent_runtime::task::TaskStatus::Completed
            )
        })
        .map(|outcome| outcome.id.as_str())
        .collect();
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "plan: task(s) did not complete: {}",
            failed.join(", ")
        ))
    }
}

/// Parse `workflow --file FILE`.
fn parse_workflow<I: Iterator<Item = String>>(mut arguments: I) -> Result<Invocation, String> {
    let mut path = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--file" => {
                path = Some(arguments.next().ok_or(
                    "`--file` needs a JSON graph, e.g. `--file share/workflows/iss-london.json`",
                )?);
            }
            other => return Err(format!("unknown option `{other}` for `workflow`")),
        }
    }
    Ok(Invocation::Workflow(WorkflowRequest {
        path: path.ok_or("`workflow` needs `--file <graph.json>`")?,
    }))
}

/// Run a deterministic task graph against a fresh scenario.
fn workflow(request: &WorkflowRequest) -> Result<(), String> {
    let text = std::fs::read_to_string(&request.path)
        .map_err(|error| format!("could not read `{}`: {error}", request.path))?;
    let graph: stkcraft_agent_runtime::task::TaskGraph = serde_json::from_str(&text)
        .map_err(|error| format!("`{}` is not a task graph: {error}", request.path))?;
    let mut scenario = Scenario::empty("workflow");
    let outcomes = graph
        .execute(&Registry::builtin(), &mut scenario)
        .map_err(|errors| errors.join("\n"))?;
    let text = serde_json::to_string_pretty(&outcomes)
        .map_err(|error| format!("could not print the workflow: {error}"))?;
    println!("{text}");
    let failed: Vec<&str> = outcomes
        .iter()
        .filter(|outcome| {
            !matches!(
                outcome.status,
                stkcraft_agent_runtime::task::TaskStatus::Completed
            )
        })
        .map(|outcome| outcome.id.as_str())
        .collect();
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "workflow: task(s) did not complete: {}",
            failed.join(", ")
        ))
    }
}

/// Parse `agent-eval [--file FILE]`.
fn parse_agent_eval<I: Iterator<Item = String>>(mut arguments: I) -> Result<Invocation, String> {
    let mut path = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--file" => {
                path = Some(arguments.next().ok_or(
                    "`--file` needs a JSONL path, e.g. `--file share/evals/agent/basic.jsonl`",
                )?);
            }
            other => return Err(format!("unknown option `{other}` for `agent-eval`")),
        }
    }
    Ok(Invocation::AgentEval(AgentEvalRequest { path }))
}

/// Check one case's expectation against the result envelope.
fn check_expectation(
    expect: &serde_json::Value,
    envelope: &stkcraft_agent_runtime::ResultEnvelope,
) -> Vec<String> {
    let mut problems = Vec::new();
    if let Some(status) = expect.get("status").and_then(serde_json::Value::as_str)
        && status != envelope.status
    {
        problems.push(format!("status `{}`, expected `{status}`", envelope.status));
    }
    if let Some(kind) = expect.get("kind").and_then(serde_json::Value::as_str)
        && !envelope
            .results
            .iter()
            .any(|result| result.get("kind").and_then(serde_json::Value::as_str) == Some(kind))
    {
        problems.push(format!("no result of kind `{kind}`"));
    }
    if let Some(minimum) = expect
        .get("min_windows")
        .and_then(serde_json::Value::as_u64)
    {
        let windows = envelope
            .results
            .iter()
            .filter_map(|result| result.get("pairs").and_then(serde_json::Value::as_array))
            .flatten()
            .filter_map(|pair| pair.get("windows").and_then(serde_json::Value::as_array))
            .map(Vec::len)
            .sum::<usize>()
            + envelope
                .results
                .iter()
                .filter_map(|result| result.get("passes").and_then(serde_json::Value::as_array))
                .map(Vec::len)
                .sum::<usize>();
        if windows < minimum as usize {
            problems.push(format!("{windows} window(s), expected at least {minimum}"));
        }
    }
    if let Some(formula) = expect.get("formula").and_then(serde_json::Value::as_str)
        && !envelope.results.iter().any(|result| {
            result
                .get("formulas")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|formulas| {
                    formulas.iter().any(|entry| {
                        entry.get("id").and_then(serde_json::Value::as_str) == Some(formula)
                    })
                })
        })
    {
        problems.push(format!("no citation of formula `{formula}`"));
    }
    problems
}

/// Run the fixed cases against the configured provider; this is a real network run.
fn agent_eval(request: &AgentEvalRequest) -> Result<(), String> {
    let path = request
        .path
        .clone()
        .unwrap_or_else(|| "share/evals/agent/basic.jsonl".to_owned());
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("could not read `{path}`: {error}"))?;
    let mut total = 0_usize;
    let mut failures = 0_usize;
    for (line_number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let case: serde_json::Value = serde_json::from_str(line)
            .map_err(|error| format!("{path}:{}: {error}", line_number + 1))?;
        let id = case
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("{path}:{}: case has no id", line_number + 1))?;
        let question = case
            .get("question")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("{path}:{}: case `{id}` has no question", line_number + 1))?;
        let expect = case
            .get("expect")
            .ok_or_else(|| format!("{path}:{}: case `{id}` has no expect", line_number + 1))?;
        total += 1;
        let model = crate::prefs::model_provider()?;
        let server = stkcraft_agent_mcp::Server::new(
            app_demo_scenario()?,
            stkcraft_agent_mcp::load_skills(&skills_directory()),
        )
        .with_knowledge(knowledge_library()?)
        .with_capabilities(load_capabilities()?)
        .with_limits(load_limits()?);
        let mut agent = stkcraft_agent_runtime::Agent::new(model, server);
        let mut trace = |line: &str| eprintln!("  · {line}");
        match agent.ask_observing(question, &mut trace) {
            Ok(answer) => {
                let envelope = answer.envelope(question);
                let problems = check_expectation(expect, &envelope);
                if problems.is_empty() {
                    println!("PASS {id} ({})", envelope.status);
                } else {
                    failures += 1;
                    println!("FAIL {id} ({}): {}", envelope.status, problems.join("; "));
                }
            }
            Err(message) => {
                failures += 1;
                println!("FAIL {id}: {message}");
            }
        }
    }
    if failures == 0 {
        println!("agent-eval: {total} case(s), all passed");
        Ok(())
    } else {
        Err(format!("agent-eval: {failures}/{total} case(s) failed"))
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn agent_eval_expectations_check_status_kind_windows_and_formula() {
        let envelope = stkcraft_agent_runtime::ResultEnvelope {
            status: "complete".to_owned(),
            question: "q".to_owned(),
            results: vec![serde_json::json!({
                "kind": "access",
                "pairs": [{"windows": [{"start_utc": "a"}, {"start_utc": "b"}]}],
                "formulas": [{"id": "access.look-angles", "source": "Vallado"}]
            })],
            trace: Vec::new(),
            errors: Vec::new(),
            reason: None,
        };
        let expect = serde_json::json!({
            "status": "complete",
            "kind": "access",
            "min_windows": 2,
            "formula": "access.look-angles"
        });
        assert_eq!(check_expectation(&expect, &envelope), Vec::<String>::new());
        let expect = serde_json::json!({
            "status": "unsupported",
            "kind": "planet-passes",
            "min_windows": 3,
            "formula": "access.window-sampling"
        });
        let problems = check_expectation(&expect, &envelope);
        assert!(problems.iter().any(|problem| problem.contains("status")));
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("planet-passes"))
        );
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("at least 3"))
        );
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("window-sampling"))
        );
    }

    #[test]
    fn model_protocol_flags_are_parsed_without_a_credential() {
        let result = parse_model(
            ["--protocol", "deepseek", "--thinking", "enabled"]
                .into_iter()
                .map(str::to_owned),
        );
        let Ok(Invocation::Model(request)) = result else {
            panic!("protocol options were not accepted");
        };
        assert_eq!(request.protocol.as_deref(), Some("deepseek"));
        assert_eq!(request.thinking.as_deref(), Some("enabled"));
        for options in [
            vec!["--protocol", "typo"],
            vec!["--thinking", "turbo"],
            vec!["--thinking"],
        ] {
            assert!(parse_model(options.into_iter().map(str::to_owned)).is_err());
        }
    }

    #[test]
    fn a_run_is_recorded_with_its_provenance_and_its_windows() {
        // The property the record exists for: months later, a quoted number traces back to the
        // command, the engine version and the digest of the inputs it ran on.
        let directory = std::env::temp_dir().join("stkcraft-cli-record");
        let _ = std::fs::remove_dir_all(&directory);
        let paths = stkcraft_framework_storage::Paths::at(&directory);
        let windows = vec![stkcraft_framework_storage::AccessWindowRecord {
            satellite: "ISS".to_owned(),
            station: "London".to_owned(),
            start_utc: "2024-01-01T09:12:04Z".to_owned(),
            end_utc: "2024-01-01T09:18:16Z".to_owned(),
            start_seconds: 372.0,
            duration_seconds: 372.0,
            minimum_elevation_deg: 5.0,
        }];
        record_run_at(
            &paths,
            &RunRecord {
                command: "access.compute",
                parameters: "{\"step-seconds\":30}",
                inputs: "1 25544U ...2 25544 ...London",
                summary: "Computed access: 1 pair(s), 1 window(s)",
                result: "{\"kind\":\"access\"}",
                errors: &[],
                windows: &windows,
            },
        )
        .expect("recorded");

        let store = stkcraft_framework_storage::Store::open(&paths.record()).expect("a record");
        let computations = store.computations(5).expect("read");
        assert_eq!(computations.len(), 1);
        let computation = &computations[0];
        assert_eq!(computation.record.command, "access.compute");
        assert_eq!(computation.record.engine_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(computation.record.result, "{\"kind\":\"access\"}");
        assert_eq!(computation.record.errors, "[]");
        assert_eq!(computation.windows, 1);
        // The digest is of the inputs as given, so two runs of the same inputs share it and a
        // different scenario does not.
        assert_eq!(
            computation.record.inputs_digest,
            stkcraft_framework_storage::digest(b"1 25544U ...2 25544 ...London")
        );
        let stored = store.windows(computation.id).expect("read");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].start_utc, "2024-01-01T09:12:04Z");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_recorded_instant_is_utc_text() {
        let epoch = stkcraft_framework_time::Epoch::from_jd(
            2_460_311.5,
            stkcraft_framework_time::TimeScale::Tai,
        );
        let text = utc_text(epoch, 0.0);
        assert!(text.ends_with('Z'), "{text}");
        assert_eq!(text.len(), 20, "{text}");
    }
}
