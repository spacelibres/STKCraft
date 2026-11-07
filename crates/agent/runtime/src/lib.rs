//! The model agent: a model driving the engine through its own tools.
//!
//! This crate is the other half of the MCP server. `stkcraft-agent-mcp` lets a client *outside* the
//! process — Claude Desktop, a local harness — drive the engine; this one lets the engine drive
//! itself from a prompt, with the connection the person configured (`DeepSeek` is the default and
//! [`ChatModel`] is a trait, so a different endpoint is a different provider rather than a second
//! loop), and it is the same surface either way: the tools are the command registry's, executed
//! through [`stkcraft_agent_mcp::Server`], and the skills are the same markdown documents.
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
//! # Talking to the model without an HTTP dependency
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

use serde_json::{Value, json};
use stkcraft_agent_mcp::Server;

pub mod control;
pub use control::{CancellationToken, ModelError, RequestControl};
pub mod session;
pub mod translate;
pub mod verify;
pub use verify::verify_results;
#[cfg(not(target_arch = "wasm32"))]
mod transport;
pub mod usage;
pub use usage::{AnswerUsage, CompletionTokensDetails, Usage, UsageAggregate};

/// How many model turns one question may take before the loop stops.
///
/// A tool loop that does not converge is a bug in the prompt or a model that is lost, and both are
/// better reported than paid for indefinitely.
pub const MAX_TURNS: usize = 12;

/// The rule a compaction summary follows.
///
/// It asks for facts and not for prose because the summary is the only record of the turns it
/// replaces: a later question can depend on a number that was in them, and a fluent paragraph that
/// dropped the number has lost the conversation.
pub const COMPACTION_PROMPT: &str = "You are compacting a transcript so it fits a context window. \
Write the summary a careful colleague would need to continue the work: every fact, number, name, \
decision and result that appears, the questions that were asked and what was answered or left open. \
No commentary, no headings, no apologies. Prose in the language the conversation used.";

/// How many messages a conversation keeps before the oldest turn is dropped, system prompt aside.
///
/// A conversation that grows without bound is a conversation that eventually fails on a context
/// limit, halfway through an analysis, with the answer already computed in the part that was cut.
/// Twenty turns is far more than a session needs and far less than a limit anyone will hit.
pub const MAX_HISTORY: usize = 40;

/// The tool the model calls to read a skill in full.
pub const SKILL_TOOL: &str = "skill.read";

/// The tool the model calls to search the user's own documents.
///
/// The order matters more than anything else in this crate: a platform whose answer can be traced to
/// a line of the person's own specification is worth trusting with a mission timeline, and one whose
/// answer came from a model's memory is not. This tool is named in the system prompt as the first
/// thing to do.
pub const KNOWLEDGE_SEARCH: &str = "knowledge.search";

/// The tool the model calls to list the documents it can search.
pub const KNOWLEDGE_DOCUMENTS: &str = "knowledge.documents";

/// The tool the model calls to read the scenario document.
///
/// Without it the agent can change the scenario and cannot see it: asked "how many satellites are
/// there?", a model with only add/remove tools answers that it cannot know, which is honest and
/// useless. The document is already an MCP resource; this is the model's way to reach it.
pub const SCENARIO_TOOL: &str = "scenario.read";

/// The separator a tool name uses where a command id has a dot.
///
/// An `OpenAI`-compatible function name must match `^[a-zA-Z0-9_-]+$`, and every `STKCraft` command id is
/// `namespace.verb`, so `access.compute` is refused by the provider before a single token is
/// generated: `400 Invalid 'tools[0].function.name'`. The ids stay dotted — MCP allows it, and they
/// are the stable public names — so the agent translates at its own boundary.
///
/// A double underscore and not a hyphen: an id may itself contain a hyphen (`scenario.set-duration`),
/// so `a.b-c` and `a-b.c` would both become `a-b-c`. Ids never contain `_`, which makes this
/// injective.
pub const FUNCTION_NAME_SEPARATOR: &str = "__";

/// The name a model sees for a command id: `access.compute` → `access__compute`.
#[must_use]
pub fn function_name(id: &str) -> String {
    id.replace('.', FUNCTION_NAME_SEPARATOR)
}

/// The command id a model's tool name stands for: `access__compute` → `access.compute`.
///
/// A name that is not a translated id is returned as it is, so an unknown tool still reaches the
/// server's own "no such tool" error instead of being silently rewritten into one.
#[must_use]
pub fn command_id(function: &str) -> String {
    function.replace(FUNCTION_NAME_SEPARATOR, ".")
}

/// One message in the conversation the model sees.
///
/// [`Message::Summary`] is what a compaction leaves behind: a *model-written* account of the turns
/// that were dropped, kept where the system prompt was so the model reads it first. It travels to the
/// provider as a second system message, which is the only slot the protocol has for "this is context,
/// not conversation", and it is stored in the transcript so a resumed session starts with the same
/// memory a live one has.
///
/// The same values are what a [`session::Session`] writes to disk, so a conversation survives the
/// process: the transcript is this enum, one JSON object per line.
#[derive(Clone, PartialEq)]
pub enum Message {
    /// The instruction the model starts from.
    System(String),
    /// A summary of turns that were compacted away, written by the model itself.
    Summary(String),
    /// What the person asked.
    User(String),
    /// What the model said.
    Assistant {
        /// Its text, which may be empty when it only called tools.
        text: String,
        /// The calls it asked for.
        calls: Vec<ToolCall>,
        /// Opaque provider state, never displayed or summarized.
        reasoning_content: Option<String>,
        /// Provider-reported usage, retained locally and never replayed on the wire.
        usage: Option<Usage>,
        /// Provider completion status.
        finish_reason: Option<String>,
    },
    /// What a tool answered.
    Tool {
        /// The identifier of the call being answered.
        call_id: String,
        /// The tool's output.
        content: String,
    },
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Assistant { text, calls, .. } => f
                .debug_struct("Assistant")
                .field("text", text)
                .field("calls", calls)
                .finish_non_exhaustive(),
            Self::System(text) => f.debug_tuple("System").field(text).finish(),
            Self::Summary(text) => f.debug_tuple("Summary").field(text).finish(),
            Self::User(text) => f.debug_tuple("User").field(text).finish(),
            Self::Tool { call_id, content } => f
                .debug_struct("Tool")
                .field("call_id", call_id)
                .field("content", content)
                .finish(),
        }
    }
}

/// Saturating Unicode character cost of retained protocol payloads, not a token count.
#[must_use]
pub fn message_characters(message: &Message) -> usize {
    match message {
        Message::System(text) | Message::Summary(text) | Message::User(text) => {
            text.chars().count()
        }
        Message::Assistant {
            text,
            calls,
            reasoning_content,
            ..
        } => calls.iter().fold(
            text.chars().count().saturating_add(
                reasoning_content
                    .as_deref()
                    .map_or(0, |value| value.chars().count()),
            ),
            |cost, call| {
                cost.saturating_add(call.id.chars().count())
                    .saturating_add(call.name.chars().count())
                    .saturating_add(call.arguments.to_string().chars().count())
            },
        ),
        Message::Tool { call_id, content } => call_id
            .chars()
            .count()
            .saturating_add(content.chars().count()),
    }
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
#[derive(Clone, PartialEq, Default)]
pub struct Turn {
    /// The text of the answer, possibly empty.
    pub text: String,
    /// The tool calls to run before the next turn.
    pub calls: Vec<ToolCall>,
    /// Opaque provider state, never displayed or summarized.
    pub reasoning_content: Option<String>,
    /// Provider-reported token counters; absent means unknown.
    pub usage: Option<Usage>,
    /// Completion status; unknown future values are preserved.
    pub finish_reason: Option<String>,
}

impl std::fmt::Debug for Turn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Turn")
            .field("text", &self.text)
            .field("calls", &self.calls)
            .finish_non_exhaustive()
    }
}

/// The explicitly selected wire protocol; a model name never implies capability.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProviderProtocol {
    /// The original generic chat-completions shape.
    #[default]
    OpenAiCompatible,
    /// `DeepSeek`'s documented thinking and reasoning replay extensions.
    DeepSeek,
}

/// Explicit `DeepSeek` thinking selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThinkingMode {
    /// Request thinking mode.
    Enabled,
    /// Request ordinary mode.
    #[default]
    Disabled,
}

/// Credential-free provider protocol options.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProviderOptions {
    /// Wire protocol to use.
    pub protocol: ProviderProtocol,
    /// Thinking selection, meaningful only for `DeepSeek`.
    pub thinking: ThinkingMode,
    /// Optional total output cap, including reasoning tokens.
    pub max_output_tokens: Option<u32>,
}

impl ProviderOptions {
    /// Resolve explicit strings and endpoint defaults, shared by environment and app settings.
    ///
    /// # Errors
    /// Unknown explicit modes, or thinking enabled on the compatible protocol.
    pub fn resolve(
        base_url: &str,
        protocol: Option<&str>,
        thinking: Option<&str>,
    ) -> Result<Self, String> {
        let protocol = match protocol {
            Some("deepseek") => ProviderProtocol::DeepSeek,
            Some("compatible") => ProviderProtocol::OpenAiCompatible,
            Some(_) => return Err("model protocol must be deepseek or compatible".to_owned()),
            None if base_url == DEEPSEEK_BASE_URL => ProviderProtocol::DeepSeek,
            None => ProviderProtocol::OpenAiCompatible,
        };
        let thinking = match thinking {
            Some("enabled") => ThinkingMode::Enabled,
            Some("disabled") | None => ThinkingMode::Disabled,
            Some(_) => return Err("model thinking must be enabled or disabled".to_owned()),
        };
        if protocol == ProviderProtocol::OpenAiCompatible && thinking == ThinkingMode::Enabled {
            return Err("thinking enabled requires the deepseek protocol; select --protocol deepseek or --thinking disabled".to_owned());
        }
        // Application policy, not a provider/model maximum: total output includes reasoning.
        Ok(Self {
            protocol,
            thinking,
            max_output_tokens: (protocol == ProviderProtocol::DeepSeek).then_some(8192),
        })
    }
    /// Override the output cap; absent configuration preserves protocol defaults.
    ///
    /// # Errors
    /// A non-integer, zero or value outside u32.
    pub fn with_max_output_tokens(mut self, value: Option<&str>) -> Result<Self, String> {
        if let Some(value) = value {
            let value = value
                .parse::<u32>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| "max_output_tokens must be a nonzero u32 integer".to_owned())?;
            self.max_output_tokens = Some(value);
        }
        Ok(self)
    }
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
    /// Cooperative adapter; cannot interrupt an in-progress legacy implementation.
    /// # Errors
    /// Cancelled, Timeout or sanitized `ModelFailed`.
    fn complete_controlled(
        &self,
        messages: &[Message],
        tools: &[Value],
        control: &RequestControl,
    ) -> Result<Turn, ModelError> {
        control.check()?;
        let result = self
            .complete(messages, tools)
            .map_err(|_| ModelError::ModelFailed);
        control.check()?;
        result
    }
}

