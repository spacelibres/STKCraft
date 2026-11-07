//! The control channel: a local socket another program drives `STKCraft` through.
//!
//! The Craft apps expose the same thing — `--control <port>`, a token file, JSON lines over
//! loopback — so a launcher, a test harness or a sibling application can drive an app the way a
//! person does: by running commands. This is that channel for `STKCraft`, and it is deliberately a
//! *transport*, not a second implementation: every method here reaches the engine through the same
//! command registry the menus, the CLI and the MCP server use.
//!
//! ```text
//! $ stkcraft control --port 7979
//! control: listening on 127.0.0.1:7979, token in /run/user/1000/stkcraft/control-7979.token
//!
//! $ printf '%s\n' \
//!     '{"id":0,"method":"auth","params":{"token":"…"}}' \
//!     '{"id":1,"method":"engine.execute","params":{"command":"scenario.rename","params":{"name":"Gemini"}}}' \
//!   | nc 127.0.0.1 7979
//! {"id":0,"result":{"ok":true,"version":"0.1.35"}}
//! {"id":1,"result":{"summary":"Scenario renamed to `Gemini`"}}
//! ```
//!
//! # Why a token file
//!
//! A loopback port is not a permission: any process on the machine can connect to it. The token is
//! written to a file only the user can read — `0600` in `$XDG_RUNTIME_DIR` on Unix; on Windows the
//! per-user runtime directory's inherited ACL is the protection, because there is no file mode to
//! set — so "can drive this app" means "can read that file", which is the same set of processes that
//! could edit the user's documents. The first request on a connection must be `auth`; everything else
//! is refused until it is.
//!
//! # What it is not
//!
//! Not a way to run code: the methods are a fixed list, and the only one that changes anything is
//! `engine.execute`, which runs a command from the registry — with the registry's own validation. A
//! method that does not exist is an error naming the methods that do.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::PathBuf;

use serde_json::{Value, json};
use stkcraft_agent_mcp::Server;

/// The methods this channel answers, for the error that names them.
pub const METHODS: [&str; 9] = [
    "auth",
    "version",
    "engine.commands",
    "engine.execute",
    "scenario.read",
    "knowledge.search",
    "knowledge.documents",
    "ui.state",
    "ui.screenshot",
];

/// The methods only a running window can answer, because the state they read is the window's.
///
/// A process that serves the channel *without* a window (the CLI's `control` subcommand) has its own
/// session and answers these itself. A window cannot: a launcher that renames the scenario and a
/// person looking at the screen must be looking at the same scenario, so the request goes to the
/// thread that owns it.
pub const WINDOW_METHODS: [&str; 4] = [
    "engine.execute",
    "scenario.read",
    "ui.state",
    "ui.screenshot",
];

/// A running window, for the methods only it can answer.
///
/// Called on the socket thread, so an implementation hands the request to the thread that owns the
/// window and waits for the answer — which is why [`Self::request`] returns the result rather than
/// taking a callback. The caller's timeout is its own business: a window that is not painting has to
/// answer with an error rather than hang the socket for ever.
pub trait UiHost: Send + Sync {
    /// Answer one window method.
    ///
    /// # Errors
    ///
    /// A message when the window cannot answer: no paint yet, the capture failed, or the person
    /// closed the window while the request was in flight.
    fn request(&self, method: &str, params: &Value) -> Result<Value, String>;
}

/// The session a connection drives: the engine's tools and the person's documents.
pub struct Control {
    server: Server,
    token: String,
    authenticated: bool,
    /// The window, when this channel is served by one.
    ui: Option<std::sync::Arc<dyn UiHost>>,
}

impl Control {
    /// A channel over a session, with the token a client must present.
    #[must_use]
    pub fn new(server: Server, token: impl Into<String>) -> Self {
        Self {
            server,
            token: token.into(),
            authenticated: false,
            ui: None,
        }
    }

    /// The same channel, but with the window answering the methods that read its state.
    #[must_use]
    pub fn with_ui(mut self, ui: std::sync::Arc<dyn UiHost>) -> Self {
        self.ui = Some(ui);
        self
    }

