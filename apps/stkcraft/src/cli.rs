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
    /// One satellite against a station on another body, printed as a pass table.
    Planet(PlanetRequest),
    /// Serve the command registry to an agent over standard input and output.
    Mcp,
    /// Ask a model a question; it answers by running the engine's commands.
    Ask(String),
    /// Ask questions one after another, in one conversation, on standard input.
    Chat,
    /// List the extension packages installed for this person.
    Extensions,
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
        Invocation::Chat => chat(),
        Invocation::Extensions => crate::extensions::list(),
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
    extensions  List the extension packages installed for this person
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
        "chat" => Ok(Invocation::Chat),
        "extensions" => Ok(Invocation::Extensions),
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
fn instant_at(epoch: stkcraft_time::Epoch, offset_seconds: f64) -> stkcraft_time::Epoch {
    stkcraft_time::Epoch::from_julian(epoch.julian().plus_seconds(offset_seconds), epoch.scale())
}

/// Serve the engine's commands over the Model Context Protocol on stdin/stdout.
///
/// The skills directory is looked for beside the executable first and then in the working
/// directory, because a deployment copies the binary and its documents together while a developer
/// runs from the repository root. Absent skills are not an error: the tools still work.
fn mcp() -> Result<(), String> {
    let skills = skills_directory();
    let mut server =
        stkcraft_mcp::Server::new(app_demo_scenario()?, stkcraft_mcp::load_skills(&skills))
            .with_knowledge(knowledge_library()?);
    server.serve()
}

/// Ask `DeepSeek` a question, with the engine's commands as its tools and the skills as its library.
///
/// The model needs `DEEPSEEK_API_KEY`; without one the message says so instead of failing inside a
/// request. Every tool call it makes is printed as it happens, because an analysis whose steps a
/// person cannot see is an analysis nobody can check.
fn ask(question: &str) -> Result<(), String> {
    let model = stkcraft_agent::DeepSeek::from_environment()?;
    let server = stkcraft_mcp::Server::new(
        app_demo_scenario()?,
        stkcraft_mcp::load_skills(&skills_directory()),
    )
    .with_knowledge(knowledge_library()?);
    let mut agent = stkcraft_agent::Agent::new(model, server);
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
fn chat() -> Result<(), String> {
    let model = stkcraft_agent::DeepSeek::from_environment()?;
    let server = stkcraft_mcp::Server::new(
        app_demo_scenario()?,
        stkcraft_mcp::load_skills(&skills_directory()),
    )
    .with_knowledge(knowledge_library()?);
    let mut agent = stkcraft_agent::Agent::new(model, server);
    println!(
        "STKCraft {} — ask about the scenario; `exit` ends the conversation.",
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
pub fn skills_directory() -> std::path::PathBuf {
    let beside_executable = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("skills")));
    if let Some(path) = beside_executable.filter(|path| path.is_dir()) {
        return path;
    }
    std::path::PathBuf::from("skills")
}

/// The user's knowledge base: `--knowledge <dir>`, or `STKCRAFT_KNOWLEDGE`, or the configuration
/// directory's `knowledge/`.
///
/// A `--knowledge` path that does not exist is an error with the path in it: a person who pointed the
/// platform at their documents deserves to know it read nothing, rather than getting answers that
/// silently came from memory.
fn knowledge_library() -> Result<stkcraft_knowledge::Library, String> {
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
        return Ok(stkcraft_knowledge::Library::default());
    }
    stkcraft_knowledge::Library::open(&directory)
}

/// Run the `knowledge` subcommand: list the library, or search it.
fn knowledge(request: &KnowledgeRequest) -> Result<(), String> {
    let library = knowledge_library()?;
    match &request.query {
        None => {
            println!(
                "STKCraft {} — knowledge base at {}",
                env!("CARGO_PKG_VERSION"),
                library.root().display()
            );
            if library.is_empty() {
                println!("  no documents: drop text, markdown or CSV files in that directory");
                return Ok(());
            }
            println!(
                "  {} document(s), {} passage(s)",
                library.documents().len(),
                library.passages()
            );
            for document in library.documents() {
                println!(
                    "  {} ({} byte(s), {} passage(s))",
                    document.path,
                    document.bytes,
                    document.chunks.len()
                );
            }
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
fn app_demo_scenario() -> Result<stkcraft_engine::Scenario, String> {
    crate::app::demo_scenario()
}