/// A chat-completions endpoint, spoken over `curl`.
///
/// Generic construction preserves the OpenAI-compatible request shape. Provider extensions are
/// selected explicitly through [`ProviderOptions`], independently of the model's name, so a custom
/// gateway is never assumed to support `DeepSeek` reasoning replay.
///
/// Nothing is retried silently and nothing is streamed. Errors report protocol fields or curl's
/// process status without echoing response bodies, which can contain opaque reasoning.
#[derive(Clone)]
pub struct Provider {
    /// The endpoint: an OpenAI-compatible `/chat/completions` URL.
    pub base_url: String,
    /// The model name, as the endpoint spells it.
    pub model: String,
    /// The credential.
    pub api_key: String,
    /// How long one request may take, in seconds.
    pub timeout_seconds: u64,
    /// Explicit wire protocol options.
    pub options: ProviderOptions,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A key in a log line is a key in a bug report. The length is enough to tell "set" from
        // "empty" without leaking anything.
        formatter
            .debug_struct("Provider")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &format_args!("<{} bytes>", self.api_key.len()))
            .field("timeout_seconds", &self.timeout_seconds)
            .field("options", &self.options)
            .finish()
    }
}

/// The default provider, so a person who sets only a key gets a working model.
pub const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com/v1/chat/completions";
/// The default model name.
pub const DEEPSEEK_MODEL: &str = "deepseek-flash";

impl Provider {
    /// A provider with an explicit endpoint, model and credential.
    #[must_use]
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            api_key: api_key.into(),
            timeout_seconds: 120,
            options: ProviderOptions::default(),
        }
    }

    /// Select extensions explicitly; construction itself never infers endpoint capabilities.
    #[must_use]
    pub fn with_options(mut self, options: ProviderOptions) -> Self {
        self.options = options;
        self
    }

    /// Check compatibility before opening a network connection.
    fn validate_history(&self, messages: &[Message]) -> Result<(), String> {
        if self.options.max_output_tokens == Some(0) {
            return Err("max_output_tokens must be nonzero".to_owned());
        }
        if self.options.protocol == ProviderProtocol::OpenAiCompatible
            && self.options.thinking == ThinkingMode::Enabled
        {
            return Err("thinking enabled requires the deepseek protocol".to_owned());
        }
        if self.options.protocol == ProviderProtocol::DeepSeek && self.options.thinking == ThinkingMode::Enabled && messages.iter().any(|message| matches!(message, Message::Assistant { calls, reasoning_content: None, .. } if !calls.is_empty())) {
            return Err("cannot resume DeepSeek thinking with assistant tool calls missing reasoning_content; start a new session or explicitly select --thinking disabled".to_owned());
        }
        Ok(())
    }

    /// Validate response protocol state before handing any calls to the agent's dispatcher.
    fn parse_turn(&self, response: &str) -> Result<Turn, String> {
        let turn = parse_response(response)?;
        if self.options.protocol == ProviderProtocol::DeepSeek
            && self.options.thinking == ThinkingMode::Enabled
            && !turn.calls.is_empty()
            && turn.reasoning_content.is_none()
        {
            return Err("DeepSeek thinking tool response is missing reasoning_content; no tool calls were executed; retry in a new session or explicitly select --thinking disabled".to_owned());
        }
        Ok(turn)
    }

    /// Whether a credential is present at all.
    #[must_use]
    pub fn has_credential(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    /// Read the configuration from `DEEPSEEK_API_KEY`, with the endpoint and model overridable by
    /// `DEEPSEEK_BASE_URL` and `DEEPSEEK_MODEL`. `DEEPSEEK_PROTOCOL` and `DEEPSEEK_THINKING`
    /// select explicit protocol options; custom endpoints default to compatible mode.
    ///
    /// This is the fallback, not the primary path: the application reads the person's settings
    /// first, so a key stored in the settings works with no environment at all.
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
        let base_url =
            std::env::var("DEEPSEEK_BASE_URL").unwrap_or_else(|_| DEEPSEEK_BASE_URL.to_owned());
        let protocol = std::env::var("DEEPSEEK_PROTOCOL").ok();
        let thinking = std::env::var("DEEPSEEK_THINKING").ok();
        let options =
            ProviderOptions::resolve(&base_url, protocol.as_deref(), thinking.as_deref())?
                .with_max_output_tokens(
                    std::env::var("DEEPSEEK_MAX_OUTPUT_TOKENS").ok().as_deref(),
                )?;
        Ok(Self::new(
            base_url,
            std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| DEEPSEEK_MODEL.to_owned()),
            api_key,
        )
        .with_options(options))
    }

    /// The request body for a conversation.
    fn body(&self, messages: &[Message], tools: &[Value]) -> Value {
        let mut converted = Vec::with_capacity(messages.len());
        for message in messages {
            converted.push(match message {
                Message::System(text) => json!({"role": "system", "content": text}),
                // A summary is context, not conversation: it goes in the system slot, phrased so the
                // model cannot mistake it for something the user said.
                Message::Summary(text) => json!({
                    "role": "system",
                    "content": format!("Earlier in this conversation, in summary:\n{text}"),
                }),
                Message::User(text) => json!({"role": "user", "content": text}),
                Message::Assistant {
                    text,
                    calls,
                    reasoning_content,
                    ..
                } => {
                    let mut converted = if calls.is_empty() {
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
                    };
                    if self.options.protocol == ProviderProtocol::DeepSeek
                        && let Some(reasoning) = reasoning_content
                        && let Some(object) = converted.as_object_mut()
                    {
                        object.insert("reasoning_content".to_owned(), json!(reasoning));
                    }
                    converted
                }
                Message::Tool { call_id, content } => json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": content,
                }),
            });
        }
        let mut body = json!({
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
        });
        // DeepSeek API thinking-mode guide, verified 2026-11-07:
        // https://api-docs.deepseek.com/guides/thinking_mode/
        if self.options.protocol == ProviderProtocol::DeepSeek
            && let Some(object) = body.as_object_mut()
        {
            object.insert("thinking".to_owned(), json!({"type": match self.options.thinking { ThinkingMode::Enabled => "enabled", ThinkingMode::Disabled => "disabled" }}));
            if self.options.thinking == ThinkingMode::Enabled {
                object.remove("temperature");
            }
        }
        if let Some(max_tokens) = self.options.max_output_tokens
            && let Some(object) = body.as_object_mut()
        {
            object.insert("max_tokens".to_owned(), json!(max_tokens));
        }
        body
    }
}

impl ChatModel for Provider {
    fn complete(&self, messages: &[Message], tools: &[Value]) -> Result<Turn, String> {
        self.complete_controlled(messages, tools, &RequestControl::default())
            .map_err(|e| e.to_string())
    }
    fn complete_controlled(
        &self,
        messages: &[Message],
        tools: &[Value],
        control: &RequestControl,
    ) -> Result<Turn, ModelError> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (messages, tools, control);
            Err(ModelError::Unsupported)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            control.check()?;
            if self.options.max_output_tokens == Some(0) {
                return Err(ModelError::InvalidOutputLimit);
            }
            if self.options.protocol == ProviderProtocol::OpenAiCompatible
                && self.options.thinking == ThinkingMode::Enabled
            {
                return Err(ModelError::ThinkingProtocol);
            }
            self.validate_history(messages)
                .map_err(|_| ModelError::MissingHistoryReasoning)?;
            let control = control.bounded(std::time::Duration::from_secs(self.timeout_seconds))?;
            let body = self.body(messages, tools);
            control.check()?;
            let config = transport::encode_config(
                &self.base_url,
                &self.api_key,
                &body,
                control.remaining()?,
            )?;
            control.check()?;
            let mut command = std::process::Command::new("curl");
            command.args(["-q", "--config", "-"]);
            let text = transport::run(command, config, &control)?;
            let turn = self.parse_turn(&text).map_err(|_| ModelError::Protocol)?;
            control.check()?;
            Ok(turn)
        }
    }
}

