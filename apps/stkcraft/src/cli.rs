//! The command line: argument parsing and dispatch into the engine's command registry.
//!
//! Hand-written rather than generated, because the surface is small and a dependency that
//! cannot be fetched offline would block the build. The shape is deliberate: every subcommand
//! builds a [`Args`] and calls [`Registry::run`] by id, exactly as a menu item or a control
//! channel will. Nothing here reimplements engine behaviour.

use std::fmt::Write as _;

use stkcraft_astro_engine::{Arg, Args, Registry, Scenario};
use stkcraft_astro_sgp4::TwoLineElement;
use stkcraft_framework_core::Angle;

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
    Mcp,
    /// Ask a model a question; it answers by running the engine's commands.
    Ask(String),
    /// Ask questions one after another, in one conversation, on standard input.
    Chat(ChatRequest),
    /// List the extension packages installed for this person.
    Extensions,
    /// Show what this installation has computed, newest first.
    History,
    /// Serve the control channel on a loopback port.
    Control(ControlRequest),
    /// List or search the person's knowledge base.
    Knowledge(KnowledgeRequest),
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
        Invocation::Mcp => mcp(),
        Invocation::Ask(question) => ask(&question),
        Invocation::Chat(request) => chat(&request),
        Invocation::Extensions => crate::extensions::list(),
        Invocation::History => history(),
        Invocation::Control(request) => control(&request),
        Invocation::Knowledge(request) => knowledge(&request),
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
    ask         Ask DeepSeek a question; it answers by running the engine's commands
    chat        Ask questions in one conversation, reading them from standard input
                (`chat --session <id>` resumes one; `chat --sessions` lists them)
    extensions  List the extension packages installed for this person
    history     Show what this installation has computed, newest first
    control     Serve the control channel on loopback (`control --port 7979`)
    knowledge   List the documents in your knowledge base, or search them:
                `stkcraft knowledge [<query>]`
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
        "mcp" => Ok(Invocation::Mcp),
        "chat" => parse_chat(arguments),
        "extensions" => Ok(Invocation::Extensions),
        "history" => Ok(Invocation::History),
        "control" => parse_control(arguments),
        "knowledge" => Ok(Invocation::Knowledge(KnowledgeRequest {
            query: arguments.next(),
        })),
        "ask" => {
            let question = arguments.next().ok_or_else(|| {
                "`ask` needs a question, e.g. `stkcraft ask \"when does the ISS pass over London?\"`"
                    .to_owned()
            })?;
            Ok(Invocation::Ask(question))
        }
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
/// Record what a run computed, so the number can be traced back to its inputs.
///
/// A failure to record is reported on stderr and never fails the run: the analysis already happened,
/// and refusing to print it because a database was busy would be the tail wagging the dog. What is
/// *not* optional is that the record carries the inputs' digest — a row without it cannot be
/// reproduced, and the storage layer refuses to pretend otherwise.
fn record_run(
    command: &str,
    parameters: &str,
    inputs: &str,
    summary: &str,
    windows: &[stkcraft_framework_storage::AccessWindowRecord],
) {
    let Ok(paths) = stkcraft_framework_storage::Paths::for_user() else {
        eprintln!("stkcraft: the run was not recorded: no data directory");
        return;
    };
    if let Err(message) = record_run_at(&paths, command, parameters, inputs, summary, windows) {
        eprintln!("stkcraft: the run was not recorded: {message}");
    }
}

