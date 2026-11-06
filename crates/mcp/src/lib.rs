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
use stkcraft_engine::commands::{Arg, Args, Parameter, Registry};
use stkcraft_engine::scenario::Scenario;

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
}

impl Server {
    /// Build a server over a scenario.
    #[must_use]
    pub fn new(scenario: Scenario, skills: Vec<Skill>) -> Self {
        Self {
            registry: Registry::builtin(),
            scenario,
            skills,
        }
    }

    /// The scenario the tools act on.
    #[must_use]
    pub fn scenario(&self) -> &Scenario {
        &self.scenario
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
            "tools/list" => Ok(json!({"tools": self.tools()})),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or((-32602, "tools/call needs a name".to_owned()))?;
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                self.call_tool(name, &arguments)
                    .map(|text| json!({"content": [{"type": "text", "text": text}], "isError": false}))
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

    /// Every command as an MCP tool, with a schema built from its documented parameters.
    fn tools(&self) -> Vec<Value> {
        self.registry
            .iter()
            .map(|command| {
                let parameters = command.descriptor.parameters;
                let mut properties = serde_json::Map::new();
                let mut required = Vec::new();
                for Parameter {
                    name,
                    description,
                    required: needed,
                } in parameters
                {
                    properties.insert(
                        (*name).to_owned(),
                        json!({"type": "number", "description": description}),
                    );
                    if *needed {
                        required.push(Value::String((*name).to_owned()));
                    }
                }
                // Some parameters are text by nature (a body name, a scenario title); the schema
                // says string for those so a model does not send a number the command will refuse.
                for name in TEXT_PARAMETERS {
                    if let Some(property) = properties.get_mut(name) {
                        *property = json!({"type": "string"});
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
    fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<String, String> {
        let Some(command) = self.registry.get(name) else {
            return Err(format!("no such tool `{name}`; call tools/list"));
        };
        let mut args = Args::none();
        for parameter in command.descriptor.parameters {
            let Some(value) = arguments.get(parameter.name) else {
                continue;
            };
            args = args.with(parameter.name, argument_from_json(parameter.name, value));
        }
        let outcome = self
            .registry
            .run(name, &mut self.scenario, &args)
            .map_err(|error| format!("{name}: {error}"))?;
        let mut text = outcome.summary;
        if let Some(access) = outcome.access {
            let windows: usize = access.iter().map(|entry| entry.windows.len()).sum();
            let _ = write!(
                text,
                "\n{windows} window(s) over {} pair(s); read `stkcraft://scenario` for the document.",
                access.len()
            );
        }
        Ok(text)
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
                let catalogue: Vec<Value> = self.tools();
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

/// Parameters that are text rather than numbers.
///
/// The command registry describes parameters by name and prose, not by type, because a menu does
/// not need types. A model does: it will send `"Mars"` for a body and `1.5` for a latitude, and the
/// schema has to say which is which. This list is the bridge, and it is short on purpose.
const TEXT_PARAMETERS: [&str; 6] = ["body", "name", "line1", "line2", "station", "satellite"];

/// Turn a JSON argument into the engine's [`Arg`].
///
/// A number stays a number; anything else becomes text, which is what a command with a text
/// parameter expects and what a wrong type deserves — the command's own validation rejects it with
/// a message that names the parameter.
fn argument_from_json(name: &str, value: &Value) -> Arg {
    if TEXT_PARAMETERS.contains(&name) {
        return Arg::Text(match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        });
    }
    match value {
        Value::Number(number) => Arg::Number(number.as_f64().unwrap_or(f64::NAN)),
        Value::String(text) => match text.parse::<f64>() {
            Ok(number) => Arg::Number(number),
            Err(_) => Arg::Text(text.clone()),
        },
        other => Arg::Text(other.to_string()),
    }
}

#[cfg(test)]
#[expect(clippy::expect_used)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;
    use serde_json::json;

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