/// Turn a chat-completions response into a [`Turn`].
///
/// # Errors
///
/// A message naming the invalid protocol field, without quoting opaque reasoning payloads.
pub fn parse_response(text: &str) -> Result<Turn, String> {
    let response: Value = serde_json::from_str(text)
        .map_err(|error| format!("the model's answer is not JSON: {error}"))?;
    if response.get("error").is_some() {
        return Err("the model reported an error".to_owned());
    }
    let message = response
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(Value::as_object)
        .ok_or_else(|| "the answer has no message".to_owned())?;
    let finish_reason = match response
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("finish_reason"))
    {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => return Err("finish_reason must be a string or null".to_owned()),
    };
    let usage = match response.get("usage") {
        None | Some(Value::Null) => None,
        Some(value) => Some(serde_json::from_value::<Usage>(value.clone()).map_err(|_| {
            "usage must contain unsigned integer token counters and valid breakdowns".to_owned()
        })?),
    };
    reject_truncated(finish_reason.as_deref())?;
    let reasoning_content = match message.get("reasoning_content") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => return Err("the answer's reasoning_content must be a string or null".to_owned()),
    };
    let mut turn = Turn {
        usage,
        finish_reason,
        reasoning_content,
        text: message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        calls: Vec::new(),
    };
    if let Some(calls) = message.get("tool_calls").filter(|calls| !calls.is_null()) {
        let calls = calls
            .as_array()
            .ok_or_else(|| "the answer's tool_calls must be an array".to_owned())?;
        for (index, call) in calls.iter().enumerate() {
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| format!("tool_calls[{index}].id must be a nonempty string"))?;
            let function = call
                .get("function")
                .and_then(Value::as_object)
                .ok_or_else(|| format!("tool_calls[{index}].function must be an object"))?;
            let name = function
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| {
                    format!("tool_calls[{index}].function.name must be a nonempty string")
                })?;
            let raw = function
                .get("arguments")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("tool_calls[{index}].function.arguments must be a JSON object encoded as a string"))?;
            let arguments: Value = serde_json::from_str(raw).map_err(|error| {
                format!("tool_calls[{index}].function.arguments is invalid JSON: {error}")
            })?;
            if !arguments.is_object() {
                return Err(format!(
                    "tool_calls[{index}].function.arguments must decode to an object"
                ));
            }
            turn.calls.push(ToolCall {
                id: id.to_owned(),
                name: name.to_owned(),
                arguments,
            });
        }
    }
    Ok(turn)
}

/// Refuse incomplete output before tools or final answers are accepted.
///
/// # Errors
/// An explicit error for the provider's output-length termination status.
pub fn reject_truncated(finish_reason: Option<&str>) -> Result<(), String> {
    if finish_reason == Some("length") {
        return Err("model output was truncated (finish_reason=length); no tool calls from this response were executed; increase max_output_tokens and retry".to_owned());
    }
    Ok(())
}

/// What one run of the agent did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Answer {
    /// The model's final text.
    pub text: String,
    /// Every tool call it made, in order, as `name(args) → summary` for a person reading the log.
    pub trace: Vec<String>,
    /// The structured results the tools returned, in call order.
    pub results: Vec<serde_json::Value>,
    /// Reported usage and gaps for this answer and its compaction requests.
    pub usage: AnswerUsage,
}

/// The machine-readable answer the platform renders.
///
/// The model's prose is not the result. A computation's structured content is; the answer's status
/// says whether the platform has one, and the trace names the tools that produced it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ResultEnvelope {
    /// `complete` when at least one structured result exists, `unsupported` otherwise.
    pub status: String,
    /// The question as the caller asked it.
    pub question: String,
    /// The structured results, in call order.
    pub results: Vec<serde_json::Value>,
    /// The tool trace.
    pub trace: Vec<String>,
    /// The invariants a result failed; empty when every result passed.
    pub errors: Vec<String>,
}

impl Answer {
    /// Build the result envelope for this answer.
    #[must_use]
    pub fn envelope(&self, question: &str) -> ResultEnvelope {
        let errors = verify_results(&self.results);
        let status = if self.results.is_empty() {
            "unsupported"
        } else if errors.is_empty() {
            "complete"
        } else {
            "failed"
        };
        ResultEnvelope {
            status: status.to_owned(),
            question: question.to_owned(),
            results: self.results.clone(),
            trace: self.trace.clone(),
            errors,
        }
    }
}