/// [`record_run`] against an explicit data directory, which is what a test uses.
fn record_run_at(
    paths: &stkcraft_framework_storage::Paths,
    command: &str,
    parameters: &str,
    inputs: &str,
    summary: &str,
    windows: &[stkcraft_framework_storage::AccessWindowRecord],
) -> Result<(), String> {
    let mut store = stkcraft_framework_storage::Store::open(&paths.record())
        .map_err(|error| error.to_string())?;
    let id = store
        .record_computation(&stkcraft_framework_storage::Computation {
            scenario_name: "CLI".to_owned(),
            scenario_revision: None,
            command: command.to_owned(),
            parameters: parameters.to_owned(),
            engine_version: env!("CARGO_PKG_VERSION").to_owned(),
            inputs_digest: stkcraft_framework_storage::digest(inputs.as_bytes()),
            summary: summary.to_owned(),
        })
        .map_err(|error| error.to_string())?;
    for window in windows {
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
    record_run(
        "access.compute",
        &format!(
            "{{\"step-seconds\":{},\"hours\":{}}}",
            request.step_seconds, request.hours
        ),
        &format!("{}{}{}", request.line1, request.line2, request.name),
        &outcome.summary,
        &windows,
    );

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
    let mut scenario = Scenario::empty("CLI");
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
fn mcp() -> Result<(), String> {
    let skills = skills_directory();
    let mut server = stkcraft_agent_mcp::Server::new(
        app_demo_scenario()?,
        stkcraft_agent_mcp::load_skills(&skills),
    )
    .with_knowledge(knowledge_library()?);
    server.serve()
}

/// Ask `DeepSeek` a question, with the engine's commands as its tools and the skills as its library.
///
/// The model needs `DEEPSEEK_API_KEY`; without one the message says so instead of failing inside a
/// request. Every tool call it makes is printed as it happens, because an analysis whose steps a
/// person cannot see is an analysis nobody can check.
fn ask(question: &str) -> Result<(), String> {
    let model = stkcraft_agent_runtime::DeepSeek::from_environment()?;
    let server = stkcraft_agent_mcp::Server::new(
        app_demo_scenario()?,
        stkcraft_agent_mcp::load_skills(&skills_directory()),
    )
    .with_knowledge(knowledge_library()?);
    let mut agent = stkcraft_agent_runtime::Agent::new(model, server);
    // Printed as they happen rather than collected and printed at the end: a question can take a
    // minute, and a step nobody can see while it runs is a step nobody can interrupt.
    let mut print = |line: &str| println!("  · {line}");
    match agent.ask_observing(question, &mut print) {
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
    let model = stkcraft_agent_runtime::DeepSeek::from_environment()?;
    let server = stkcraft_agent_mcp::Server::new(
        app_demo_scenario()?,
        stkcraft_agent_mcp::load_skills(&skills_directory()),
    )
    .with_knowledge(knowledge_library()?);
    // A conversation is a session on disk, so closing the terminal does not lose it: `--session`
    // resumes one and the id is printed for next time. This is the same shape a harness keeps its own
    // sessions in — a transcript that outlives the process.
    let session = match &request.session {
        Some(id) => stkcraft_agent_runtime::session::Session::open(&directory, id)?,
        None => stkcraft_agent_runtime::session::Session::create(&directory)?,
    };
    let id = session.id.clone();
    let mut agent = stkcraft_agent_runtime::Agent::new(model, server).with_session(session);
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
    let server = stkcraft_agent_mcp::Server::new(
        app_demo_scenario()?,
        stkcraft_agent_mcp::load_skills(&skills_directory()),
    )
    .with_knowledge(knowledge_library()?);
    let session = stkcraft_agent_control::Control::new(server, stkcraft_agent_control::new_token());
    stkcraft_agent_control::serve(request.port, session)
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

/// The user's knowledge base: `--knowledge <dir>`, or `STKCRAFT_KNOWLEDGE`, or the configuration
/// directory's `knowledge/`.
///
/// A `--knowledge` path that does not exist is an error with the path in it: a person who pointed the
/// platform at their documents deserves to know it read nothing, rather than getting answers that
/// silently came from memory.
fn knowledge_library() -> Result<stkcraft_agent_knowledge::Library, String> {
    Ok(knowledge_opened()?.library)
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

/// The library and what opening it cost, so the command can say whether the cache worked.
fn knowledge_opened() -> Result<stkcraft_agent_knowledge::Opened, String> {
    let explicit = std::env::var("STKCRAFT_KNOWLEDGE")
        .ok()
        .map(std::path::PathBuf::from);
    let directory = match explicit {
        Some(directory) => directory,
        None => crate::prefs::knowledge_dir().ok_or_else(|| {
            "this platform has no configuration directory for a knowledge base".to_owned()
        })?,
    };
    if !directory.is_dir() {
        if std::env::var_os("STKCRAFT_KNOWLEDGE").is_some() {
            return Err(format!(
                "STKCRAFT_KNOWLEDGE points at {}, which is not a directory",
                directory.display()
            ));
        }
        // No library yet is the normal state of a fresh install, and it is not an error.
        return Ok(stkcraft_agent_knowledge::Opened {
            library: stkcraft_agent_knowledge::Library::default(),
            reuse: stkcraft_agent_knowledge::Reuse::default(),
            skipped: Vec::new(),
        });
    }
    // The derived index is cached under the data directory, so a second run over a paper corpus is a
    // directory walk rather than a re-parse of every document.
    let cache = stkcraft_framework_storage::Paths::for_user()
        .ok()
        .map(|paths| stkcraft_framework_storage::Cache::at(paths.cache().join("knowledge")));
    stkcraft_agent_knowledge::Library::open_cached(
        &directory,
        stkcraft_agent_knowledge::Limits::default(),
        cache.as_ref(),
    )
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

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

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
            "access.compute",
            "{\"step-seconds\":30}",
            "1 25544U ...2 25544 ...London",
            "Computed access: 1 pair(s), 1 window(s)",
            &windows,
        )
        .expect("recorded");

        let store = stkcraft_framework_storage::Store::open(&paths.record()).expect("a record");
        let computations = store.computations(5).expect("read");
        assert_eq!(computations.len(), 1);
        let computation = &computations[0];
        assert_eq!(computation.record.command, "access.compute");
        assert_eq!(computation.record.engine_version, env!("CARGO_PKG_VERSION"));
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
