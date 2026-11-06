//! The `DeepSeek` agent: a model driving the engine through its own tools.
//!
//! This crate is the other half of the MCP server. `stkcraft-mcp` lets a client *outside* the
//! process — Claude Desktop, a local harness — drive the engine; this one lets the engine drive
//! itself from a prompt, with `DeepSeek` as the model, and it is the same surface either way: the
//! tools are the command registry's, executed through [`stkcraft_mcp::Server`], and the skills are
//! the same markdown documents.
//!
//! # The loop
//!
//! 1. The system prompt carries the mission, the tool discipline and the *names* of the skills.
//! 2. The model answers with text, tool calls, or both.
//! 3. Every tool call runs through the MCP server — the same JSON-RPC path a remote client uses, so
//!    an agent bug is a protocol bug and shows up in the MCP tests too.
//! 4. Tool results go back as messages and the loop repeats, bounded by [`MAX_TURNS`].
//! 5. The final text is the answer.
//!
//! # Why skills are loaded lazily
//!
//! A system prompt that carried every skill would be long, expensive and mostly irrelevant to the
//! question. The model is told which skills exist and calls [`SKILL_TOOL`] when it wants one — the
//! same "progressive disclosure" an MCP client gets from `resources/list` and `prompts/get`.
//!
//! # Talking to `DeepSeek` without an HTTP dependency
//!
//! The transport is `curl` on standard input, which is already how this repository fetches its
//! bulk data: no TLS stack in the build, no HTTP client to audit, and the request and response are
//! text a person can read when something goes wrong. [`ChatModel`] is a trait precisely so the
//! tests never touch the network — a model that answers from a script exercises the whole loop.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

use std::fmt::Write as _;
use std::io::Write as _;

use serde_json::{Value, json};
use stkcraft_mcp::Server;

/// How many model turns one question may take before the loop stops.
///
/// A tool loop that does not converge is a bug in the prompt or a model that is lost, and both are
/// better reported than paid for indefinitely.
pub const MAX_TURNS: usize = 12;

/// How many messages a conversation keeps before the oldest turn is dropped, system prompt aside.
///
/// A conversation that grows without bound is a conversation that eventually fails on a context
/// limit, halfway through an analysis, with the answer already computed in the part that was cut.
/// Twenty turns is far more than a session needs and far less than a limit anyone will hit.
pub const MAX_HISTORY: usize = 40;

/// The tool the model calls to read a skill in full.
pub const SKILL_TOOL: &str = "skill.read";

/// The `DeepSeek` endpoint the default configuration points at.
pub const DEFAULT_BASE_URL: &str = "https://api.deepseek.com/v1/chat/completions";

/// The model used when the caller does not name one.
pub const DEFAULT_MODEL: &str = "deepseek-chat";

/// One message in the conversation the model sees.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// The instruction the model starts from.
    System(String),
    /// What the person asked.
    User(String),
    /// What the model said.
    Assistant {
        /// Its text, which may be empty when it only called tools.
        text: String,
        /// The calls it asked for.
        calls: Vec<ToolCall>,
    },
    /// What a tool answered.
    Tool {
        /// The identifier of the call being answered.
        call_id: String,
        /// The tool's output.
        content: String,
    },
}

/// One tool call the model asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// The provider's identifier for this call, echoed back with the result.
    pub id: String,
    /// The tool name, which is an engine command id.
    pub name: String,
    /// Its arguments, as JSON.
    pub arguments: Value,
}

/// What one model turn produced.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Turn {
    /// The text of the answer, possibly empty.
    pub text: String,
    /// The tool calls to run before the next turn.
    pub calls: Vec<ToolCall>,
}

/// Something that can answer a conversation.
///
/// The trait exists so the loop is testable without a network: a model that answers from a script
/// runs the same code path as the real one, and a transport failure is an error the loop reports
/// rather than a panic.
pub trait ChatModel {
    /// Answer the conversation so far.
    ///
    /// # Errors
    ///
    /// Whatever the transport failed at, as a message for the caller.
    fn complete(&self, messages: &[Message], tools: &[Value]) -> Result<Turn, String>;
}

/// `DeepSeek`'s chat-completions API, spoken over `curl`.
#[derive(Debug, Clone)]
pub struct DeepSeek {
    /// The endpoint.
    pub base_url: String,
    /// The model name.
    pub model: String,
    /// The credential, read from the environment rather than stored in a document.
    pub api_key: String,
}