/// The agent: a model, the engine's tools and the skills.
pub struct Agent<M: ChatModel> {
    model: M,
    usage: AnswerUsage,
    server: Server,
    /// How many turns one question may take.
    pub max_turns: usize,
    /// The transcript this conversation is written to and resumed from, when there is one.
    session: Option<crate::session::Session>,
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
            usage: AnswerUsage::default(),
            server,
            max_turns: MAX_TURNS,
            messages: Vec::new(),
            session: None,
        }
    }

    /// Continue a stored conversation, or write a new one to `session`.
    ///
    /// The messages the session already holds become the conversation's past, so a resumed session
    /// answers "and the day after?" with the scenario and the answers of yesterday's questions still
    /// in front of it.
    #[must_use]
    pub fn with_session(mut self, session: crate::session::Session) -> Self {
        self.messages.clone_from(&session.messages);
        self.session = Some(session);
        self
    }

    /// Usage of the latest attempted answer, available even when it failed.
    #[must_use]
    pub fn usage(&self) -> &AnswerUsage {
        &self.usage
    }

    /// The session, when this agent keeps one.
    #[must_use]
    pub fn session(&self) -> Option<&crate::session::Session> {
        self.session.as_ref()
    }

    /// Record a message in the conversation and, when there is one, in its transcript.
    fn push(&mut self, message: Message) {
        if let Some(session) = &mut self.session {
            // A failed write costs the resume, not the answer: it is reported once, by the session,
            // and the conversation continues in memory.
            let _ = session.record(message.clone());
        }
        self.messages.push(message);
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
             1. **Read the user's own documents first.** Before answering any factual or numeric \
             question, call `knowledge.search` with the words the document would use. When it returns \
             passages, quote them and cite each exactly as it is printed — `path:lines a-b`, or \
             `path:page P, lines a-b` for a PDF. When it reports that nothing matched, \
             say so plainly, and only then may you answer from general knowledge — labelled as \
             general knowledge, not as the library's.\n\
             2. Every number you report must come from a tool call or a cited passage. Never \
             estimate an orbit, a pass time or a distance yourself.\n\
             3. Call the tools in the order a mission analyst would: add the satellite and the \
             station first, then compute.\n\
             4. If a tool reports an error, correct the parameter and retry once; if it fails \
             again, report the error instead of inventing a result.\n\
             5. When the answer is a list of passes, give the count and the total time in view, \
             and quote the times as the tool returned them.\n\
            6. If the question needs a procedure you do not know, read the skill for it first.\n\
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
        self.ask_observing_controlled(question, observer, &RequestControl::default())
            .map_err(|error| {
                if error == ModelError::ModelFailed {
                    format!(
                        "the model did not complete within {} turns or failed",
                        self.max_turns
                    )
                } else {
                    error.to_string()
                }
            })
    }
    /// Ask with a shared request control. Tools already executing finish synchronously;
    /// cancelled transcripts may be partial and isolated UI workers must discard them.
    /// # Errors
    /// Typed model or control failure.
    pub fn ask_controlled(
        &mut self,
        question: &str,
        control: &RequestControl,
    ) -> Result<Answer, ModelError> {
        self.ask_observing_controlled(question, &mut |_| {}, control)
    }
    /// Ask with cooperative control and a tool observer.
    /// # Errors
    /// As [`Self::ask_controlled`].
    pub fn ask_observing_controlled<F: FnMut(&str)>(
        &mut self,
        question: &str,
        observer: &mut F,
        control: &RequestControl,
    ) -> Result<Answer, ModelError> {
        control.check()?;
        self.usage = AnswerUsage::default();
        if self.messages.is_empty() {
            let system = self.system_prompt();
            self.push(Message::System(system));
        }
        self.push(Message::User(question.to_owned()));
        self.trim_history();
        self.compact_session_controlled(control)?;
        control.check()?;

        let mut answer = Answer::default();
        let tools = self.tools_with_skill_reader();
        for _ in 0..self.max_turns {
            control.check()?;
            let turn = match self
                .model
                .complete_controlled(&self.messages, &tools, control)
            {
                Ok(turn) => turn,
                Err(error) => {
                    self.usage.model.record(None, true);
                    return Err(error);
                }
            };
            let truncated = reject_truncated(turn.finish_reason.as_deref());
            self.usage
                .model
                .record(turn.usage.as_ref(), truncated.is_err());
            truncated.map_err(|_| ModelError::TruncatedOutput)?;
            control.check()?;
            if turn.calls.is_empty() {
                answer.text.clone_from(&turn.text);
                self.push(Message::Assistant {
                    usage: turn.usage,
                    finish_reason: turn.finish_reason,
                    reasoning_content: turn.reasoning_content,
                    text: turn.text,
                    calls: Vec::new(),
                });
                answer.usage = self.usage.clone();
                control.check()?;
                return Ok(answer);
            }
            self.push(Message::Assistant {
                usage: turn.usage.clone(),
                finish_reason: turn.finish_reason.clone(),
                reasoning_content: turn.reasoning_content.clone(),
                text: turn.text.clone(),
                calls: turn.calls.clone(),
            });
            for call in turn.calls {
                control.check()?;
                let (content, summary, structured) = self.run_tool(&call);
                control.check()?;
                observer(&summary);
                control.check()?;
                if let Some(structured) = structured {
                    answer.results.push(structured);
                }
                answer.trace.push(summary);
                self.push(Message::Tool {
                    call_id: call.id,
                    content,
                });
            }
        }
        Err(ModelError::ModelFailed)
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

    /// Compact the transcript when it outgrows the context budget.
    ///
    /// The oldest turns are dropped at a user message and the transcript gains a note, so the model
    /// knows the conversation has a past it cannot see. This mirrors how a harness keeps a long
    /// session inside a context window: the recent turns stay exact, the old ones stop being sent,
    /// and the record says so rather than pretending the conversation began yesterday.
    fn compact_session_controlled(&mut self, control: &RequestControl) -> Result<(), ModelError> {
        control.check()?;
        // The budget is measured on what the model would actually receive: the messages plus the tool
        // schemas, which for the engine's ten tools is a real part of the request.
        if self.context_characters() <= session::CONTEXT_BUDGET_CHARS {
            return Ok(());
        }
        let Some(session) = &self.session else {
            return Ok(());
        };
        let Some(span) = session.plan_compaction() else {
            return Ok(());
        };
        // The summary is written by the model, about the transcript, before the transcript changes:
        // a summary asked for afterwards would describe text nobody kept.
        let summary = self.summarize_controlled(&span, control)?;
        control.check()?;
        if let Some(session) = &mut self.session {
            session.apply_compaction(&span, summary.as_deref());
            session.messages.clone_into(&mut self.messages);
        }
        Ok(())
    }

    /// Ask the model to summarise the span a compaction is about to drop.
    ///
    /// The prompt is a rule, not a request for prose: the summary has to keep the *facts* a later
    /// question could depend on — numbers, names, decisions, what was computed — because it is the
    /// only record of them. A model that cannot answer leaves `None`, and the compaction falls back to
    /// a note that says so rather than inventing a summary.
    #[cfg(test)]
    fn summarize(&mut self, span: &[Message]) -> Option<String> {
        self.summarize_controlled(span, &RequestControl::default())
            .ok()
            .flatten()
    }

    fn summarize_controlled(
        &mut self,
        span: &[Message],
        control: &RequestControl,
    ) -> Result<Option<String>, ModelError> {
        control.check()?;
        let mut transcript = String::new();
        for message in span {
            let line = match message {
                Message::System(text) | Message::Summary(text) => format!("[context] {text}"),
                Message::User(text) => format!("[user] {text}"),
                Message::Assistant { text, calls, .. } => {
                    let mut line = format!("[assistant] {text}");
                    for call in calls {
                        let _ = std::fmt::Write::write_fmt(
                            &mut line,
                            format_args!(
                                "\n  called {}({})",
                                command_id(&call.name),
                                call.arguments
                            ),
                        );
                    }
                    line
                }
                Message::Tool { content, .. } => format!("[tool] {content}"),
            };
            transcript.push_str(&line);
            transcript.push('\n');
        }
        let messages = [
            Message::System(COMPACTION_PROMPT.to_owned()),
            Message::User(transcript),
        ];
        control.check()?;
        match self.model.complete_controlled(&messages, &[], control) {
            Ok(turn) => {
                let failed = reject_truncated(turn.finish_reason.as_deref()).is_err()
                    || !turn.calls.is_empty()
                    || turn.text.trim().is_empty();
                self.usage.compaction.record(turn.usage.as_ref(), failed);
                if failed {
                    control.check()?;
                    return Ok(None);
                }
                control.check()?;
                Ok(Some(turn.text.trim().to_owned()))
            }
            Err(error) => {
                // Unknown failed-summary usage is explicitly counted, never priced at zero.
                self.usage.compaction.record(None, true);
                if matches!(
                    error,
                    ModelError::Cancelled | ModelError::Timeout | ModelError::Cleanup
                ) {
                    return Err(error);
                }
                control.check()?;
                Ok(None)
            }
        }
    }

    /// The characters the next request would carry: messages and tool schemas.
    #[must_use]
    pub fn context_characters(&self) -> usize {
        let messages = self
            .messages
            .iter()
            .map(message_characters)
            .fold(0usize, usize::saturating_add);
        let tools: usize = self
            .tools_with_skill_reader()
            .iter()
            .map(|tool| tool.to_string().chars().count())
            .fold(0usize, usize::saturating_add);
        messages.saturating_add(tools)
    }

    /// Forget the conversation, keeping the system prompt.
    ///
    /// A session that has gone wrong should be recoverable without restarting the process: the
    /// scenario lives in the server, so the tools still know everything the document says.
    pub fn forget(&mut self) {
        self.messages
            .retain(|message| matches!(message, Message::System(_)));
    }

    /// The engine's tools plus the skill reader, named the way a model's function call allows.
    ///
    /// The server's tool list is MCP's, where an id may be `access.compute`; a model's function name
    /// may not, so every name is translated here and translated back when a call arrives. This is the
    /// only place the two vocabularies meet, which is what keeps the registry the single source of
    /// truth for what a tool is.
    fn tools_with_skill_reader(&self) -> Vec<Value> {
        let mut tools: Vec<Value> = self
            .server
            .tools_json()
            .into_iter()
            .map(|mut tool| {
                if let Some(name) = tool.get("name").and_then(Value::as_str) {
                    let mapped = function_name(name);
                    if let Some(object) = tool.as_object_mut() {
                        object.insert("name".to_owned(), Value::String(mapped));
                    }
                }
                tool
            })
            .collect();
        tools.push(json!({
            "name": function_name(SKILL_TOOL),
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
        // The knowledge tools are the server's own, so they are taken from it rather than written
        // here twice: one description, one schema, two clients.
        for tool in self.server.knowledge_tools() {
            let mut tool = tool;
            if let Some(name) = tool.get("name").and_then(Value::as_str) {
                let mapped = function_name(name);
                if let Some(object) = tool.as_object_mut() {
                    object.insert("name".to_owned(), Value::String(mapped));
                }
            }
            tools.push(tool);
        }
        tools.push(json!({
            "name": function_name(SCENARIO_TOOL),
            "description": "Read the current scenario document as JSON: its name, epoch, duration, \
                            satellites and stations. Call it before answering anything about what \
                            the scenario contains.",
            "inputSchema": {"type": "object", "properties": {}},
        }));
        tools
    }

    /// Execute one tool call, returning the content for the model and a line for the trace.
    fn run_tool(&mut self, call: &ToolCall) -> (String, String, Option<serde_json::Value>) {
        let id = command_id(&call.name);
        if id == SCENARIO_TOOL {
            let request = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "resources/read",
                "params": {"uri": "stkcraft://scenario"},
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
                .unwrap_or_else(|| "the scenario could not be read".to_owned());
            // The summary is one line: the trace is a log, and a document is not a log line.
            return (content, format!("{SCENARIO_TOOL}()"), None);
        }
        if id == SKILL_TOOL {
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
            return (content, format!("{SKILL_TOOL}({name})"), None);
        }
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": id, "arguments": call.arguments},
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
                    id,
                    call.arguments,
                    text.lines().next().unwrap_or_default()
                );
                let structured = answer
                    .get("result")
                    .and_then(|result| result.get("structuredContent"))
                    .cloned();
                (text, summary, structured)
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
                    None,
                )
            }
            None => (
                "error: no answer from the tool".to_owned(),
                format!("{}({}) → no answer", call.name, call.arguments),
                None,
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
    use stkcraft_astro_engine::Scenario;

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

    fn reported_usage(tokens: u64) -> Usage {
        Usage {
            prompt_tokens: tokens,
            completion_tokens: tokens,
            total_tokens: tokens,
            ..Usage::default()
        }
    }

    #[test]
    fn persisted_usage_survives_tool_final_and_resume_without_wire_metadata() {
        let directory = std::env::temp_dir().join("stkcraft-usage-resume");
        let mut session = session::Session::create(&directory).expect("session");
        session.id = "usage-resume".into();
        session.path = directory.join("usage-resume.jsonl");
        let _ = std::fs::remove_file(&session.path);
        let mut agent = agent(vec![
            Turn {
                calls: vec![ToolCall {
                    id: "c".into(),
                    name: "scenario.read".into(),
                    arguments: json!({}),
                }],
                usage: Some(reported_usage(11)),
                finish_reason: Some("tool_calls".into()),
                ..Turn::default()
            },
            Turn {
                text: "done".into(),
                usage: Some(reported_usage(22)),
                finish_reason: Some("stop".into()),
                ..Turn::default()
            },
        ])
        .with_session(session);
        let answer = agent.ask("read").expect("answer");
        assert_eq!(answer.usage.model.known.total_tokens, 33);
        let resumed = session::Session::open(&directory, "usage-resume").expect("resume");
        assert_eq!(resumed.messages, agent.messages);
        let metadata: Vec<_> = resumed
            .messages
            .iter()
            .filter_map(|message| match message {
                Message::Assistant {
                    usage,
                    finish_reason,
                    ..
                } => Some((
                    usage.as_ref().map(|usage| usage.total_tokens),
                    finish_reason.as_deref(),
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            metadata,
            [(Some(11), Some("tool_calls")), (Some(22), Some("stop"))]
        );
        for protocol in [
            ProviderProtocol::OpenAiCompatible,
            ProviderProtocol::DeepSeek,
        ] {
            let provider = Provider::new("unused", "m", "test").with_options(ProviderOptions {
                protocol,
                ..ProviderOptions::default()
            });
            let body = provider.body(&resumed.messages, &[]);
            assert_eq!(body, provider.body(&agent.messages, &[]));
            for message in body["messages"].as_array().expect("messages") {
                assert!(message.get("usage").is_none());
                assert!(message.get("finish_reason").is_none());
            }
        }
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn answer_includes_summary_cost_and_unknown_failed_summary_attempt() {
        struct CompactionModel {
            fail_summary: bool,
        }
        impl ChatModel for CompactionModel {
            fn complete(&self, messages: &[Message], _: &[Value]) -> Result<Turn, String> {
                if matches!(messages.first(),Some(Message::System(prompt)) if prompt == COMPACTION_PROMPT)
                {
                    if self.fail_summary {
                        return Err("offline".into());
                    }
                    return Ok(Turn {
                        text: "summary".into(),
                        usage: Some(reported_usage(13)),
                        ..Turn::default()
                    });
                }
                Ok(Turn {
                    text: "answer".into(),
                    usage: Some(reported_usage(7)),
                    ..Turn::default()
                })
            }
        }
        for fail_summary in [false, true] {
            let directory =
                std::env::temp_dir().join(format!("stkcraft-summary-usage-{fail_summary}"));
            let mut session = session::Session::create(&directory).expect("session");
            session
                .record(Message::System("rules".into()))
                .expect("record");
            for index in 0..9 {
                session
                    .record(Message::User(format!(
                        "question{index} {}",
                        "x".repeat(16_000)
                    )))
                    .expect("record");
                session
                    .record(Message::Assistant {
                        text: "done".into(),
                        calls: vec![],
                        reasoning_content: None,
                        usage: None,
                        finish_reason: None,
                    })
                    .expect("record");
            }
            let mut agent = Agent::new(
                CompactionModel { fail_summary },
                Server::new(Scenario::empty("t"), vec![]),
            )
            .with_session(session);
            let answer = agent.ask("next").expect("answer");
            assert_eq!(answer.usage.model.requests, 1);
            assert_eq!(answer.usage.model.known.total_tokens, 7);
            assert_eq!(answer.usage.compaction.requests, 1);
            assert_eq!(
                answer.usage.compaction.known.total_tokens,
                if fail_summary { 0 } else { 13 }
            );
            assert_eq!(
                answer.usage.compaction.failed_requests,
                u64::from(fail_summary)
            );
            assert_eq!(
                answer.usage.compaction.unknown_usage_requests,
                u64::from(fail_summary)
            );
            assert_eq!(answer.usage.compaction.is_complete(), !fail_summary);
            let _ = std::fs::remove_dir_all(directory);
        }
    }

    #[test]
    fn usage_parser_preserves_unknown_zero_huge_cache_and_future_status() {
        for value in [None, Some(Value::Null)] {
            let mut response = json!({"choices":[{"message":{"content":"ok"}}]});
            if let Some(value) = value {
                response["usage"] = value;
            }
            let turn = parse_response(&response.to_string()).expect("unknown usage");
            assert!(turn.usage.is_none());
            assert!(turn.finish_reason.is_none());
        }
        for tokens in [0, u64::MAX] {
            let usage = json!({"prompt_tokens":tokens,"completion_tokens":tokens,"total_tokens":tokens,"prompt_cache_hit_tokens":tokens,"prompt_cache_miss_tokens":0,"completion_tokens_details":{"reasoning_tokens":tokens,"future":true},"future":123});
            let response = json!({"usage":usage,"choices":[{"finish_reason":"future_status","message":{"content":"ok"}}]});
            let turn = parse_response(&response.to_string()).expect("valid usage");
            let usage = turn.usage.expect("reported");
            assert_eq!(usage.prompt_tokens, tokens);
            assert_eq!(usage.total_tokens, tokens);
            assert_eq!(usage.prompt_cache_hit_tokens, Some(tokens));
            assert_eq!(usage.prompt_cache_miss_tokens, Some(0));
            assert_eq!(
                usage
                    .completion_tokens_details
                    .expect("details")
                    .reasoning_tokens,
                Some(tokens)
            );
            assert_eq!(turn.finish_reason.as_deref(), Some("future_status"));
        }
    }

    #[test]
    fn usage_boundary_checks_every_counter_and_finish_type() {
        for field in [
            "prompt_tokens",
            "completion_tokens",
            "total_tokens",
            "prompt_cache_hit_tokens",
            "prompt_cache_miss_tokens",
        ] {
            for invalid in [
                json!(-1),
                json!(1.5),
                json!("3"),
                json!(true),
                json!([]),
                json!({}),
            ] {
                let mut usage = json!({"prompt_tokens":1,"completion_tokens":1,"total_tokens":2});
                usage[field] = invalid;
                assert!(
                    parse_response(&json!({"usage":usage,"choices":[{"message":{}}]}).to_string())
                        .is_err(),
                    "{field}"
                );
            }
        }
        for invalid in [json!(-1), json!(1.5), json!("3"), json!(true)] {
            let response = json!({"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2,"completion_tokens_details":{"reasoning_tokens":invalid}},"choices":[{"message":{}}]});
            assert!(parse_response(&response.to_string()).is_err());
            if !invalid.is_string() {
                let response = json!({"choices":[{"finish_reason":invalid,"message":{}}]});
                assert!(parse_response(&response.to_string()).is_err());
            }
        }
        let response = r#"{"usage":{"prompt_tokens":18446744073709551616,"completion_tokens":0,"total_tokens":0},"choices":[{"message":{}}]}"#;
        assert!(parse_response(response).is_err());
    }

    #[test]
    fn cap_is_opt_in_for_generic_and_validated_before_transport() {
        let generic = Provider::new("invalid-no-network", "m", "test");
        assert!(generic.body(&[], &[]).get("max_tokens").is_none());
        let options = ProviderOptions::resolve("custom", None, None)
            .expect("resolve")
            .with_max_output_tokens(Some("37"))
            .expect("cap");
        assert_eq!(
            generic.clone().with_options(options).body(&[], &[])["max_tokens"],
            37
        );
        for invalid in ["0", "-1", "1.5", "4294967296", "garbage", ""] {
            assert!(
                ProviderOptions::default()
                    .with_max_output_tokens(Some(invalid))
                    .is_err()
            );
        }
        let provider = generic.with_options(ProviderOptions {
            max_output_tokens: Some(0),
            ..ProviderOptions::default()
        });
        assert!(
            provider
                .complete(&[], &[])
                .expect_err("no network")
                .contains("nonzero")
        );
    }

    #[test]
    fn scripted_truncated_turns_never_dispatch_or_succeed() {
        for calls in [
            vec![],
            vec![ToolCall {
                id: "c".into(),
                name: "scenario.rename".into(),
                arguments: json!({"name":"changed"}),
            }],
        ] {
            let mut agent = agent(vec![Turn {
                text: "partial".into(),
                calls,
                finish_reason: Some("length".into()),
                usage: Some(reported_usage(3)),
                ..Turn::default()
            }]);
            let original = agent.server().scenario().name.clone();
            assert!(
                agent
                    .ask("run")
                    .expect_err("truncated")
                    .contains("truncated")
            );
            assert_eq!(agent.server().scenario().name, original);
            assert!(
                !agent
                    .messages
                    .iter()
                    .any(|message| matches!(message, Message::Assistant { .. }))
            );
            assert_eq!(agent.usage().model.failed_requests, 1);
            assert_eq!(agent.usage().model.known.total_tokens, 3);
            assert!(agent.usage().model.is_complete());
        }
    }

    #[test]
    fn answer_usage_counts_all_tool_rounds_and_reports_unknown_failures() {
        struct Failure;
        impl ChatModel for Failure {
            fn complete(&self, _: &[Message], _: &[Value]) -> Result<Turn, String> {
                Err("offline".into())
            }
        }
        let call = || ToolCall {
            id: "c".into(),
            name: "scenario.read".into(),
            arguments: json!({}),
        };
        let mut agent = agent(vec![
            Turn {
                calls: vec![call()],
                usage: Some(reported_usage(10)),
                ..Turn::default()
            },
            Turn {
                calls: vec![call()],
                ..Turn::default()
            },
            Turn {
                text: "done".into(),
                usage: Some(reported_usage(20)),
                finish_reason: Some("stop".into()),
                ..Turn::default()
            },
        ]);
        let answer = agent.ask("read").expect("answer");
        assert_eq!(answer.usage.model.requests, 3);
        assert_eq!(answer.usage.model.known.total_tokens, 30);
        assert_eq!(answer.usage.model.unknown_usage_requests, 1);
        assert!(!answer.usage.model.is_complete());
        assert_eq!(answer.usage.compaction.requests, 0);
        assert_eq!(agent.usage(), &answer.usage);
        let mut failed = Agent::new(Failure, Server::new(Scenario::empty("t"), vec![]));
        assert!(failed.ask("x").is_err());
        assert_eq!(failed.usage().model.failed_requests, 1);
        assert_eq!(failed.usage().model.unknown_usage_requests, 1);
        assert!(!failed.usage().model.is_complete());
    }

    #[test]
    fn usage_aggregation_saturates_and_marks_overflow_and_optional_gaps() {
        let mut aggregate = UsageAggregate::default();
        let mut usage = reported_usage(u64::MAX);
        usage.prompt_cache_hit_tokens = Some(u64::MAX);
        aggregate.record(Some(&usage), false);
        assert!(aggregate.is_complete());
        aggregate.record(Some(&usage), false);
        assert!(aggregate.overflowed);
        assert!(!aggregate.is_complete());
        assert_eq!(aggregate.known.total_tokens, u64::MAX);
        aggregate.record(Some(&reported_usage(1)), false);
        assert_eq!(aggregate.known.prompt_cache_hit_tokens, None);
    }

    #[test]
    fn compaction_request_usage_is_separate_including_unknown_failure() {
        for (usage, finish, expected_summary) in [
            (Some(reported_usage(17)), None, true),
            (None, None, true),
            (Some(reported_usage(9)), Some("length".into()), false),
        ] {
            let mut agent = agent(vec![Turn {
                text: "summary".into(),
                usage: usage.clone(),
                finish_reason: finish,
                ..Turn::default()
            }]);
            assert_eq!(
                agent.summarize(&[Message::User("old".into())]).is_some(),
                expected_summary
            );
            assert_eq!(agent.usage().compaction.requests, 1);
            assert_eq!(agent.usage().model.requests, 0);
            assert_eq!(
                agent.usage().compaction.unknown_usage_requests,
                u64::from(usage.is_none())
            );
            assert_eq!(
                agent.usage().compaction.failed_requests,
                u64::from(!expected_summary)
            );
            assert_eq!(
                agent.usage().compaction.known.total_tokens,
                usage.map_or(0, |usage| usage.total_tokens)
            );
        }
    }

    #[test]
    fn output_contract_rejects_invalid_usage() {
        for usage in [
            json!({"prompt_tokens":-1,"completion_tokens":0,"total_tokens":0}),
            json!({"prompt_tokens":"1","completion_tokens":0,"total_tokens":1}),
            json!({}),
            json!(false),
            json!([1, 2, 3, null, null, null]),
        ] {
            let response = json!({"usage":usage,"choices":[{"message":{"content":"ok"}}]});
            assert!(parse_response(&response.to_string()).is_err(), "{usage}");
        }
    }

    #[test]
    fn configured_deepseek_has_bounded_output() {
        let options = ProviderOptions::resolve(DEEPSEEK_BASE_URL, None, None).expect("options");
        let provider = Provider::new("unused", "m", "test").with_options(options);
        assert_eq!(provider.body(&[], &[])["max_tokens"], 8192);
    }

    #[test]
    fn output_contract_rejects_truncated_text_and_tools() {
        for message in [json!({"content":"partial"}), json!({"tool_calls":[]})] {
            let response = json!({"choices":[{"finish_reason":"length","message":message}]});
            let error =
                parse_response(&response.to_string()).expect_err("truncated output must fail");
            assert!(error.contains("truncated"));
        }
    }

    #[test]
    fn provider_response_boundary_requires_new_enabled_tool_reasoning_before_dispatch() {
        let mut provider = Provider::new("unused", "m", "test").with_options(ProviderOptions {
            max_output_tokens: None,
            protocol: ProviderProtocol::DeepSeek,
            thinking: ThinkingMode::Enabled,
        });
        let tool = json!({"id":"c1","function":{"name":"scenario__rename","arguments":"{\"name\":\"changed\"}"}});
        for reasoning in [
            None,
            Some(Value::Null),
            Some(json!("")),
            Some(json!(" private 雪\n")),
        ] {
            let mut message = json!({"tool_calls":[tool.clone()]});
            if let Some(value) = &reasoning {
                message["reasoning_content"] = value.clone();
            }
            let response = json!({"choices":[{"message":message}]}).to_string();
            assert_eq!(
                provider.parse_turn(&response).is_ok(),
                reasoning.as_ref().is_some_and(Value::is_string)
            );
            provider.options.thinking = ThinkingMode::Disabled;
            assert!(provider.parse_turn(&response).is_ok());
            provider.options = ProviderOptions::default();
            assert!(provider.parse_turn(&response).is_ok());
            provider.options = ProviderOptions {
                max_output_tokens: None,
                protocol: ProviderProtocol::DeepSeek,
                thinking: ThinkingMode::Enabled,
            };
        }
        assert!(
            provider
                .parse_turn(r#"{"choices":[{"message":{"content":"answer"}}]}"#)
                .is_ok()
        );
        let contradictory =
            Provider::new("invalid-no-network", "m", "test").with_options(ProviderOptions {
                max_output_tokens: None,
                protocol: ProviderProtocol::OpenAiCompatible,
                thinking: ThinkingMode::Enabled,
            });
        assert!(
            contradictory
                .complete(&[], &[])
                .expect_err("before curl")
                .contains("requires the deepseek protocol")
        );
    }

    #[test]
    fn malformed_reasoning_cannot_dispatch_otherwise_valid_calls() {
        struct InvalidReasoning;
        impl ChatModel for InvalidReasoning {
            fn complete(&self, _: &[Message], _: &[Value]) -> Result<Turn, String> {
                parse_response(
                    r#"{"choices":[{"message":{"reasoning_content":false,"tool_calls":[{"id":"c1","function":{"name":"scenario__rename","arguments":"{\"name\":\"changed\"}"}}]}}]}"#,
                )
            }
        }
        let mut agent = Agent::new(
            InvalidReasoning,
            Server::new(Scenario::empty("unchanged"), vec![]),
        );
        let before = serde_json::to_value(agent.server().scenario()).expect("scenario");
        let mut trace = vec![];
        let error = agent
            .ask_observing("rename", &mut |line| trace.push(line.to_owned()))
            .expect_err("invalid reasoning");
        assert!(error.contains("model did not complete"));
        assert_eq!(trace, Vec::<String>::new());
        assert_eq!(
            serde_json::to_value(agent.server().scenario()).expect("scenario"),
            before
        );
    }

    #[test]
    fn provider_error_payload_never_echoes_opaque_reasoning() {
        let error = parse_response(
            r#"{"error":{"message":"refused","reasoning_content":"secret-reasoning-marker"}}"#,
        )
        .expect_err("provider error");
        assert!(!error.contains("secret-reasoning-marker"));
    }

    #[test]
    fn summaries_use_only_public_text_and_agent_session_costs_agree() {
        struct SummaryModel;
        impl ChatModel for SummaryModel {
            fn complete(&self, messages: &[Message], _: &[Value]) -> Result<Turn, String> {
                assert!(!format!("{messages:?}").contains("secret-reasoning"));
                assert!(format!("{messages:?}").contains("public-answer"));
                Ok(Turn {
                    text: "public summary".into(),
                    ..Turn::default()
                })
            }
        }
        let directory = std::env::temp_dir().join("stkcraft-reasoning-cost");
        std::fs::create_dir_all(&directory).expect("directory");
        let mut session = session::Session::create(&directory).expect("session");
        let message = Message::Assistant {
            text: "public-answer".into(),
            calls: vec![],
            usage: None,
            finish_reason: None,
            reasoning_content: Some("secret-reasoning 雪".into()),
        };
        session.record(message.clone()).expect("record");
        let path = session.path.clone();
        let cost = session.characters();
        let mut agent = Agent::new(SummaryModel, Server::new(Scenario::empty("test"), vec![]))
            .with_session(session);
        let schemas = agent
            .tools_with_skill_reader()
            .iter()
            .map(|tool| tool.to_string().chars().count())
            .sum::<usize>();
        assert_eq!(agent.context_characters(), cost + schemas);
        assert_eq!(agent.summarize(&[message]), Some("public summary".into()));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn reasoning_survives_two_tool_rounds_final_answer_and_resume() {
        struct Replay {
            index: std::cell::Cell<usize>,
        }
        impl ChatModel for Replay {
            fn complete(&self, messages: &[Message], _: &[Value]) -> Result<Turn, String> {
                let index = self.index.get();
                let provider = Provider::new("unused", "m", "test").with_options(ProviderOptions {
                    max_output_tokens: None,
                    protocol: ProviderProtocol::DeepSeek,
                    thinking: ThinkingMode::Enabled,
                });
                provider.validate_history(messages)?;
                let body = provider.body(messages, &[]);
                let reasoning: Vec<_> = body["messages"]
                    .as_array()
                    .expect("messages")
                    .iter()
                    .filter_map(|message| message.get("reasoning_content").and_then(Value::as_str))
                    .collect();
                assert_eq!(
                    reasoning,
                    ["earlier private", " private 雪\n", ""][..=index]
                );
                self.index.set(index + 1);
                Ok(Turn {
                    text: if index == 2 {
                        "public answer".into()
                    } else {
                        String::new()
                    },
                    calls: if index == 2 {
                        vec![]
                    } else {
                        vec![ToolCall {
                            id: format!("c{index}"),
                            name: "scenario__read".into(),
                            arguments: json!({}),
                        }]
                    },
                    usage: None,
                    finish_reason: None,
                    reasoning_content: Some([" private 雪\n", "", "final private"][index].into()),
                })
            }
        }
        let directory = std::env::temp_dir().join("stkcraft-reasoning-replay");
        std::fs::create_dir_all(&directory).expect("directory");
        let mut session = session::Session::create(&directory).expect("session");
        session
            .record(Message::System("rules".into()))
            .expect("record");
        session
            .record(Message::User("earlier".into()))
            .expect("record");
        session
            .record(Message::Assistant {
                text: "earlier answer".into(),
                calls: vec![],
                usage: None,
                finish_reason: None,
                reasoning_content: Some("earlier private".into()),
            })
            .expect("record");
        let mut agent = Agent::new(
            Replay {
                index: std::cell::Cell::new(0),
            },
            Server::new(Scenario::empty("test"), vec![]),
        )
        .with_session(session);
        let mut trace = vec![];
        let answer = agent
            .ask_observing("continue", &mut |value| trace.push(value.to_owned()))
            .expect("answer");
        assert_eq!(answer.text, "public answer");
        assert!(!format!("{answer:?} {trace:?}").contains("private"));
        let saved = agent.session.as_ref().expect("session");
        let resumed = session::Session::open(&directory, &saved.id).expect("resume");
        assert_eq!(resumed.messages, agent.messages);
        let provider = Provider::new("unused", "m", "test").with_options(ProviderOptions {
            max_output_tokens: None,
            protocol: ProviderProtocol::DeepSeek,
            thinking: ThinkingMode::Enabled,
        });
        assert_eq!(
            provider.body(&resumed.messages, &[]),
            provider.body(&agent.messages, &[])
        );
        assert!(!format!("{:?}", resumed.messages).contains("private"));
        let _ = std::fs::remove_file(&saved.path);
    }

    #[test]
    fn legacy_tool_history_is_refused_before_network_only_for_enabled_deepseek() {
        let messages = vec![Message::Assistant {
            text: String::new(),
            calls: vec![ToolCall {
                id: "legacy".into(),
                name: "scenario__read".into(),
                arguments: json!({}),
            }],
            usage: None,
            finish_reason: None,
            reasoning_content: None,
        }];
        let mut provider =
            Provider::new("invalid-no-network", "m", "test").with_options(ProviderOptions {
                max_output_tokens: None,
                protocol: ProviderProtocol::DeepSeek,
                thinking: ThinkingMode::Enabled,
            });
        let error = provider
            .complete(&messages, &[])
            .expect_err("refused before curl");
        assert!(error.contains("new session"));
        assert!(error.contains("--thinking disabled"));
        provider.options.thinking = ThinkingMode::Disabled;
        assert!(provider.validate_history(&messages).is_ok());
        provider.options = ProviderOptions::default();
        assert!(provider.validate_history(&messages).is_ok());
    }

    #[test]
    fn reasoning_parser_distinguishes_absent_null_empty_and_unicode_and_rejects_all_wrong_types() {
        for value in [Value::Null, json!(""), json!(" 私密\n")] {
            let turn = parse_response(
                &json!({"choices":[{"message":{"reasoning_content":value}}]}).to_string(),
            )
            .expect("valid reasoning");
            assert_eq!(turn.reasoning_content, value.as_str().map(str::to_owned));
            assert!(!format!("{turn:?}").contains("私密"));
        }
        assert_eq!(
            parse_response(r#"{"choices":[{"message":{}}]}"#)
                .expect("absent")
                .reasoning_content,
            None
        );
        for value in [
            json!(true),
            json!(123),
            json!([]),
            json!({"private":"secret"}),
        ] {
            assert!(
                parse_response(
                    &json!({"choices":[{"message":{"reasoning_content":value}}]}).to_string()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn explicit_protocol_modes_preserve_generic_requests_and_replay_reasoning() {
        let generic = Provider::new(DEEPSEEK_BASE_URL, "deepseek-chat", "test");
        let messages = vec![Message::Assistant {
            text: "answer".into(),
            calls: vec![],
            usage: None,
            finish_reason: None,
            reasoning_content: Some(" 秘密\n".into()),
        }];
        let original = generic.body(&messages, &[]);
        assert!(original.get("thinking").is_none());
        assert!(original["messages"][0].get("reasoning_content").is_none());
        assert_eq!(original["temperature"], 0.0);
        for (mode, name) in [
            (ThinkingMode::Enabled, "enabled"),
            (ThinkingMode::Disabled, "disabled"),
        ] {
            let provider = generic.clone().with_options(ProviderOptions {
                max_output_tokens: None,
                protocol: ProviderProtocol::DeepSeek,
                thinking: mode,
            });
            let body = provider.body(&messages, &[]);
            assert_eq!(body["thinking"]["type"], name);
            assert_eq!(body["messages"][0]["reasoning_content"], " 秘密\n");
            assert_eq!(
                body.get("temperature").is_none(),
                mode == ThinkingMode::Enabled
            );
            assert!(body.get("max_tokens").is_none());
        }
        assert_eq!(
            ProviderOptions::resolve(DEEPSEEK_BASE_URL, None, None)
                .expect("defaults")
                .protocol,
            ProviderProtocol::DeepSeek
        );
        assert_eq!(
            ProviderOptions::resolve("https://proxy.invalid", None, None)
                .expect("proxy")
                .protocol,
            ProviderProtocol::OpenAiCompatible
        );
        assert!(ProviderOptions::resolve(DEEPSEEK_BASE_URL, Some("typo"), None).is_err());
        assert!(
            ProviderOptions::resolve(DEEPSEEK_BASE_URL, Some("compatible"), Some("enabled"))
                .is_err()
        );
        assert!(ProviderOptions::resolve(DEEPSEEK_BASE_URL, None, Some("typo")).is_err());
    }

    #[test]
    fn malformed_reasoning_is_rejected_without_echoing_it() {
        let response = r#"{"choices":[{"message":{"content":"ok","reasoning_content":{"secret":"private-reasoning"}}}]}"#;
        let error = parse_response(response).expect_err("reasoning must be a string or null");
        assert!(error.contains("reasoning_content"));
        assert!(!error.contains("private-reasoning"));
    }

    #[test]
    fn the_observer_sees_each_tool_call_as_it_happens() {
        // A caller that shows progress needs the lines while the loop runs, not afterwards: the
        // same sequence arrives in `answer.trace`, in the same order.
        let mut agent = agent(vec![
            Turn {
                usage: None,
                finish_reason: None,
                reasoning_content: None,
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
                usage: None,
                finish_reason: None,
                reasoning_content: None,
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
    fn a_session_survives_the_process_and_resumes_where_it_stopped() {
        // The property the transcript exists for: a conversation is not a process's memory. Close
        // the terminal, come back, and the previous questions and answers are still in front of the
        // model.
        let directory = std::env::temp_dir().join("stkcraft-agent-session");
        let _ = std::fs::remove_dir_all(&directory);
        let session = crate::session::Session::create(&directory).expect("a session");
        let mut agent = agent(vec![
            Turn {
                usage: None,
                finish_reason: None,
                reasoning_content: None,
                text: "Renamed.".to_owned(),
                calls: Vec::new(),
            },
            Turn {
                usage: None,
                finish_reason: None,
                reasoning_content: None,
                text: "It is still Gemini.".to_owned(),
                calls: Vec::new(),
            },
        ])
        .with_session(session.clone());
        agent.ask("Rename it").expect("the first answer");
        let id = agent.session().expect("a session").id.clone();
        assert_eq!(
            agent.history_len(),
            3,
            "the transcript is not in the conversation"
        );

        // A new process: a new agent, the same session on disk.
        let resumed = crate::session::Session::open(&directory, &id).expect("reopened");
        assert_eq!(resumed.messages.len(), 3);
        let mut agent = Agent::new(
            Script {
                turns: std::cell::RefCell::new(vec![Turn {
                    usage: None,
                    finish_reason: None,
                    reasoning_content: None,
                    text: "Still Gemini.".to_owned(),
                    calls: Vec::new(),
                }]),
            },
            Server::new(Scenario::empty("session test"), Vec::new()),
        )
        .with_session(resumed);
        agent
            .ask("What is it called?")
            .expect("the follow-up answer");
        let history = &agent.messages;
        let questions: Vec<&str> = history
            .iter()
            .filter_map(|message| match message {
                Message::User(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(questions, vec!["Rename it", "What is it called?"]);
        let _ = std::fs::remove_dir_all(&directory);
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
                        usage: None,
                        finish_reason: None,
                        reasoning_content: None,
                        text: String::new(),
                        calls: vec![ToolCall {
                            id: "call-1".to_owned(),
                            name: "scenario.rename".to_owned(),
                            arguments: json!({"name": "Gemini"}),
                        }],
                    },
                    Turn {
                        usage: None,
                        finish_reason: None,
                        reasoning_content: None,
                        text: "Renamed.".to_owned(),
                        calls: Vec::new(),
                    },
                    Turn {
                        usage: None,
                        finish_reason: None,
                        reasoning_content: None,
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
                Message::System(_) | Message::Summary(_) => "system",
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
                usage: None,
                finish_reason: None,
                reasoning_content: None,
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
    fn a_model_sees_a_name_the_provider_accepts_and_the_registry_still_answers() {
        // The defect this pins: every command id is `namespace.verb`, and an OpenAI-compatible
        // function name must match `^[a-zA-Z0-9_-]+$`, so the provider refused the whole request
        // before generating a token. The translation has to be injective as well as legal.
        let separator_free = |name: &str| {
            !name.is_empty()
                && name.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
                })
        };
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for command in stkcraft_astro_engine::Registry::builtin().iter() {
            let mapped = function_name(command.descriptor.id);
            assert!(
                separator_free(&mapped),
                "{mapped} is not a legal function name"
            );
            assert_eq!(command_id(&mapped), command.descriptor.id);
            assert!(!mapped.contains('.'), "{mapped} still has a dot");
            assert!(seen.insert(mapped), "two ids map to the same function name");
        }
        // Ids that would collide under a hyphen still do not under the separator.
        assert_ne!(function_name("a.b-c"), function_name("a-b.c"));
        assert_eq!(function_name(SKILL_TOOL), "skill__read");
    }

    #[test]
    fn a_call_named_the_way_the_model_sees_it_reaches_the_command() {
        let mut agent = agent(vec![
            Turn {
                usage: None,
                finish_reason: None,
                reasoning_content: None,
                text: String::new(),
                calls: vec![ToolCall {
                    id: "call-1".to_owned(),
                    name: "scenario__rename".to_owned(),
                    arguments: json!({"name": "Gemini"}),
                }],
            },
            Turn {
                usage: None,
                finish_reason: None,
                reasoning_content: None,
                text: "Renamed.".to_owned(),
                calls: Vec::new(),
            },
        ]);
        let answer = agent.ask("Rename it").expect("an answer");
        assert_eq!(agent.server().scenario().name, "Gemini");
        assert!(
            answer.trace[0].starts_with("scenario.rename("),
            "{:?}",
            answer.trace
        );
    }

    #[test]
    fn the_prompt_tells_the_model_to_read_the_library_first() {
        // The ordering is the product: a number that can be traced to the person's own specification
        // is worth trusting with a mission timeline. A prompt that mentions the library anywhere
        // else would let a model answer from memory first and cite afterwards.
        let agent = agent(vec![]);
        let mut agent = agent;
        let prompt = agent.system_prompt();
        let knowledge = prompt
            .find("knowledge.search")
            .expect("the library is named");
        let first_rule = prompt.find("1. ").expect("the rules are numbered");
        assert!(
            knowledge < prompt.find("2. ").expect("a second rule"),
            "the library is not rule one: {prompt}"
        );
        assert!(
            knowledge > first_rule,
            "the library is named before the rules"
        );
        assert!(
            prompt.contains("do not answer the question from memory")
                || prompt.contains("say so plainly")
        );
        // The tools the model is offered include the library's, and the documents listing.
        let tools = agent.tools_with_skill_reader();
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect();
        assert!(names.contains(&"knowledge__search"), "{names:?}");
        assert!(names.contains(&"knowledge__documents"), "{names:?}");
    }

    #[test]
    fn the_model_can_read_the_scenario_it_is_asked_about() {
        // Asked "how many satellites are there?", a model with only add/remove tools can only say it
        // cannot know. The document is an MCP resource; this tool is how the model reaches it.
        let mut agent = agent(vec![
            Turn {
                usage: None,
                finish_reason: None,
                reasoning_content: None,
                text: String::new(),
                calls: vec![ToolCall {
                    id: "call-1".to_owned(),
                    name: function_name(SCENARIO_TOOL),
                    arguments: json!({}),
                }],
            },
            Turn {
                usage: None,
                finish_reason: None,
                reasoning_content: None,
                text: "It has none.".to_owned(),
                calls: Vec::new(),
            },
        ]);
        agent.ask("How many satellites?").expect("an answer");
        let tool_message = agent
            .messages
            .iter()
            .find_map(|message| match message {
                Message::Tool { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .expect("a tool result");
        // The document, not a summary of it: the model can count what is in it.
        assert!(tool_message.contains("\"satellites\""), "{tool_message}");
        assert!(tool_message.contains("\"stations\""), "{tool_message}");
        assert!(tool_message.contains("agent test"), "{tool_message}");
    }

    #[test]
    fn the_tool_schemas_a_model_receives_carry_mapped_names() {
        let agent = agent(vec![]);
        let tools = agent.tools_with_skill_reader();
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect();
        assert!(names.contains(&"access__compute"), "{names:?}");
        assert!(names.contains(&"skill__read"), "{names:?}");
        assert!(names.iter().all(|name| !name.contains('.')), "{names:?}");
    }

    #[test]
    fn a_plain_answer_needs_no_tools() {
        let mut agent = agent(vec![Turn {
            usage: None,
            finish_reason: None,
            reasoning_content: None,
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
                usage: None,
                finish_reason: None,
                reasoning_content: None,
                text: String::new(),
                calls: vec![ToolCall {
                    id: "call-1".to_owned(),
                    name: "scenario.rename".to_owned(),
                    arguments: json!({"name": "Gemini"}),
                }],
            },
            Turn {
                usage: None,
                finish_reason: None,
                reasoning_content: None,
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
                usage: None,
                finish_reason: None,
                reasoning_content: None,
                text: String::new(),
                calls: vec![ToolCall {
                    id: "call-1".to_owned(),
                    name: "satellite.add".to_owned(),
                    arguments: json!({"name": "broken"}),
                }],
            },
            Turn {
                usage: None,
                finish_reason: None,
                reasoning_content: None,
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
                    usage: None,
                    finish_reason: None,
                    reasoning_content: None,
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
        let skills = vec![stkcraft_agent_mcp::Skill::parse(
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
        let deepseek = Provider {
            options: ProviderOptions::default(),
            timeout_seconds: 120,
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
                usage: None,
                finish_reason: None,
                reasoning_content: None,
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
    fn response_boundary_rejects_invalid_tool_fields() {
        let valid = json!({"id":"c1", "function":{"name":"scenario__rename", "arguments":"{}"}});
        let mut cases = Vec::new();
        for arguments in [
            json!("{"),
            json!("[]"),
            json!("null"),
            json!("3"),
            json!("true"),
            json!("\"text\""),
            json!({}),
            Value::Null,
        ] {
            let mut call = valid.clone();
            call["function"]["arguments"] = arguments;
            cases.push((call, "arguments"));
        }
        let mut missing_arguments = valid.clone();
        missing_arguments["function"]
            .as_object_mut()
            .expect("object")
            .remove("arguments");
        cases.push((missing_arguments, "arguments"));
        for field in ["id", "name"] {
            for value in [json!(""), json!("   "), Value::Null, json!(7)] {
                let mut call = valid.clone();
                if field == "id" {
                    call[field] = value;
                } else {
                    call["function"][field] = value;
                }
                cases.push((call, field));
            }
            let mut call = valid.clone();
            if field == "id" {
                call.as_object_mut().expect("object").remove(field);
            } else {
                call["function"]
                    .as_object_mut()
                    .expect("object")
                    .remove(field);
            }
            cases.push((call, field));
        }
        for (call, field) in cases {
            let response = json!({"choices":[{"message":{"tool_calls":[call]}}]});
            let error = parse_response(&response.to_string()).expect_err("invalid field must fail");
            assert!(error.contains(field), "{field}: {error}");
        }
    }

    #[test]
    fn response_boundary_requires_message_object_and_call_array() {
        for message in [Value::Null, json!("text"), json!([]), json!(4)] {
            let response = json!({"choices":[{"message":message}]});
            let error = parse_response(&response.to_string()).expect_err("message object required");
            assert!(error.contains("message"), "{error}");
        }
        for calls in [json!({}), json!("calls"), json!(1), json!(true)] {
            let response = json!({"choices":[{"message":{"tool_calls":calls}}]});
            let error = parse_response(&response.to_string()).expect_err("array required");
            assert!(error.contains("tool_calls"), "{error}");
        }
    }

    #[test]
    fn response_boundary_preserves_plain_text_and_multiple_calls() {
        for message in [
            json!({"content":"answer"}),
            json!({"content":"answer", "tool_calls":null}),
        ] {
            let turn = parse_response(&json!({"choices":[{"message":message}]}).to_string())
                .expect("plain answer");
            assert_eq!(turn.text, "answer");
            assert_eq!(turn.calls, Vec::<ToolCall>::new());
        }
        let calls = json!([
            {"id":"c1","function":{"name":"scenario__rename","arguments":"{\"name\":\"Gemini\"}"}},
            {"id":"c2","function":{"name":"scenario__read","arguments":"{}"}}
        ]);
        let turn =
            parse_response(&json!({"choices":[{"message":{"tool_calls":calls}}]}).to_string())
                .expect("valid calls");
        assert_eq!(turn.calls.len(), 2);
        assert_eq!(turn.calls[0].id, "c1");
        assert_eq!(turn.calls[0].arguments["name"], "Gemini");
        assert_eq!(turn.calls[1].name, "scenario__read");
        assert_eq!(turn.calls[1].arguments, json!({}));
    }

    #[test]
    fn response_boundary_does_not_echo_unicode_payloads() {
        let response = format!("{{\"padding\":\"{}\"}}", "星".repeat(100));
        assert!(!response.is_char_boundary(200));
        let error = parse_response(&response).expect_err("no message");
        assert!(error.contains("no message"));
        assert!(!error.contains("星"));
    }

    #[test]
    fn response_boundary_invalid_second_call_cannot_mutate_the_scenario() {
        struct InvalidResponse;
        impl ChatModel for InvalidResponse {
            fn complete(&self, _messages: &[Message], _tools: &[Value]) -> Result<Turn, String> {
                parse_response(
                    r#"{"choices":[{"message":{"tool_calls":[
                    {"id":"c1","function":{"name":"scenario__rename","arguments":"{\"name\":\"Gemini\"}"}},
                    {"id":"c2","function":{"name":"scenario__rename","arguments":"{"}}
                ]}}]}"#,
                )
            }
        }
        let scenario = Scenario::empty("unchanged");
        let before = serde_json::to_value(&scenario).expect("scenario JSON");
        let mut agent = Agent::new(InvalidResponse, Server::new(scenario, Vec::new()));
        assert!(agent.ask("rename").is_err());
        assert_eq!(
            serde_json::to_value(agent.server().scenario()).expect("scenario JSON"),
            before
        );
    }

    #[test]
    fn a_malformed_response_is_an_error() {
        assert!(parse_response("not json").is_err());
        assert!(parse_response(r#"{"error":{"message":"bad key"}}"#).is_err());
        assert!(parse_response(r#"{"choices":[]}"#).is_err());
    }

    #[test]
    fn a_missing_key_is_reported_by_name() {
        // The message must say which variable a caller has to set. (The environment is read, not
        // modified: `remove_var` is unsafe in this toolchain and removing another test's key would
        // be a race.)
        let message = match Provider::from_environment() {
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

    #[test]
    fn a_provider_never_prints_its_credential() {
        // A key in a log line is a key in a bug report: `Debug` shows that one is present and
        // nothing about what it is.
        let provider = Provider::new(
            "https://example/v1/chat/completions",
            "m",
            "sk-secret-value",
        );
        let printed = format!("{provider:?}");
        assert!(!printed.contains("sk-secret-value"), "{printed}");
        // The length is stated so "set" and "empty" can be told apart in a log without the value.
        assert!(
            printed.contains(&format!("<{} bytes>", "sk-secret-value".len())),
            "{printed}"
        );
        assert!(printed.contains("https://example"), "{printed}");
        assert!(provider.has_credential());
        assert!(!Provider::new("u", "m", "   ").has_credential());
    }
    #[test]
    fn controlled_cancellation_stops_before_mutation_and_between_tools() {
        let token = CancellationToken::default();
        token.cancel();
        let mut a = agent(vec![]);
        assert_eq!(
            a.ask_controlled("x", &RequestControl::new(token)),
            Err(ModelError::Cancelled)
        );
        assert_eq!(a.messages, []);
        assert_eq!(a.usage().model.requests, 0);
        let token = CancellationToken::default();
        let control = RequestControl::new(token.clone());
        let mut a = agent(vec![Turn {
            calls: vec![
                ToolCall {
                    id: "first".into(),
                    name: "scenario.rename".into(),
                    arguments: json!({"name":"first"}),
                },
                ToolCall {
                    id: "second".into(),
                    name: "scenario.rename".into(),
                    arguments: json!({"name":"second"}),
                },
            ],
            usage: Some(reported_usage(3)),
            ..Turn::default()
        }]);
        let result = a.ask_observing_controlled("rename", &mut |_| token.cancel(), &control);
        assert_eq!(result, Err(ModelError::Cancelled));
        assert_eq!(a.server().scenario().name, "first");
        assert_eq!(a.usage().model.requests, 1);
        assert_eq!(a.usage().model.known.total_tokens, 3);
    }
    #[test]
    fn cancellation_after_legacy_response_discards_turn_and_accounts_unknown_attempt() {
        struct Cancels(CancellationToken);
        impl ChatModel for Cancels {
            fn complete(&self, _: &[Message], _: &[Value]) -> Result<Turn, String> {
                self.0.cancel();
                Ok(Turn {
                    text: "unaccepted".into(),
                    ..Turn::default()
                })
            }
        }
        let token = CancellationToken::default();
        let mut a = Agent::new(
            Cancels(token.clone()),
            Server::new(Scenario::empty("t"), vec![]),
        );
        assert_eq!(
            a.ask_controlled("x", &RequestControl::new(token)),
            Err(ModelError::Cancelled)
        );
        assert_eq!(a.usage().model.requests, 1);
        assert_eq!(a.usage().model.unknown_usage_requests, 1);
        assert_eq!(a.usage().model.failed_requests, 1);
        assert!(
            !a.messages
                .iter()
                .any(|m| matches!(m, Message::Assistant { .. }))
        );
    }
    #[test]
    fn summary_interruptions_propagate_instead_of_falling_back() {
        struct Interrupted(ModelError);
        impl ChatModel for Interrupted {
            fn complete(&self, _: &[Message], _: &[Value]) -> Result<Turn, String> {
                Err("unused".into())
            }
            fn complete_controlled(
                &self,
                _: &[Message],
                _: &[Value],
                _: &RequestControl,
            ) -> Result<Turn, ModelError> {
                Err(self.0)
            }
        }
        for error in [ModelError::Cancelled, ModelError::Timeout] {
            let mut a = Agent::new(
                Interrupted(error),
                Server::new(Scenario::empty("t"), vec![]),
            );
            assert_eq!(
                a.summarize_controlled(&[Message::User("old".into())], &RequestControl::default()),
                Err(error)
            );
            assert_eq!(a.usage().compaction.failed_requests, 1);
            assert_eq!(a.usage().compaction.unknown_usage_requests, 1);
        }
    }
}