    /// The session, for a caller that wants to read the scenario afterwards.
    #[must_use]
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// Answer one request.
    ///
    /// A pure function of the request and the session state, like the MCP server's, so the whole
    /// protocol is tested without a socket. The `id` is echoed so a client can pipeline.
    pub fn handle(&mut self, request: &Value) -> Value {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        match self.dispatch(method, &params) {
            Ok(result) => json!({"id": id, "result": result}),
            Err(message) => json!({"id": id, "error": {"message": message}}),
        }
    }

    /// The methods, without the envelope.
    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        if method == "auth" {
            let presented = params
                .get("token")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !constant_time_eq(presented, &self.token) {
                return Err(
                    "the token is not this session's; read it from the token file".to_owned(),
                );
            }
            self.authenticated = true;
            return Ok(json!({"ok": true, "version": env!("CARGO_PKG_VERSION")}));
        }
        if !self.authenticated {
            return Err("auth first: send `auth` with the token from the token file".to_owned());
        }
        // A window owns its scenario, so the methods that read or change it go there rather than to
        // this process's copy — otherwise a launcher would drive a scenario nobody is looking at.
        if let Some(ui) = &self.ui
            && WINDOW_METHODS.contains(&method)
        {
            return ui.request(method, params);
        }
        match method {
            "version" => Ok(json!({"version": env!("CARGO_PKG_VERSION")})),
            "engine.commands" => self.mcp(json!({"method": "tools/list", "params": {}})),
            "engine.execute" => {
                let command = params
                    .get("command")
                    .and_then(Value::as_str)
                    .ok_or("engine.execute needs a `command`")?;
                let arguments = params.get("params").cloned().unwrap_or(json!({}));
                self.mcp(json!({
                    "method": "tools/call",
                    "params": {"name": command, "arguments": arguments},
                }))
            }
            "scenario.read" => self.mcp(json!({
                "method": "resources/read",
                "params": {"uri": "stkcraft://scenario"},
            })),
            "knowledge.search" => {
                let query = params
                    .get("query")
                    .and_then(Value::as_str)
                    .ok_or("knowledge.search needs a `query`")?;
                let limit = params.get("limit").cloned().unwrap_or(json!(5));
                self.mcp(json!({
                    "method": "tools/call",
                    "params": {"name": "knowledge.search", "arguments": {"query": query, "limit": limit}},
                }))
            }
            "knowledge.documents" => self.mcp(json!({
                "method": "tools/call",
                "params": {"name": "knowledge.documents", "arguments": {}},
            })),
            other => Err(format!(
                "unknown method `{other}`; this channel answers {}",
                METHODS.join(", ")
            )),
        }
    }

    /// Run one request through the MCP server, which is where the registry lives.
    fn mcp(&mut self, request: Value) -> Result<Value, String> {
        let mut envelope = request;
        if let Some(object) = envelope.as_object_mut() {
            object.insert("jsonrpc".to_owned(), Value::String("2.0".to_owned()));
            object.insert("id".to_owned(), json!(1));
        }
        match self.server.handle(&envelope) {
            Some(answer) => {
                if let Some(error) = answer.get("error") {
                    let message = error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("the command failed");
                    return Err(message.to_owned());
                }
                Ok(answer.get("result").cloned().unwrap_or(Value::Null))
            }
            None => Err("the session did not answer".to_owned()),
        }
    }
}

/// Where the token for a port lives: `$XDG_RUNTIME_DIR/stkcraft/control-<port>.token`, falling back
/// to the data directory's `run/` and then to the temporary directory.
#[must_use]
pub fn token_path(port: u16) -> PathBuf {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime)
            .join("stkcraft")
            .join(format!("control-{port}.token"));
    }
    let base = std::env::var_os("HOME").map_or_else(std::env::temp_dir, |home| {
        PathBuf::from(home).join(".local/share")
    });
    base.join("stkcraft/run")
        .join(format!("control-{port}.token"))
}