impl DeepSeek {
    /// Read the configuration from `DEEPSEEK_API_KEY`, with the endpoint and model overridable by
    /// `DEEPSEEK_BASE_URL` and `DEEPSEEK_MODEL`.
    ///
    /// # Errors
    ///
    /// A message naming the missing variable when there is no key: a model client that silently
    /// does nothing is worse than one that says why it cannot start.
    pub fn from_environment() -> Result<Self, String> {
        let Ok(api_key) = std::env::var("DEEPSEEK_API_KEY") else {
            return Err(
                "DEEPSEEK_API_KEY is not set; export it to give the agent a model".to_owned(),
            );
        };
        Ok(Self {
            base_url: std::env::var("DEEPSEEK_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned()),
            model: std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_owned()),
            api_key,
        })
    }

    /// The request body for a conversation.
    fn body(&self, messages: &[Message], tools: &[Value]) -> Value {
        let mut converted = Vec::with_capacity(messages.len());
        for message in messages {
            converted.push(match message {
                Message::System(text) => json!({"role": "system", "content": text}),
                Message::User(text) => json!({"role": "user", "content": text}),
                Message::Assistant { text, calls } => {
                    if calls.is_empty() {
                        json!({"role": "assistant", "content": text})
                    } else {
                        json!({
                            "role": "assistant",
                            "content": text,
                            "tool_calls": calls.iter().map(|call| json!({
                                "id": call.id,
                                "type": "function",
                                "function": {
                                    "name": call.name,
                                    "arguments": call.arguments.to_string(),
                                },
                            })).collect::<Vec<_>>(),
                        })
                    }
                }
                Message::Tool { call_id, content } => json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": content,
                }),
            });
        }
        json!({
            "model": self.model,
            "messages": converted,
            "tools": tools.iter().map(|tool| json!({
                "type": "function",
                "function": {
                    "name": tool["name"],
                    "description": tool["description"],
                    "parameters": tool["inputSchema"],
                },
            })).collect::<Vec<_>>(),
            "tool_choice": "auto",
            "temperature": 0.0,
        })
    }
}

impl ChatModel for DeepSeek {
    fn complete(&self, messages: &[Message], tools: &[Value]) -> Result<Turn, String> {
        let body = self.body(messages, tools).to_string();
        let mut child = std::process::Command::new("curl")
            .args([
                "-sS",
                "--fail",
                "--max-time",
                "120",
                "-X",
                "POST",
                &self.base_url,
                "-H",
                "Content-Type: application/json",
                "-H",
            ])
            .arg(format!("Authorization: Bearer {}", self.api_key))
            .arg("--data-binary")
            .arg("@-")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| format!("could not run curl: {error}"))?;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(body.as_bytes())
                .map_err(|error| format!("could not send the request: {error}"))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|error| format!("the model request did not finish: {error}"))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            return Err(format!("DeepSeek refused the request: {}", detail.trim()));
        }
        parse_response(&String::from_utf8_lossy(&output.stdout))
    }
}

/// Turn a chat-completions response into a [`Turn`].
///
/// # Errors
///
/// A message quoting the response when it is not the shape the API documents: a malformed answer
/// is a bug to see, not a reason to pretend the model said nothing.
pub fn parse_response(text: &str) -> Result<Turn, String> {
    let response: Value = serde_json::from_str(text)
        .map_err(|error| format!("the model's answer is not JSON: {error}"))?;
    if let Some(error) = response.get("error") {
        return Err(format!("the model reported an error: {error}"));
    }
    let message = response
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .ok_or_else(|| {
            format!(
                "the answer has no message: {}",
                &text[..text.len().min(200)]
            )
        })?;
    let mut turn = Turn {
        text: message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        calls: Vec::new(),
    };
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let function = call.get("function").cloned().unwrap_or(Value::Null);
            let arguments = function
                .get("arguments")
                .and_then(Value::as_str)
                .map_or_else(
                    || json!({}),
                    |raw| serde_json::from_str(raw).unwrap_or_else(|_| json!({})),
                );
            turn.calls.push(ToolCall {
                id: call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                name: function
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                arguments,
            });
        }
    }
    Ok(turn)
}

