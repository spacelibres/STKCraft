//! The agent interface: `STKCraft` as an MCP server.
//!
//! The engine's command registry is already the single door every transport goes through — a menu
//! click, a keystroke, the CLI. This crate adds the last transport and the one that matters most
//! for an agent-drivable tool: the Model Context Protocol, so a large language model can ask
//! "what can you do?", run those commands, and read the methods and documents it needs.
//!
//! # Why the model is not in the kernel
//!
//! A physics kernel that calls a language model would be a kernel that cannot be tested without a
//! network, cannot be deterministic, and cannot answer why a number is what it is. So the model
//! lives *outside*, on the other side of this protocol, and the kernel's job is to make itself
//! completely usable from there: every command is a tool with a described schema, every skill is a
//! prompt with the procedure it teaches, and the scenario is a resource an agent can read and
//! write. The engine stays deterministic and the model stays replaceable — swapping GPT for a local
//! model changes nothing in the kernel.
//!
//! # Shape of the server
//!
//! JSON-RPC 2.0 over standard input and output, one message per line, as the MCP specification for
//! a stdio transport requires. [`Server::handle`] is the whole protocol and it is a pure function of
//! a request and the session state, which is what makes it testable without a pipe: the transport
//! loop in [`Server::serve`] only reads lines, hands them over and writes the answers.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

use std::path::Path;

use std::fmt::Write as _;

use serde_json::{Value, json};
use stkcraft_astro_engine::commands::{Parameter, ParameterKind, Registry};
use stkcraft_astro_engine::scenario::Scenario;
use stkcraft_astro_engine::{PlanetPass, SatelliteAccess};
use stkcraft_framework_time::{Epoch, julian_to_utc_naive};

/// The MCP revision this server implements.
///
/// Pinned rather than echoed: a client that asks for a newer revision gets this one in the
/// handshake and can decide for itself whether the difference matters.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// A skill: a procedure written for a reader that will act on it.
///
/// Skills are the half of the interface that a schema cannot carry — *how* to approach a job, in
/// what order, and what the traps are. They are files, not code, so one can be written, reviewed
/// and corrected without touching the kernel.
#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    /// The directory name, which is also the prompt name.
    pub name: String,
    /// A one-line summary, from the first paragraph of the file.
    pub description: String,
    /// The whole document.
    pub content: String,
}

impl Skill {
    /// Parse a `SKILL.md`.
    ///
    /// The format is deliberately plain: the first heading is the title and the first paragraph
    /// after it is the description. Nothing here depends on YAML, so a skill is markdown and
    /// nothing else.
    #[must_use]
    pub fn parse(name: &str, text: &str) -> Self {
        let mut description = String::new();
        let mut seen_heading = false;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                seen_heading = true;
                continue;
            }
            if trimmed.is_empty() {
                if !description.is_empty() {
                    break;
                }
                continue;
            }
            if seen_heading {
                if !description.is_empty() {
                    description.push(' ');
                }
                description.push_str(trimmed);
            }
        }
        Self {
            name: name.to_owned(),
            description,
            content: text.to_owned(),
        }
    }
}

/// Every skill in a directory, in name order.
///
/// A directory that does not exist is no skills rather than an error: a deployment that ships the
/// binary alone still has tools, and saying so is more useful than refusing to start.
#[must_use]
pub fn load_skills(directory: &Path) -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut skills: Vec<Skill> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(entry.path().join("SKILL.md")).ok()?;
            Some(Skill::parse(&name, &text))
        })
        .collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// The session an MCP client drives: a scenario, the command registry and the skills.
pub struct Server {
    registry: Registry,
    scenario: Scenario,
    skills: Vec<Skill>,
    /// The user's own documents, searched before the model answers.
    knowledge: stkcraft_agent_knowledge::Library,
}

impl Server {
    /// Build a server over a scenario.
    #[must_use]
    pub fn new(scenario: Scenario, skills: Vec<Skill>) -> Self {
        Self {
            registry: Registry::builtin(),
            scenario,
            skills,
            knowledge: stkcraft_agent_knowledge::Library::default(),
        }
    }

    /// Serve a knowledge base as well: the person's own documents, searched before a model answers.
    ///
    /// A separate setter rather than another constructor argument because a session without a
    /// library is the normal case — an empty library answers every search with "nothing here", which
    /// is the truth — and two constructors would fork a call that has one meaning.
    #[must_use]
    pub fn with_knowledge(mut self, knowledge: stkcraft_agent_knowledge::Library) -> Self {
        self.knowledge = knowledge;
        self
    }