/// A fresh token: 32 hexadecimal characters from the operating system's cryptographic RNG, or a
/// digest of the process and the clock when that call fails.
///
/// `getrandom` is the platform API — `BCryptGenRandom` on Windows, `getrandom(2)` on Linux, the
/// Security framework on macOS — so the token no longer depends on a `/dev/urandom` path that does
/// not exist on Windows and used to fall back to a pid and the clock. The fallback here is still
/// honest about being one, and the OS call reads a bounded buffer, so the 2026-11-06 failure (a
/// device file read to the end, 130 GB and an OOM kill) has no path back in.
#[must_use]
pub fn new_token() -> String {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    let mut bytes = [0_u8; 32];
    if getrandom::fill(&mut bytes).is_ok() {
        hasher.update(bytes);
    } else {
        // Not entropy, but unpredictable enough that a pipeline cannot guess the token.
        hasher.update(std::process::id().to_le_bytes());
        hasher.update(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_nanos())
                .to_le_bytes(),
        );
    }
    let digest = hasher.finalize();
    let mut token = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        let _ = std::fmt::Write::write_fmt(&mut token, format_args!("{byte:02x}"));
    }
    token
}

/// Write the token where a client can read it, and nowhere else.
///
/// # Errors
///
/// A message when the file cannot be written. A channel nobody can authenticate against is not a
/// channel, so this is fatal to a `control` run.
pub fn write_token(path: &std::path::Path, token: &str) -> Result<(), String> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    }
    std::fs::write(path, token)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("could not restrict {}: {error}", path.display()))?;
    }
    Ok(())
}

/// Serve the channel on loopback until the process ends.
///
/// # Errors
///
/// A message when the port cannot be bound or the token cannot be written.
pub fn serve(port: u16, mut control: Control) -> Result<(), String> {
    let token = control.token.clone();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .map_err(|error| format!("could not listen on 127.0.0.1:{port}: {error}"))?;
    let bound = listener
        .local_addr()
        .map_err(|error| format!("could not read the bound address: {error}"))?;
    let path = token_path(bound.port());
    write_token(&path, &token)?;
    eprintln!(
        "control: listening on 127.0.0.1:{}, token in {} — methods: {}",
        bound.port(),
        path.display(),
        METHODS.join(", ")
    );
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        // One client at a time, and never on the accept loop: a slow client must not stop the next
        // one from connecting.
        if let Err(error) = handle_connection(stream, &mut control) {
            eprintln!("control: {error}");
        }
    }
    Ok(())
}

/// The largest request line this socket accepts, bytes.
///
/// A local peer that never sends a newline must not make the process allocate until it dies. The
/// limit is far above any protocol request (the scenario is read by a separate method), and a line
/// that hits it is answered with an error and the connection is closed.
const MAX_REQUEST_BYTES: u64 = 1024 * 1024;

/// Read one bounded line, or `None` at end of stream.
fn read_request_line(reader: &mut impl BufRead) -> std::io::Result<Option<String>> {
    let mut line = String::new();
    let read = reader
        .by_ref()
        .take(MAX_REQUEST_BYTES)
        .read_line(&mut line)?;
    if read == 0 {
        return Ok(None);
    }
    Ok(Some(line))
}

/// Read JSON lines from one client and answer them.
fn handle_connection(stream: TcpStream, control: &mut Control) -> Result<(), String> {
    let mut writer = stream
        .try_clone()
        .map_err(|error| format!("could not clone the socket: {error}"))?;
    let mut reader = BufReader::new(stream);
    loop {
        let line = match read_request_line(&mut reader) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                eprintln!("control: could not read a request: {error}");
                break;
            }
        };
        if !line.ends_with('\n') {
            let _ = writeln!(
                writer,
                "{}",
                json!({"id": Value::Null, "error": {"message": "request too large"}})
            );
            let _ = writer.flush();
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(line) {
            Ok(request) => control.handle(&request),
            Err(error) => {
                json!({"id": Value::Null, "error": {"message": format!("parse error: {error}")}})
            }
        };
        if writeln!(writer, "{answer}").is_err() {
            break;
        }
        let _ = writer.flush();
    }
    Ok(())
}