/// What one run of the agent did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Answer {
    /// The model's final text.
    pub text: String,
    /// Every tool call it made, in order, as `name(args) → summary` for a person reading the log.
    pub trace: Vec<String>,
}

/// The agent: a model, the engine's tools and the skills.
pub struct Agent<M: ChatModel> {
    model: M,
    server: Server,
    /// How many turns one question may take.
    pub max_turns: usize,
    /// The conversation so far, including the system prompt once it exists.
    ///
    /// Kept across questions so a follow-up can refer to the previous answer: "and the day after?"
    /// is the natural way to ask, and an agent that forgot everything between questions would force
    /// every question to restate the scenario it already established.
    messages: Vec<Message>,
}

impl<M: ChatModel> Agent<M> {
    /// Build an agent over a session.
    #[must_use]
    pub fn new(model: M, server: Server) -> Self {
        Self {
            model,
            server,
            max_turns: MAX_TURNS,
            messages: Vec::new(),
        }
    }

    /// How many messages the conversation holds, system prompt included.
    ///
    /// Exposed for the panel, which shows a session length, and for tests, which assert that a
    /// follow-up carries the earlier turns.
    #[must_use]
    pub fn history_len(&self) -> usize {
        self.messages.len()
    }

    /// The session, for a caller that wants to read the scenario afterwards.
    #[must_use]
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// The session, for a caller that has to hand the agent the current document.
    pub fn server_mut(&mut self) -> &mut Server {
        &mut self.server
    }

    /// The instruction the model starts from.
    ///
    /// Written as rules rather than as a description: a model that knows it must not invent a pass,
    /// and that every number comes from a tool, behaves differently from one that is only told what
    /// the software does.
    fn system_prompt(&mut self) -> String {
        let mut prompt = String::from(
            "You are STKCraft's analysis assistant. You answer questions about spacecraft, ground \
             stations and the bodies of the solar system over time.\n\
             \n\
             Rules:\n\
             1. Every number you report must come from a tool call. Never estimate an orbit, a pass \
             time or a distance yourself.\n\
             2. Call the tools in the order a mission analyst would: add the satellite and the \
             station first, then compute.\n\
             3. If a tool reports an error, correct the parameter and retry once; if it fails \
             again, report the error instead of inventing a result.\n\
             4. When the answer is a list of passes, give the count and the total time in view, \
             and quote the times as the tool returned them.\n\
            5. If the question needs a procedure you do not know, read the skill for it first.\n\
             \n",
        );
        let mut skill_names = Vec::new();
        for resource in self.server_resource_list() {
            if let Some(name) = resource.strip_prefix("stkcraft://skills/") {
                skill_names.push(name.to_owned());
            }
        }
        if skill_names.is_empty() {
            prompt.push_str("No skills are installed.\n");
        } else {
            let _ = writeln!(
                prompt,
                "Skills available (read one with `{SKILL_TOOL}` when a job needs it): {}.",
                skill_names.join(", ")
            );
        }
        prompt
    }

    /// The skill names, from the server's own resource list.
    fn server_resource_list(&mut self) -> Vec<String> {
        let request = json!({"jsonrpc": "2.0", "id": 0, "method": "resources/list"});
        self.server
            .handle(&request)
            .and_then(|answer| {
                answer
                    .get("result")
                    .and_then(|result| result.get("resources"))
                    .and_then(Value::as_array)
                    .cloned()
            })
            .unwrap_or_default()
            .into_iter()
            .filter_map(|resource| resource.get("uri")?.as_str().map(str::to_owned))
            .collect()
    }

    /// Run one question to an answer.
    ///
    /// # Errors
    ///
    /// A message when the model fails, when a tool fails in a way the model cannot correct, or when
    /// the loop reaches [`Self::max_turns`] without a final answer.
    pub fn ask(&mut self, question: &str) -> Result<Answer, String> {
        self.ask_observing(question, &mut |_| {})
    }