    /// The knowledge base this session searches.
    #[must_use]
    pub fn knowledge(&self) -> &stkcraft_agent_knowledge::Library {
        &self.knowledge
    }

    /// The scenario the tools act on.
    #[must_use]
    pub fn scenario(&self) -> &Scenario {
        &self.scenario
    }

    /// Replace the scenario the tools act on.
    ///
    /// The document belongs to the window or to the client that loaded it, not to the session that
    /// happens to be serving it, so a session has to be able to take the current document: a model
    /// that kept answering against the scenario it started with would be answering about a file
    /// nobody has open.
    pub fn set_scenario(&mut self, scenario: Scenario) {
        self.scenario = scenario;
    }

    /// Answer one JSON-RPC message.
    ///
    /// Returns `None` for a notification, which by definition has no response.
    #[must_use]
    pub fn handle(&mut self, request: &Value) -> Option<Value> {
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let id = request.get("id").cloned();
        // A notification carries no `id`: it is a statement, not a question.
        let id = id?;
        let params = request.get("params").cloned().unwrap_or(Value::Null);

        let answer = match self.dispatch(method, &params) {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": code, "message": message},
            }),
        };
        Some(answer)
    }

    /// The protocol itself, without the JSON-RPC envelope.
    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {
                    "tools": {"listChanged": false},
                    "resources": {"listChanged": false},
                    "prompts": {"listChanged": false},
                },
                "serverInfo": {"name": "stkcraft", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "STKCraft answers questions about spacecraft, ground stations and \
                                 the bodies of the solar system over time. Every command is a \
                                 tool; the skills under `stkcraft://skills/` describe how to \
                                 approach the common jobs.",
            })),
            "ping" => Ok(json!({})),
            "tools/list" => {
                let mut tools = self.tools_json();
                tools.extend(self.knowledge_tools());
                Ok(json!({"tools": tools}))
            }
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or((-32602, "tools/call needs a name".to_owned()))?;
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                // The knowledge tools are the server's, not the registry's: a search reads the
                // person's documents and changes nothing.
                if name.starts_with("knowledge.") {
                    return self
                        .call_knowledge_tool(name, &arguments)
                        .map(|(text, structured)| {
                            let mut result = json!({
                                "content": [{"type": "text", "text": text}],
                                "isError": false,
                            });
                            if let (Some(structured), Some(object)) =
                                (structured, result.as_object_mut())
                            {
                                object.insert("structuredContent".to_owned(), structured);
                            }
                            result
                        })
                        .map_err(|message| (-32000, message));
                }
                self.call_tool(name, &arguments)
                    .map(|(text, structured)| {
                        let mut result = json!({
                            "content": [{"type": "text", "text": text}],
                            "isError": false,
                        });
                        // `structuredContent` is the machine-readable half: a client that only
                        // shows the summary loses the pass times, and a model that has to parse
                        // prose to find a number will eventually find the wrong one.
                        if let Some(structured) = structured
                            && let Some(object) = result.as_object_mut()
                        {
                            object.insert("structuredContent".to_owned(), structured);
                        }
                        result
                    })
                    .map_err(|message| (-32000, message))
            }
            "resources/list" => Ok(json!({"resources": self.resources()})),
            "resources/read" => {
                let uri = params
                    .get("uri")
                    .and_then(Value::as_str)
                    .ok_or((-32602, "resources/read needs a uri".to_owned()))?;
                self.read_resource(uri)
                    .map(|(mime, text)| {
                        json!({"contents": [{"uri": uri, "mimeType": mime, "text": text}]})
                    })
                    .map_err(|message| (-32002, message))
            }
            "prompts/list" => Ok(json!({"prompts": self.prompts()})),
            "prompts/get" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or((-32602, "prompts/get needs a name".to_owned()))?;
                self.prompt(name).map_err(|message| (-32602, message))
            }
            other => Err((-32601, format!("unknown method `{other}`"))),
        }
    }

    /// The knowledge-base tools, which are not engine commands.
    ///
    /// A command changes the scenario; a search reads the person's own documents. Putting it in the
    /// registry would make it a menu item, and it is not one.
    #[must_use]
    pub fn knowledge_tools(&self) -> Vec<Value> {
        vec![
            json!({
                "name": "knowledge.search",
                "description": "Search the user's own documents — their requirements, data sheets, \
                                notes — for passages about a question. Call this BEFORE answering \
                                anything factual or numeric, then quote what it returns and cite \
                                it exactly as printed — `path:lines a-b`, or `path:page P, \
                                lines a-b` for a PDF. If it reports that nothing matched, say so \
                                plainly instead of answering from memory.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "What to look for, in the \
                                   words a document would use."},
                        "limit": {"type": "number", "description": "How many passages to return \
                                  at most. Default 5."},
                    },
                    "required": ["query"],
                },
            }),
            json!({
                "name": "knowledge.documents",
                "description": "List the documents in the user's knowledge base, so you know what \
                                it covers before claiming it does not.",
                "inputSchema": {"type": "object", "properties": {}},
            }),
        ]
    }

    /// Every command as an MCP tool, with a schema built from its documented parameters.
    ///
    /// Public because a second client — the `DeepSeek` agent in `stkcraft-agent-runtime` — must build its
    /// function schema from the same source of truth. Two copies of a schema is two things to keep
    /// in step, and the one that falls behind is always the one an agent uses.
    #[must_use]
    pub fn tools_json(&self) -> Vec<Value> {
        self.registry
            .iter()
            .map(|command| {
                let parameters = command.descriptor.parameters;
                let mut properties = serde_json::Map::new();
                let mut required = Vec::new();
                for parameter in parameters {
                    let Parameter {
                        name,
                        description,
                        required: needed,
                        kind,
                    } = parameter;
                    // The type comes from the parameter's own declaration, so the schema a model
                    // reads and the conversion its argument goes through cannot disagree.
                    let schema = match kind {
                        ParameterKind::Text => {
                            json!({"type": "string", "description": description})
                        }
                        ParameterKind::Number => {
                            json!({"type": "number", "description": description})
                        }
                        ParameterKind::Degrees => json!({
                            "type": "number",
                            "description": description,
                        }),
                    };
                    properties.insert((*name).to_owned(), schema);
                    if *needed {
                        required.push(Value::String((*name).to_owned()));
                    }
                }
                json!({
                    "name": command.descriptor.id,
                    "description": format!(
                        "{}. Parameters: {}",
                        command.descriptor.label,
                        parameters
                            .iter()
                            .map(|parameter| parameter.name)
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                    "inputSchema": {
                        "type": "object",
                        "properties": properties,
                        "required": required,
                    },
                })
            })
            .collect()
    }

    /// Run one command by id.
    /// Run a knowledge-base tool.
    fn call_knowledge_tool(
        &mut self,
        name: &str,
        arguments: &Value,
    ) -> Result<(String, Option<Value>), String> {
        match name {
            "knowledge.documents" => {
                let library = &self.knowledge;
                let mut text = format!(
                    "{} document(s), {} passage(s) in `{}`",
                    library.documents().len(),
                    library.passages(),
                    library.root().display()
                );
                if library.is_empty() {
                    text.push_str(
                        "\nThe knowledge base is empty: say so rather than answering from memory.",
                    );
                }
                for document in library.documents() {
                    let _ = write!(
                        text,
                        "\n  {} ({} byte(s), {} passage(s))",
                        document.path,
                        document.bytes,
                        document.chunks.len()
                    );
                }
                let structured = json!({
                    "documents": library
                        .documents()
                        .iter()
                        .map(|document| json!({
                            "path": document.path,
                            "bytes": document.bytes,
                            "passages": document.chunks.len(),
                        }))
                        .collect::<Vec<_>>(),
                });
                Ok((text, Some(structured)))
            }
            "knowledge.search" => {
                let query = arguments
                    .get("query")
                    .and_then(Value::as_str)
                    .ok_or("knowledge.search needs a `query`")?;
                let limit = arguments
                    .get("limit")
                    .and_then(Value::as_f64)
                    .map_or(5, |limit| {
                        // A model may ask for any number; the library answers with a page, not a
                        // document dump.
                        usize::try_from(limit.clamp(1.0, 20.0) as u64).unwrap_or(5)
                    });
                let search = self.knowledge.search(query, limit);
                let mut text = if search.missed {
                    format!(
                        "No passage in the user's documents matches `{query}` \
                         ({} document(s) searched). Say that the knowledge base does not cover \
                         this; do not answer the question from memory as though it did.",
                        search.documents
                    )
                } else {
                    format!(
                        "{} passage(s) from the user's documents, best first. Quote them and cite \
                         each one exactly as it is printed here — `path:lines a-b`, or \
                         `path:page P, lines a-b` for a PDF.",
                        search.hits.len()
                    )
                };
                for hit in &search.hits {
                    let _ = write!(
                        text,
                        "\n\n[{}] score {:.2}\n{}",
                        hit.cite(),
                        hit.score,
                        hit.text
                    );
                }
                // Formulas are cited by their box, because a page has no lines: the sentence that
                // introduces the equation is printed with it, since that is what it solves.
                for hit in &search.formulas {
                    let _ = write!(
                        text,
                        "\n\n[{}] score {:.2} (formula){}\n{}",
                        hit.cite(),
                        hit.score,
                        hit.formula
                            .problem
                            .as_deref()
                            .map(|problem| format!("\nintroduced by: {problem}"))
                            .unwrap_or_default(),
                        hit.formula.unicode
                    );
                }
                let structured = json!({
                    "missed": search.missed,
                    "documents": search.documents,
                    "passages": search.passages,
                    "formulas": search
                        .formulas
                        .iter()
                        .map(|hit| json!({
                            "path": hit.path,
                            "page": hit.formula.page,
                            "box": [hit.formula.x0, hit.formula.y0, hit.formula.x1, hit.formula.y1],
                            "unicode": hit.formula.unicode,
                            "problem": hit.formula.problem,
                            "score": hit.score,
                        }))
                        .collect::<Vec<_>>(),
                    "hits": search
                        .hits
                        .iter()
                        .map(|hit| json!({
                            "path": hit.path,
                            "page": hit.page,
                            "line_start": hit.line_start,
                            "line_end": hit.line_end,
                            "score": hit.score,
                            "text": hit.text,
                        }))
                        .collect::<Vec<_>>(),
                });
                Ok((text, Some(structured)))
            }
            other => Err(format!("no such knowledge tool `{other}`")),
        }
    }

    fn call_tool(
        &mut self,
        name: &str,
        arguments: &Value,
    ) -> Result<(String, Option<Value>), String> {
        let Some(command) = self.registry.get(name) else {
            return Err(format!("no such tool `{name}`; call tools/list"));
        };
        // The conversion lives in the engine so every transport makes it identically: the kind a
        // parameter declares is what decides the typed value a command receives.
        let args = stkcraft_astro_engine::args_from(&command.descriptor, arguments);
        let outcome = self
            .registry
            .run(name, &mut self.scenario, &args)
            .map_err(|error| format!("{name}: {error}"))?;
        let mut text = outcome.summary;
        let mut structured = None;
        if let Some(access) = outcome.access {
            let windows: usize = access.iter().map(|entry| entry.windows.len()).sum();
            let _ = write!(
                text,
                "\n{windows} window(s) over {} pair(s); read `stkcraft://scenario` for the document.",
                access.len()
            );
            let pairs = self.pairs_json(&access, &mut text);
            structured = Some(json!({"kind": "access", "pairs": pairs}));
        }
        if let Some(passes) = outcome.passes {
            let epoch = self.scenario.epoch;
            let _ = writeln!(text);
            let listed = Self::passes_json(&passes, epoch, &mut text);
            structured = Some(json!({
                "kind": "planet-passes",
                "epoch": self.scenario.epoch,
                "passes": listed,
            }));
        }
        Ok((text, structured))
    }

    /// The access windows, as JSON, appending a readable line per window to `text`.
    ///
    /// Readable *and* structured on purpose: the text is what a language model reads and what a
    /// person sees in a client, and the JSON is what a program can trust. The times are given in
    /// UTC and in seconds from the scenario epoch, because a solver reproduces the first and a
    /// spreadsheet wants the second.
    fn pairs_json(&self, access: &[SatelliteAccess], text: &mut String) -> Vec<Value> {
        /// How many windows of one pair the text lists before it summarises the rest.
        const LISTED: usize = 20;
        let epoch = self.scenario.epoch;
        access
            .iter()
            .map(|entry| {
                let satellite = self
                    .scenario
                    .satellite(entry.satellite)
                    .map_or("?", |satellite| satellite.name.as_str());
                let station = self
                    .scenario
                    .station(entry.station)
                    .map_or("?", |station| station.name.as_str());
                let _ = write!(
                    text,
                    "\n{satellite} over {station}: {} pass(es), {} in view",
                    entry.windows.len(),
                    seconds_text(entry.total.as_seconds())
                );
                let mut windows = Vec::with_capacity(entry.windows.len());
                for (index, window) in entry.windows.iter().enumerate() {
                    let start = instant(epoch, window.start.as_seconds());
                    let end = instant(epoch, window.end.as_seconds());
                    windows.push(json!({
                        "start_utc": start,
                        "end_utc": end,
                        "start_seconds": window.start.as_seconds(),
                        "duration_seconds": window.duration().as_seconds(),
                    }));
                    if index < LISTED {
                        let _ = write!(
                            text,
                            "\n  {} → {} UTC ({})",
                            start,
                            end,
                            seconds_text(window.duration().as_seconds())
                        );
                    }
                }
                if entry.windows.len() > LISTED {
                    let _ = write!(
                        text,
                        "\n  … and {} more (all of them are in the structured content)",
                        entry.windows.len() - LISTED
                    );
                }
                json!({
                    "satellite": satellite,
                    "station": station,
                    "total_seconds": entry.total.as_seconds(),
                    "windows": windows,
                })
            })
            .collect()
    }

    /// The passes over a station on another body, as JSON, with a readable line in `text`.
    fn passes_json(passes: &[PlanetPass], epoch: Epoch, text: &mut String) -> Vec<Value> {
        /// How many passes the text lists before it summarises the rest.
        const LISTED: usize = 20;
        let mut listed = Vec::with_capacity(passes.len());
        for (index, pass) in passes.iter().enumerate() {
            let start = instant(epoch, pass.start_seconds);
            let end = instant(epoch, pass.end_seconds);
            listed.push(json!({
                "start_utc": start,
                "end_utc": end,
                "start_seconds": pass.start_seconds,
                "duration_seconds": pass.duration_seconds(),
            }));
            if index < LISTED {
                let _ = write!(
                    text,
                    "\n  {} → {} UTC ({})",
                    start,
                    end,
                    seconds_text(pass.duration_seconds())
                );
            }
        }
        if passes.len() > LISTED {
            let _ = write!(
                text,
                "\n  … and {} more (all of them are in the structured content)",
                passes.len() - LISTED
            );
        }
        listed
    }

    /// What a client can read: the scenario, the command catalogue and every skill.
    fn resources(&self) -> Vec<Value> {
        let mut resources = vec![
            json!({
                "uri": "stkcraft://scenario",
                "name": "Scenario",
                "description": "The current scenario document, as JSON.",
                "mimeType": "application/json",
            }),
            json!({
                "uri": "stkcraft://commands",
                "name": "Command catalogue",
                "description": "Every command the engine knows, with its parameters.",
                "mimeType": "application/json",
            }),
        ];
        for skill in &self.skills {
            resources.push(json!({
                "uri": format!("stkcraft://skills/{}", skill.name),
                "name": skill.name,
                "description": skill.description,
                "mimeType": "text/markdown",
            }));
        }
        resources
    }

    fn read_resource(&self, uri: &str) -> Result<(&'static str, String), String> {
        match uri {
            "stkcraft://scenario" => serde_json::to_string_pretty(&self.scenario)
                .map(|text| ("application/json", text))
                .map_err(|error| format!("the scenario cannot be written as JSON: {error}")),
            "stkcraft://commands" => {
                let catalogue: Vec<Value> = self.tools_json();
                serde_json::to_string_pretty(&catalogue)
                    .map(|text| ("application/json", text))
                    .map_err(|error| format!("the catalogue cannot be written as JSON: {error}"))
            }
            other => self
                .skills
                .iter()
                .find(|skill| other == format!("stkcraft://skills/{}", skill.name))
                .map(|skill| ("text/markdown", skill.content.clone()))
                .ok_or_else(|| format!("no resource `{other}`; call resources/list")),
        }
    }

    /// Skills as prompts, so a client that only understands prompts still finds them.
    fn prompts(&self) -> Vec<Value> {
        self.skills
            .iter()
            .map(|skill| {
                json!({
                    "name": skill.name,
                    "description": skill.description,
                })
            })
            .collect()
    }

    fn prompt(&self, name: &str) -> Result<Value, String> {
        let skill = self
            .skills
            .iter()
            .find(|skill| skill.name == name)
            .ok_or_else(|| format!("no prompt `{name}`; call prompts/list"))?;
        Ok(json!({
            "description": skill.description,
            "messages": [{
                "role": "user",
                "content": {"type": "text", "text": skill.content},
            }],
        }))
    }

    /// Serve the protocol over standard input and output until the stream ends.
    ///
    /// One JSON message per line, as the stdio transport specifies. A malformed line is answered
    /// with a JSON-RPC parse error and the loop continues: a client that sends one bad message
    /// should not lose its session.
    ///
    /// # Errors
    ///
    /// A message only when standard input cannot be read at all, which is a broken pipe rather
    /// than a protocol problem.
    pub fn serve(&mut self) -> Result<(), String> {
        use std::io::{BufRead, Write};
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        for line in stdin.lock().lines() {
            let line = line.map_err(|error| format!("could not read standard input: {error}"))?;
            if line.trim().is_empty() {
                continue;
            }
            let request: Value = match serde_json::from_str(&line) {
                Ok(value) => value,
                Err(error) => {
                    let answer = json!({
                        "jsonrpc": "2.0",
                        "id": Value::Null,
                        "error": {"code": -32700, "message": format!("parse error: {error}")},
                    });
                    let _ = writeln!(stdout, "{answer}");
                    let _ = stdout.flush();
                    continue;
                }
            };
            if let Some(answer) = self.handle(&request) {
                let _ = writeln!(stdout, "{answer}");
                let _ = stdout.flush();
            }
        }
        Ok(())
    }
}