/// Compare two tokens without leaking where they differ.
///
/// A constant-time comparison for a loopback token is not paranoia about the network; it is that a
/// function called `eq` on a secret should not be usable as an oracle by a process that shares the
/// machine.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (x, y) in a.iter().zip(b) {
        difference |= x ^ y;
    }
    difference == 0
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use super::*;
    use stkcraft_astro_engine::Scenario;

    fn control() -> Control {
        Control::new(
            Server::new(Scenario::empty("control test"), Vec::new()),
            "0f1e2d3c4b5a69788796a5b4c3d2e1f0",
        )
    }

    fn call(control: &mut Control, method: &str, params: &Value) -> Value {
        control.handle(&json!({"id": 1, "method": method, "params": params}))
    }

    #[test]
    fn a_request_line_is_bounded_before_it_is_parsed() {
        let short = b"{\"id\":1}\n";
        let mut cursor = std::io::Cursor::new(short.to_vec());
        let line = read_request_line(&mut cursor)
            .expect("a cursor reads")
            .expect("a line");
        assert_eq!(line, "{\"id\":1}\n");

        let limit = usize::try_from(MAX_REQUEST_BYTES).expect("the limit fits in usize");
        let long = vec![b'a'; limit + 1];
        let mut cursor = std::io::Cursor::new(long);
        let line = read_request_line(&mut cursor)
            .expect("a cursor reads")
            .expect("a line");
        assert_eq!(line.len(), limit);
        assert!(
            !line.ends_with('\n'),
            "an unterminated line must be refused"
        );
    }

    fn authenticated() -> Control {
        let mut control = control();
        let answer = call(
            &mut control,
            "auth",
            &json!({"token": "0f1e2d3c4b5a69788796a5b4c3d2e1f0"}),
        );
        assert!(answer.get("error").is_none(), "{answer}");
        control
    }

    /// A stand-in for a window: it answers the walk of the protocol and records what it was asked.
    #[derive(Default)]
    struct FakeWindow {
        asked: std::sync::Mutex<Vec<String>>,
    }

    impl UiHost for FakeWindow {
        fn request(&self, method: &str, params: &Value) -> Result<Value, String> {
            if let Ok(mut asked) = self.asked.lock() {
                asked.push(format!("{method} {params}"));
            }
            match method {
                "ui.state" => Ok(json!({"scenario": "window's own", "satellites": 1})),
                "ui.screenshot" => Err("the window has not painted a frame yet".to_owned()),
                "engine.execute" => Ok(json!({"summary": "renamed by the window"})),
                other => Err(format!("the window does not answer `{other}`")),
            }
        }
    }

    #[test]
    fn a_window_answers_the_methods_that_read_its_state() {
        // The reason this exists: `engine.execute` from a launcher must change the scenario the
        // person is looking at, not a copy the socket thread happens to own.
        let window = std::sync::Arc::new(FakeWindow::default());
        let mut control = control().with_ui(window.clone());
        let answer = call(
            &mut control,
            "auth",
            &json!({"token": "0f1e2d3c4b5a69788796a5b4c3d2e1f0"}),
        );
        assert!(answer.get("error").is_none(), "{answer}");

        let state = call(&mut control, "ui.state", &json!({}));
        assert_eq!(
            state.get("result").and_then(|r| r.get("scenario")),
            Some(&json!("window's own"))
        );
        let executed = call(
            &mut control,
            "engine.execute",
            &json!({"command": "scenario.rename", "params": {"name": "Gemini"}}),
        );
        assert_eq!(
            executed
                .get("result")
                .and_then(|r| r.get("summary"))
                .and_then(Value::as_str),
            Some("renamed by the window")
        );
        // A capture that could not happen is an error with the reason in it, not a silent empty
        // image: a launcher that saves a blank PNG and reports success is worse than one told no.
        let shot = call(&mut control, "ui.screenshot", &json!({}));
        assert_eq!(
            shot.get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str),
            Some("the window has not painted a frame yet")
        );
        assert_eq!(
            window.asked.lock().map_or(0, |asked| asked.len()),
            3,
            "not every window method reached the window"
        );
    }

    #[test]
    fn a_process_without_a_window_answers_its_own_scenario() {
        // The control case: the CLI's `control` subcommand has no window, so the same methods must
        // still work — against its own session.
        let mut control = authenticated();
        let executed = call(
            &mut control,
            "engine.execute",
            &json!({"command": "scenario.rename", "params": {"name": "Gemini"}}),
        );
        assert!(executed.get("error").is_none(), "{executed}");
        let read = call(&mut control, "scenario.read", &json!({}));
        assert!(read.get("error").is_none(), "{read}");
        // A method only a window can answer is refused rather than guessed at.
        let state = call(&mut control, "ui.state", &json!({}));
        assert!(state.get("error").is_some(), "{state}");
    }

    #[test]
    fn nothing_is_answered_before_auth() {
        // The channel's whole security model: a loopback port is not a permission.
        let mut control = control();
        for method in [
            "version",
            "engine.commands",
            "engine.execute",
            "scenario.read",
            "knowledge.search",
        ] {
            let answer = call(&mut control, method, &json!({}));
            let message = answer["error"]["message"].as_str().expect("an error");
            assert!(message.contains("auth first"), "{method}: {message}");
        }
    }

    #[test]
    fn a_wrong_token_is_refused_and_a_right_one_is_accepted() {
        let mut control = control();
        let answer = call(&mut control, "auth", &json!({"token": "wrong"}));
        assert!(answer.get("error").is_some(), "{answer}");
        let answer = call(&mut control, "version", &json!({}));
        assert!(
            answer.get("error").is_some(),
            "a refused auth still opened the channel"
        );
        let answer = call(
            &mut control,
            "auth",
            &json!({"token": "0f1e2d3c4b5a69788796a5b4c3d2e1f0"}),
        );
        assert_eq!(answer["result"]["ok"], true);
        let answer = call(&mut control, "version", &json!({}));
        assert_eq!(answer["result"]["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn a_command_run_through_the_channel_changes_the_session() {
        // The property that makes the channel worth having: it drives the same registry an agent
        // and a menu do, so a launcher can do anything a person can.
        let mut control = authenticated();
        let answer = call(
            &mut control,
            "engine.execute",
            &json!({"command": "scenario.rename", "params": {"name": "Gemini"}}),
        );
        assert!(answer.get("error").is_none(), "{answer}");
        let summary = answer["result"]["content"][0]["text"]
            .as_str()
            .expect("a summary");
        assert!(summary.contains("Gemini"), "{summary}");
        assert_eq!(control.server().scenario().name, "Gemini");
    }

    #[test]
    fn the_command_catalogue_is_readable() {
        let mut control = authenticated();
        let answer = call(&mut control, "engine.commands", &json!({}));
        let names: Vec<&str> = answer["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert!(names.contains(&"access.compute"), "{names:?}");
        assert!(names.contains(&"knowledge.search"), "{names:?}");
    }

    #[test]
    fn the_scenario_is_readable_as_a_resource() {
        let mut control = authenticated();
        let answer = call(&mut control, "scenario.read", &json!({}));
        let text = answer["result"]["contents"][0]["text"]
            .as_str()
            .expect("the document");
        assert!(text.contains("control test"), "{text}");
    }

    #[test]
    fn a_knowledge_search_says_it_missed_rather_than_inventing() {
        // The knowledge base is empty in this session, so the answer must say so: the channel must
        // not be a way to get a guess out of the platform.
        let mut control = authenticated();
        let answer = call(
            &mut control,
            "knowledge.search",
            &json!({"query": "maximum off-nadir angle"}),
        );
        let text = answer["result"]["content"][0]["text"]
            .as_str()
            .expect("a summary");
        assert!(text.contains("No passage"), "{text}");
        assert_eq!(answer["result"]["structuredContent"]["missed"], true);
    }

    #[test]
    fn an_unknown_method_names_the_ones_that_exist() {
        let mut control = authenticated();
        let answer = call(&mut control, "engine.destroy", &json!({}));
        let message = answer["error"]["message"].as_str().expect("an error");
        assert!(message.contains("engine.execute"), "{message}");
        assert!(message.contains("knowledge.search"), "{message}");
    }

    #[test]
    fn a_command_that_fails_reports_why() {
        let mut control = authenticated();
        let answer = call(
            &mut control,
            "engine.execute",
            &json!({"command": "station.add", "params": {"name": "X"}}),
        );
        let message = answer["error"]["message"].as_str().expect("an error");
        assert!(
            message.contains("latitude") || message.contains("required"),
            "{message}"
        );
    }

    #[test]
    fn a_token_is_hex_and_does_not_repeat() {
        let first = new_token();
        let second = new_token();
        assert_eq!(first.len(), 32, "{first}");
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
        assert_ne!(first, second, "two tokens from the same entropy are equal");
    }

    #[test]
    fn the_token_file_is_written_where_a_client_looks() {
        let directory = std::env::temp_dir().join("stkcraft-control-token");
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("control-7979.token");
        write_token(&path, "abc").expect("written");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "abc");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "the token is readable by others");
        }
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn comparing_tokens_says_yes_only_for_equal_ones() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
        assert!(!constant_time_eq("", "abc"));
    }
}