    /// Run one question, reporting each tool call as it happens.
    ///
    /// The observer exists because a question can take a minute and a half: a caller that only
    /// learned the trace at the end would show a spinner and nothing else, and an analysis whose
    /// steps a person cannot follow while it runs is one they cannot interrupt either. The trace in
    /// the [`Answer`] is the same sequence, so a caller may use one, the other, or both.
    ///
    /// # Errors
    ///
    /// As [`Self::ask`].
    pub fn ask_observing<F: FnMut(&str)>(
        &mut self,
        question: &str,
        observer: &mut F,
    ) -> Result<Answer, String> {
        if self.messages.is_empty() {
            let system = self.system_prompt();
            self.messages.push(Message::System(system));
        }
        self.messages.push(Message::User(question.to_owned()));
        self.trim_history();

        let mut answer = Answer::default();
        let tools = self.tools_with_skill_reader();
        for _ in 0..self.max_turns {
            let turn = self.model.complete(&self.messages, &tools)?;
            if turn.calls.is_empty() {
                answer.text.clone_from(&turn.text);
                self.messages.push(Message::Assistant {
                    text: turn.text,
                    calls: Vec::new(),
                });
                return Ok(answer);
            }
            self.messages.push(Message::Assistant {
                text: turn.text.clone(),
                calls: turn.calls.clone(),
            });
            for call in turn.calls {
                let (content, summary) = self.run_tool(&call);
                observer(&summary);
                answer.trace.push(summary);
                self.messages.push(Message::Tool {
                    call_id: call.id,
                    content,
                });
            }
        }
        Err(format!(
            "the model did not finish within {} turns; the trace is {}",
            self.max_turns,
            answer.trace.join("; ")
        ))
    }

    /// Drop the oldest turns when the conversation outgrows [`MAX_HISTORY`].
    ///
    /// A turn is cut at a user message, so a tool result is never separated from the call that
    /// produced it: a conversation trimmed in the middle of a tool exchange is a request the model
    /// API may reject, and the failure would arrive long after the cause.
    fn trim_history(&mut self) {
        while self.messages.len() > MAX_HISTORY {
            let users: Vec<usize> = self
                .messages
                .iter()
                .enumerate()
                .filter(|(_, message)| matches!(message, Message::User(_)))
                .map(|(index, _)| index)
                .collect();
            let (Some(&first), Some(&second)) = (users.first(), users.get(1)) else {
                // One turn, or none: there is nothing older to drop.
                return;
            };
            // A turn starts at a user message; the system prompt is the only thing before it and
            // must stay, so a first index of zero would mean the invariant had already broken.
            if first == 0 || second <= first {
                return;
            }
            self.messages.drain(first..second);
        }
    }

    /// Forget the conversation, keeping the system prompt.
    ///
    /// A session that has gone wrong should be recoverable without restarting the process: the
    /// scenario lives in the server, so the tools still know everything the document says.
    pub fn forget(&mut self) {
        self.messages
            .retain(|message| matches!(message, Message::System(_)));
    }

    /// The engine's tools plus the skill reader.
    fn tools_with_skill_reader(&self) -> Vec<Value> {
        let mut tools = self.server.tools_json();
        tools.push(json!({
            "name": SKILL_TOOL,
            "description": "Read a skill — a procedure for a job — in full. Call it before \
                            attempting a job whose steps you are unsure of.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "The skill name from the list."}
                },
                "required": ["name"],
            },
        }));
        tools
    }

    /// Execute one tool call, returning the content for the model and a line for the trace.
    fn run_tool(&mut self, call: &ToolCall) -> (String, String) {
        if call.name == SKILL_TOOL {
            let name = call
                .arguments
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let request = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "resources/read",
                "params": {"uri": format!("stkcraft://skills/{name}")},
            });
            let content = self
                .server
                .handle(&request)
                .and_then(|answer| {
                    answer
                        .get("result")
                        .and_then(|result| result.get("contents"))
                        .and_then(|contents| contents.get(0))
                        .and_then(|first| first.get("text"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| format!("no skill named `{name}`"));
            return (content, format!("{SKILL_TOOL}({name})"));
        }
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": call.name, "arguments": call.arguments},
        });
        match self.server.handle(&request) {
            Some(answer) if answer.get("error").is_none() => {
                let text = answer
                    .get("result")
                    .and_then(|result| result.get("content"))
                    .and_then(|content| content.get(0))
                    .and_then(|first| first.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let summary = format!(
                    "{}({}) → {}",
                    call.name,
                    call.arguments,
                    text.lines().next().unwrap_or_default()
                );
                (text, summary)
            }
            Some(answer) => {
                let message = answer
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("the tool failed")
                    .to_owned();
                (
                    format!("error: {message}"),
                    format!("{}({}) → error: {message}", call.name, call.arguments),
                )
            }
            None => (
                "error: no answer from the tool".to_owned(),
                format!("{}({}) → no answer", call.name, call.arguments),
            ),
        }
    }
}