/// An instant as `YYYY-MM-DD HH:MM:SS`, in UTC.
///
/// Falls back to the Julian date rather than to a placeholder: an epoch outside the leap-second
/// table is a fact about the request, and a number a caller can look up beats an empty string.
fn instant(epoch: Epoch, offset_seconds: f64) -> String {
    let shifted = Epoch::from_julian(epoch.julian().plus_seconds(offset_seconds), epoch.scale());
    match shifted
        .to_utc()
        .ok()
        .and_then(|utc| julian_to_utc_naive(utc.julian()))
    {
        Some(datetime) => datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => format!("JD {:.6}", shifted.julian().sum()),
    }
}

/// A duration as `6m 12s`, rounded to the second.
fn seconds_text(seconds: f64) -> String {
    if !seconds.is_finite() {
        return "n/a".to_owned();
    }
    let total = seconds.round().max(0.0) as u64;
    let hours = total / 3_600;
    let minutes = (total % 3_600) / 60;
    let remainder = total % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m {remainder:02}s")
    } else {
        format!("{minutes}m {remainder:02}s")
    }
}

/// Turn a JSON argument into the engine's [`Arg`], in the type the parameter declares.
///
/// The parameter's [`ParameterKind`] decides, not its name and not what the caller happened to send:
/// a latitude arrives as `51.5` and becomes a typed `Angle`, and a body name arrives as `"Mars"` and
/// becomes text. Getting this wrong is not cosmetic — an angle the command never receives is a
/// command that answers "expected an angle" to a caller who sent exactly the right number.
#[cfg(test)]
#[expect(clippy::expect_used)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;
    use serde_json::json;

    /// One real element set, so a test computes a real orbit rather than an error path.
    const LEO_L1: &str = "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9009";
    const LEO_L2: &str = "2 25544  51.6400 208.9163 0006317  69.9862  25.2906 15.49569600 12343";

    fn server() -> Server {
        let skills = vec![
            Skill::parse(
                "compute-passes",
                "# Compute passes\n\nHow to answer when can X see Y.\n",
            ),
            Skill::parse(
                "planet-access",
                "# Planet access\n\nStations on other bodies.\n",
            ),
        ];
        Server::new(Scenario::empty("mcp test"), skills)
    }

    fn ask(server: &mut Server, method: &str, params: &Value) -> Value {
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        server.handle(&request).expect("a request has an answer")
    }

    #[test]
    fn the_handshake_names_the_protocol_and_the_capabilities() {
        let mut server = server();
        let answer = ask(&mut server, "initialize", &json!({}));
        let result = &answer["result"];
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert!(result["capabilities"]["tools"].is_object());
        assert!(result["capabilities"]["resources"].is_object());
        assert_eq!(result["serverInfo"]["name"], "stkcraft");
    }

    #[test]
    fn every_engine_command_appears_as_a_tool_with_a_schema() {
        // The property that makes the engine agent-drivable: nothing a user can do is missing from
        // the tool list, because the list *is* the registry.
        let mut server = server();
        let answer = ask(&mut server, "tools/list", &json!({}));
        let tools = answer["result"]["tools"].as_array().expect("an array");
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert!(names.contains(&"access.compute"));
        assert!(names.contains(&"satellite.add"));
        assert!(names.contains(&"planet.access"));
        let planet = tools
            .iter()
            .find(|tool| tool["name"] == "planet.access")
            .expect("planet.access is a tool");
        assert_eq!(planet["inputSchema"]["type"], "object");
        assert!(planet["inputSchema"]["properties"]["body"].is_object());
    }

    #[test]
    fn a_tool_call_runs_through_the_registry() {
        // A rename is a command with one parameter; calling it through the protocol must change
        // the scenario, not merely answer politely.
        let mut server = server();
        let answer = ask(
            &mut server,
            "tools/call",
            &json!({"name": "scenario.rename", "arguments": {"name": "Mars mission"}}),
        );
        assert!(answer.get("error").is_none(), "{answer}");
        assert_eq!(server.scenario().name, "Mars mission");
        let text = answer["result"]["content"][0]["text"]
            .as_str()
            .expect("text");
        assert!(text.contains("Mars mission"), "{text}");
    }

    #[test]
    fn an_access_call_answers_with_the_windows_and_not_only_a_count() {
        // The failure this pins: a tool that says "3 windows" leaves the caller to recompute the
        // times, which is the whole job of a tool call. Every window in the structured content must
        // also be in the text, because the text is what a model reads.
        let mut server = server();
        for call in [
            json!({"name": "satellite.add", "arguments": {
                "name": "ISS", "line1": LEO_L1, "line2": LEO_L2}}),
            json!({"name": "station.add", "arguments": {
                "name": "London", "latitude": 51.5, "longitude": -0.13,
                "minimum-elevation": 0.0}}),
        ] {
            let answer = ask(&mut server, "tools/call", &call);
            assert!(answer.get("error").is_none(), "{answer}");
        }
        let answer = ask(
            &mut server,
            "tools/call",
            &json!({"name": "access.compute", "arguments": {"step-seconds": 30.0}}),
        );
        assert!(answer.get("error").is_none(), "{answer}");
        let text = answer["result"]["content"][0]["text"]
            .as_str()
            .expect("text");
        let pairs = answer["result"]["structuredContent"]["pairs"]
            .as_array()
            .expect("pairs");
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0]["satellite"], "ISS");
        assert_eq!(pairs[0]["station"], "London");
        let windows = pairs[0]["windows"].as_array().expect("windows");
        assert!(
            !windows.is_empty(),
            "no passes in 24 h of a 92-minute orbit"
        );
        let first = windows[0]["start_utc"].as_str().expect("a start time");
        // A calendar instant, not a bare offset: a caller cannot place "372.5 s" on a timeline.
        assert!(first.contains(':') && first.contains('-'), "{first}");
        assert!(text.contains(first), "{text}");
        let duration = windows[0]["duration_seconds"].as_f64().expect("a duration");
        assert!(duration > 0.0, "{duration}");
    }

    #[test]
    fn a_planet_call_answers_with_its_pass_times() {
        let mut server = server();
        let answer = ask(
            &mut server,
            "tools/call",
            &json!({"name": "planet.access", "arguments": {
                "body": "Titan", "radius-km": 3000.0, "inclination-deg": 0.0,
                "latitude-deg": 10.0, "longitude-deg": 0.0, "mask-deg": 10.0, "hours": 12.0}}),
        );
        assert!(answer.get("error").is_none(), "{answer}");
        let structured = &answer["result"]["structuredContent"];
        assert_eq!(structured["kind"], "planet-passes");
        let passes = structured["passes"].as_array().expect("passes");
        assert!(!passes.is_empty(), "no passes in 12 h over Titan");
        let text = answer["result"]["content"][0]["text"]
            .as_str()
            .expect("text");
        let first = passes[0]["start_utc"].as_str().expect("a start time");
        assert!(text.contains(first), "{text}");
        // The passes are in time order and do not overlap, which is what makes the list usable.
        assert!(
            passes[0]["start_seconds"].as_f64().expect("a number")
                <= passes[0]["duration_seconds"].as_f64().expect("a number"),
            "the first pass starts before the search window"
        );
    }

    /// A library over two hand-written documents, so the knowledge tools are tested without files.
    fn library() -> stkcraft_agent_knowledge::Library {
        stkcraft_agent_knowledge::Library::from_documents(
            std::path::Path::new("/knowledge"),
            vec![
                stkcraft_agent_knowledge::Document {
                    formulas: Vec::new(),
                    path: "sensor-spec.md".to_owned(),
                    bytes: 80,
                    pages: None,
                    limited: false,
                    chunks: stkcraft_agent_knowledge::chunk(
                        "Sensor\n\nMaximum off-nadir angle is 45 degrees.",
                    ),
                },
                stkcraft_agent_knowledge::Document {
                    formulas: Vec::new(),
                    path: "thermal.txt".to_owned(),
                    bytes: 40,
                    pages: None,
                    limited: false,
                    chunks: stkcraft_agent_knowledge::chunk(
                        "Thermal\n\nRadiator area 2.4 square metres.",
                    ),
                },
            ],
        )
    }

    #[test]
    fn the_knowledge_tools_are_listed_with_the_engine_tools() {
        let mut server = server().with_knowledge(library());
        let answer = ask(&mut server, "tools/list", &json!({}));
        let names: Vec<&str> = answer["result"]["tools"]
            .as_array()
            .expect("an array")
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert!(names.contains(&"knowledge.search"), "{names:?}");
        assert!(names.contains(&"knowledge.documents"), "{names:?}");
        // The engine's own tools are still there: the knowledge base is an addition, not a
        // replacement.
        assert!(names.contains(&"access.compute"), "{names:?}");
    }

    #[test]
    fn a_search_returns_a_citable_passage() {
        // The property the whole feature exists for: the answer names a document and a line range
        // a person can open.
        let mut server = server().with_knowledge(library());
        let answer = ask(
            &mut server,
            "tools/call",
            &json!({"name": "knowledge.search", "arguments": {"query": "maximum off-nadir angle"}}),
        );
        let text = answer["result"]["content"][0]["text"]
            .as_str()
            .expect("text");
        assert!(text.contains("sensor-spec.md:lines"), "{text}");
        assert!(text.contains("45 degrees"), "{text}");
        let structured = &answer["result"]["structuredContent"];
        assert_eq!(structured["missed"], false);
        assert_eq!(structured["hits"][0]["path"], "sensor-spec.md");
    }

    #[test]
    fn a_search_that_finds_nothing_says_so_instead_of_guessing() {
        let mut server = server().with_knowledge(library());
        let answer = ask(
            &mut server,
            "tools/call",
            &json!({"name": "knowledge.search", "arguments": {"query": "hyperspectral methane"}}),
        );
        let text = answer["result"]["content"][0]["text"]
            .as_str()
            .expect("text");
        assert!(text.contains("No passage"), "{text}");
        assert!(
            text.contains("do not answer the question from memory"),
            "the tool has to say what a miss means: {text}"
        );
        assert_eq!(answer["result"]["structuredContent"]["missed"], true);
    }

    #[test]
    fn an_empty_library_is_reported_as_empty() {
        let mut server = server();
        let answer = ask(
            &mut server,
            "tools/call",
            &json!({"name": "knowledge.documents", "arguments": {}}),
        );
        let text = answer["result"]["content"][0]["text"]
            .as_str()
            .expect("text");
        assert!(text.contains("empty"), "{text}");
    }

    #[test]
    fn an_unknown_tool_is_refused_by_name() {
        let mut server = server();
        let answer = ask(&mut server, "tools/call", &json!({"name": "teleport"}));
        let message = answer["error"]["message"].as_str().expect("a message");
        assert!(message.contains("teleport"), "{message}");
    }

    #[test]
    fn the_scenario_and_the_skills_are_resources_a_client_can_read() {
        let mut server = server();
        let list = ask(&mut server, "resources/list", &json!({}));
        let uris: Vec<&str> = list["result"]["resources"]
            .as_array()
            .expect("an array")
            .iter()
            .filter_map(|resource| resource["uri"].as_str())
            .collect();
        assert!(uris.contains(&"stkcraft://scenario"));
        assert!(uris.contains(&"stkcraft://skills/compute-passes"));

        let scenario = ask(
            &mut server,
            "resources/read",
            &json!({"uri": "stkcraft://scenario"}),
        );
        let text = scenario["result"]["contents"][0]["text"]
            .as_str()
            .expect("text");
        assert!(text.contains("mcp test"), "{text}");

        let skill = ask(
            &mut server,
            "resources/read",
            &json!({"uri": "stkcraft://skills/planet-access"}),
        );
        let text = skill["result"]["contents"][0]["text"]
            .as_str()
            .expect("text");
        assert!(text.contains("Stations on other bodies"), "{text}");
    }

    #[test]
    fn skills_are_prompts_too() {
        let mut server = server();
        let list = ask(&mut server, "prompts/list", &json!({}));
        let names: Vec<&str> = list["result"]["prompts"]
            .as_array()
            .expect("an array")
            .iter()
            .filter_map(|prompt| prompt["name"].as_str())
            .collect();
        assert_eq!(names, ["compute-passes", "planet-access"]);
        let prompt = ask(
            &mut server,
            "prompts/get",
            &json!({"name": "compute-passes"}),
        );
        let text = prompt["result"]["messages"][0]["content"]["text"]
            .as_str()
            .expect("text");
        assert!(text.starts_with("# Compute passes"));
    }

    #[test]
    fn notifications_get_no_answer_and_unknown_methods_get_an_error() {
        let mut server = server();
        let notification = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        assert!(server.handle(&notification).is_none());
        let answer = ask(&mut server, "time/travel", &json!({}));
        assert_eq!(answer["error"]["code"], -32601);
    }

    #[test]
    fn a_skill_description_comes_from_its_first_paragraph() {
        let skill = Skill::parse("x", "# Title\n\nFirst line.\nStill first.\n\nSecond.");
        assert_eq!(skill.description, "First line. Still first.");
        assert_eq!(skill.name, "x");
    }

    #[test]
    fn a_missing_skills_directory_is_no_skills_rather_than_a_failure() {
        // A binary deployed without its documents still has tools; refusing to start would be a
        // worse answer than starting with fewer.
        assert_eq!(
            load_skills(std::path::Path::new("/nonexistent/stkcraft/skills")).len(),
            0
        );
    }
}