#[cfg(test)]
#[expect(clippy::expect_used)]
#[allow(clippy::indexing_slicing, clippy::panic)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use stkcraft_engine::Scenario;

    /// A model that answers from a script, so the loop is exercised without a network.
    struct Script {
        turns: RefCell<Vec<Turn>>,
    }

    impl ChatModel for Script {
        fn complete(&self, _messages: &[Message], _tools: &[Value]) -> Result<Turn, String> {
            let mut turns = self.turns.borrow_mut();
            if turns.is_empty() {
                return Ok(Turn::default());
            }
            Ok(turns.remove(0))
        }
    }

    fn agent(turns: Vec<Turn>) -> Agent<Script> {
        let server = Server::new(Scenario::empty("agent test"), Vec::new());
        Agent::new(
            Script {
                turns: RefCell::new(turns),
            },
            server,
        )
    }

    #[test]
    fn the_observer_sees_each_tool_call_as_it_happens() {
        // A caller that shows progress needs the lines while the loop runs, not afterwards: the
        // same sequence arrives in `answer.trace`, in the same order.
        let mut agent = agent(vec![
            Turn {
                text: String::new(),
                calls: vec![
                    ToolCall {
                        id: "call-1".to_owned(),
                        name: "scenario.rename".to_owned(),
                        arguments: json!({"name": "Gemini"}),
                    },
                    ToolCall {
                        id: "call-2".to_owned(),
                        name: "skill.read".to_owned(),
                        arguments: json!({"name": "compute-passes"}),
                    },
                ],
            },
            Turn {
                text: "Done.".to_owned(),
                calls: Vec::new(),
            },
        ]);
        let mut seen = Vec::new();
        let answer = agent
            .ask_observing("Rename it", &mut |line| seen.push(line.to_owned()))
            .expect("an answer");
        assert_eq!(seen, answer.trace);
        assert_eq!(seen.len(), 2);
        assert!(seen[0].starts_with("scenario.rename("), "{}", seen[0]);
        assert_eq!(seen[1], "skill.read(compute-passes)");
    }

    #[test]
    fn a_follow_up_carries_the_earlier_turns() {
        // The point of a conversation: "and the day after?" must arrive with the scenario the first
        // question built, or every follow-up has to restate the whole job.
        struct Recorder {
            turns: RefCell<Vec<Turn>>,
            seen: RefCell<Vec<Vec<Message>>>,
        }

        impl ChatModel for Recorder {
            fn complete(&self, messages: &[Message], _tools: &[Value]) -> Result<Turn, String> {
                self.seen.borrow_mut().push(messages.to_vec());
                let mut turns = self.turns.borrow_mut();
                if turns.is_empty() {
                    return Ok(Turn::default());
                }
                Ok(turns.remove(0))
            }
        }

        let server = Server::new(Scenario::empty("history test"), Vec::new());
        let mut agent = Agent::new(
            Recorder {
                turns: RefCell::new(vec![
                    Turn {
                        text: String::new(),
                        calls: vec![ToolCall {
                            id: "call-1".to_owned(),
                            name: "scenario.rename".to_owned(),
                            arguments: json!({"name": "Gemini"}),
                        }],
                    },
                    Turn {
                        text: "Renamed.".to_owned(),
                        calls: Vec::new(),
                    },
                    Turn {
                        text: "It is still called Gemini.".to_owned(),
                        calls: Vec::new(),
                    },
                ]),
                seen: RefCell::new(Vec::new()),
            },
            server,
        );
        agent.ask("Rename it to Gemini").expect("the first answer");
        agent
            .ask("What is it called?")
            .expect("the follow-up answer");

        let requests = agent.model.seen.borrow();
        assert_eq!(requests.len(), 3);
        let follow_up = &requests[2];
        let roles: Vec<&str> = follow_up
            .iter()
            .map(|message| match message {
                Message::System(_) => "system",
                Message::User(_) => "user",
                Message::Assistant { .. } => "assistant",
                Message::Tool { .. } => "tool",
            })
            .collect();
        // system, the first question, its tool call, the tool result, its answer, the follow-up.
        assert_eq!(
            roles,
            ["system", "user", "assistant", "tool", "assistant", "user"]
        );
        let tool_content = follow_up
            .iter()
            .find_map(|message| match message {
                Message::Tool { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .expect("the first turn's tool result");
        assert!(tool_content.contains("Gemini"), "{tool_content}");
    }

    #[test]
    fn the_history_is_trimmed_at_a_turn_boundary() {
        // Trimming inside a tool exchange would send a tool result whose call was gone, which a
        // model API rejects — with an error that names nothing a reader could act on.
        let server = Server::new(Scenario::empty("history test"), Vec::new());
        let turns = (0..MAX_HISTORY)
            .map(|index| Turn {
                text: format!("answer {index}"),
                calls: Vec::new(),
            })
            .collect();
        let mut agent = Agent::new(
            Script {
                turns: RefCell::new(turns),
            },
            server,
        );
        for index in 0..(MAX_HISTORY + 4) {
            agent.ask(&format!("question {index}")).expect("an answer");
        }
        assert!(
            agent.history_len() <= MAX_HISTORY + 2,
            "the conversation grew to {} messages",
            agent.history_len()
        );
        // The system prompt is still first, and every request that follows is a user message.
        match agent.messages.first() {
            Some(Message::System(_)) => {}
            other => panic!("the conversation does not start with the system prompt: {other:?}"),
        }
        agent.forget();
        assert_eq!(agent.history_len(), 1);
    }

    #[test]
    fn a_plain_answer_needs_no_tools() {
        let mut agent = agent(vec![Turn {
            text: "I can compute passes.".to_owned(),
            calls: Vec::new(),
        }]);
        let answer = agent.ask("What can you do?").expect("an answer");
        assert_eq!(answer.text, "I can compute passes.");
        assert_eq!(answer.trace, Vec::<String>::new());
    }

    #[test]
    fn a_tool_call_runs_through_the_mcp_server_and_the_answer_comes_back() {
        // The property that matters: the model's tool call is executed by the same JSON-RPC path a
        // remote client uses, so an agent cannot drift from what MCP clients see.
        let mut agent = agent(vec![
            Turn {
                text: String::new(),
                calls: vec![ToolCall {
                    id: "call-1".to_owned(),
                    name: "scenario.rename".to_owned(),
                    arguments: json!({"name": "Gemini"}),
                }],
            },
            Turn {
                text: "Renamed.".to_owned(),
                calls: Vec::new(),
            },
        ]);
        let answer = agent
            .ask("Rename the scenario to Gemini")
            .expect("an answer");
        assert_eq!(answer.text, "Renamed.");
        assert_eq!(agent.server().scenario().name, "Gemini");
        assert_eq!(answer.trace.len(), 1);
        assert!(
            answer.trace[0].starts_with("scenario.rename"),
            "{}",
            answer.trace[0]
        );
    }

    #[test]
    fn a_failing_tool_is_reported_to_the_model_as_text_not_a_panic() {
        // A model that sends a bad parameter must be able to see the error and correct it; the
        // loop must survive it.
        let mut agent = agent(vec![
            Turn {
                text: String::new(),
                calls: vec![ToolCall {
                    id: "call-1".to_owned(),
                    name: "satellite.add".to_owned(),
                    arguments: json!({"name": "broken"}),
                }],
            },
            Turn {
                text: "I could not add it.".to_owned(),
                calls: Vec::new(),
            },
        ]);
        let answer = agent.ask("add a broken satellite").expect("an answer");
        assert_eq!(answer.text, "I could not add it.");
        assert!(answer.trace[0].contains("error"), "{}", answer.trace[0]);
    }

    #[test]
    fn an_empty_script_ends_the_loop_rather_than_spinning() {
        // A model that answers nothing at all is a degenerate turn: the loop takes it as the final
        // answer (empty text) instead of calling the model max_turns times.
        let mut agent = agent(vec![]);
        let answer = agent.ask("anything").expect("an empty answer");
        assert_eq!(answer.text, "");
    }

    #[test]
    fn a_tool_loop_that_never_finishes_is_stopped_and_says_so() {
        struct Endless;
        impl ChatModel for Endless {
            fn complete(&self, _messages: &[Message], _tools: &[Value]) -> Result<Turn, String> {
                Ok(Turn {
                    text: String::new(),
                    calls: vec![ToolCall {
                        id: "x".to_owned(),
                        name: "scenario.rename".to_owned(),
                        arguments: json!({"name": "again"}),
                    }],
                })
            }
        }
        let server = Server::new(Scenario::empty("t"), Vec::new());
        let mut agent = Agent::new(Endless, server);
        agent.max_turns = 3;
        let error = agent.ask("loop").expect_err("the loop must stop");
        assert!(error.contains("3 turns"), "{error}");
    }

    #[test]
    fn the_system_prompt_names_the_rules_and_the_skills() {
        let skills = vec![stkcraft_mcp::Skill::parse(
            "compute-passes",
            "# Compute passes\n\nHow to do it.\n",
        )];
        let server = Server::new(Scenario::empty("t"), skills);
        let mut agent = Agent::new(
            Script {
                turns: RefCell::new(Vec::new()),
            },
            server,
        );
        let prompt = agent.system_prompt();
        assert!(prompt.contains("Every number you report must come from a tool call"));
        assert!(prompt.contains("compute-passes"));
        assert!(prompt.contains(SKILL_TOOL));
    }

    #[test]
    fn the_request_body_maps_tools_and_messages_to_the_documented_shape() {
        let deepseek = DeepSeek {
            base_url: "https://example.invalid".to_owned(),
            model: "deepseek-chat".to_owned(),
            api_key: "k".to_owned(),
        };
        let tools = vec![json!({
            "name": "access.compute",
            "description": "Compute passes",
            "inputSchema": {"type": "object"},
        })];
        let messages = vec![
            Message::System("rules".to_owned()),
            Message::User("when".to_owned()),
            Message::Assistant {
                text: String::new(),
                calls: vec![ToolCall {
                    id: "c1".to_owned(),
                    name: "access.compute".to_owned(),
                    arguments: json!({"step-seconds": 60}),
                }],
            },
            Message::Tool {
                call_id: "c1".to_owned(),
                content: "5 windows".to_owned(),
            },
        ];
        let body = deepseek.body(&messages, &tools);
        assert_eq!(body["model"], "deepseek-chat");
        assert_eq!(body["tools"][0]["function"]["name"], "access.compute");
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
        assert_eq!(
            body["messages"][2]["tool_calls"][0]["function"]["name"],
            "access.compute"
        );
        // The arguments are a JSON-encoded string in the API, not an object.
        assert_eq!(
            body["messages"][2]["tool_calls"][0]["function"]["arguments"],
            "{\"step-seconds\":60}"
        );
        assert_eq!(body["messages"][3]["role"], "tool");
        assert_eq!(body["messages"][3]["tool_call_id"], "c1");
    }

    #[test]
    fn a_response_is_parsed_into_text_and_calls() {
        let turn = parse_response(
            r#"{"choices":[{"message":{"content":"ok","tool_calls":[
                {"id":"c1","type":"function","function":{"name":"access.compute","arguments":"{\"step-seconds\":60}"}}
            ]}}]}"#,
        )
        .expect("a valid response");
        assert_eq!(turn.text, "ok");
        assert_eq!(turn.calls[0].name, "access.compute");
        assert_eq!(turn.calls[0].arguments["step-seconds"], 60);
    }

    #[test]
    fn a_malformed_response_is_an_error_that_quotes_it() {
        assert!(parse_response("not json").is_err());
        assert!(parse_response(r#"{"error":{"message":"bad key"}}"#).is_err());
        assert!(parse_response(r#"{"choices":[]}"#).is_err());
    }

    #[test]
    fn a_missing_key_is_reported_by_name() {
        // The message must say which variable a caller has to set. (The environment is read, not
        // modified: `remove_var` is unsafe in this toolchain and removing another test's key would
        // be a race.)
        let message = match DeepSeek::from_environment() {
            Ok(config) => format!("a key is set for {} ", config.model),
            Err(error) => error,
        };
        if message.starts_with("a key is set") {
            eprintln!(
                "DEEPSEEK_API_KEY is set in this environment; the missing-key path is not exercised"
            );
        } else {
            assert!(message.contains("DEEPSEEK_API_KEY"), "{message}");
        }
    }
}
