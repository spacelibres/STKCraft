//! Bounded native curl transport. Curl config escaping follows curl(1), verified 2026-11-07.
use crate::{ModelError, RequestControl};
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
pub(crate) const LIMIT: usize = 8 * 1024 * 1024;
const MARKER: &[u8] = b"\nSTKCRAFT_HTTP_STATUS:";
const SUFFIX: usize = 26;
struct Capped(Vec<u8>);
impl Write for Capped {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > LIMIT.saturating_sub(self.0.len()) {
            return Err(std::io::ErrorKind::FileTooLarge.into());
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn quote(out: &mut Capped, value: &str) -> Result<(), ModelError> {
    out.write_all(b"\"")
        .map_err(|_| ModelError::RequestTooLarge)?;
    for ch in value.chars() {
        let escaped = match ch {
            '\\' => Some("\\\\"),
            '"' => Some("\\\""),
            '\n' => Some("\\n"),
            '\r' => Some("\\r"),
            '\t' => Some("\\t"),
            '\u{b}' => Some("\\v"),
            _ => None,
        };
        if let Some(text) = escaped {
            out.write_all(text.as_bytes())
                .map_err(|_| ModelError::RequestTooLarge)?;
        } else {
            if ch.is_control() {
                return Err(ModelError::Configuration);
            }
            let mut bytes = [0; 4];
            out.write_all(ch.encode_utf8(&mut bytes).as_bytes())
                .map_err(|_| ModelError::RequestTooLarge)?;
        }
    }
    out.write_all(b"\"\n")
        .map_err(|_| ModelError::RequestTooLarge)
}
pub(crate) fn encode_config(
    url: &str,
    key: &str,
    body: &serde_json::Value,
    remaining: Duration,
) -> Result<Vec<u8>, ModelError> {
    if url.len() > LIMIT || key.len() > LIMIT {
        return Err(ModelError::RequestTooLarge);
    }
    if !(url.starts_with("https://") || url.starts_with("http://"))
        || url.chars().any(char::is_control)
        || key.chars().any(char::is_control)
        || remaining.is_zero()
    {
        return Err(ModelError::Configuration);
    }
    let mut json = Capped(Vec::new());
    serde_json::to_writer(&mut json, body).map_err(|_| ModelError::RequestTooLarge)?;
    let body = std::str::from_utf8(&json.0).map_err(|_| ModelError::Protocol)?;
    let mut out = Capped(Vec::new());
    out.write_all(b"silent\nshow-error\ngloboff\nrequest = \"POST\"\nproto = \"=http,https\"\nheader = \"Content-Type: application/json\"\n").map_err(|_|ModelError::RequestTooLarge)?;
    for (name, value) in [
        (
            "max-time",
            format!("{:.6}", remaining.as_secs_f64().max(0.000_001)),
        ),
        ("url", url.to_owned()),
        ("header", format!("Authorization: Bearer {key}")),
        ("write-out", "\nSTKCRAFT_HTTP_STATUS:%{http_code}\n".into()),
    ] {
        out.write_all(name.as_bytes())
            .and_then(|()| out.write_all(b" = "))
            .map_err(|_| ModelError::RequestTooLarge)?;
        quote(&mut out, &value)?;
    }
    out.write_all(b"data-raw = ")
        .map_err(|_| ModelError::RequestTooLarge)?;
    quote(&mut out, body)?;
    Ok(out.0)
}
pub(crate) fn parse_output(mut bytes: Vec<u8>) -> Result<String, ModelError> {
    let start = bytes
        .len()
        .checked_sub(MARKER.len() + 4)
        .ok_or(ModelError::Protocol)?;
    let suffix = bytes.get(start..).ok_or(ModelError::Protocol)?;
    if !suffix.starts_with(MARKER) || suffix.last() != Some(&b'\n') {
        return Err(ModelError::Protocol);
    }
    let digits = suffix
        .get(MARKER.len()..MARKER.len() + 3)
        .ok_or(ModelError::Protocol)?;
    if !digits.iter().all(u8::is_ascii_digit) {
        return Err(ModelError::Protocol);
    }
    let status = digits
        .iter()
        .fold(0u16, |value, digit| value * 10 + u16::from(digit - b'0'));
    if status < 100 {
        return Err(ModelError::Protocol);
    }
    if start > LIMIT {
        return Err(ModelError::ResponseTooLarge);
    }
    if !(200..300).contains(&status) {
        return Err(ModelError::HttpStatus(status));
    }
    bytes.truncate(start);
    String::from_utf8(bytes).map_err(|_| ModelError::Protocol)
}
struct Guard(Child);
impl Drop for Guard {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.try_wait();
        }
    }
}
enum Event {
    Written(Result<(), ModelError>),
    Output(Result<Vec<u8>, ModelError>),
    Drained(Result<(), ModelError>),
}
fn read_output(mut pipe: impl Read) -> Result<Vec<u8>, ModelError> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let n = pipe.read(&mut chunk).map_err(|_| ModelError::Read)?;
        if n == 0 {
            return Ok(bytes);
        }
        if n > (LIMIT + SUFFIX).saturating_sub(bytes.len()) {
            return Err(ModelError::ResponseTooLarge);
        }
        bytes.extend_from_slice(chunk.get(..n).ok_or(ModelError::Read)?);
    }
}
fn drain(mut pipe: impl Read) -> Result<(), ModelError> {
    let mut chunk = [0; 8192];
    let mut retained = Vec::new();
    loop {
        let n = pipe.read(&mut chunk).map_err(|_| ModelError::Read)?;
        if n == 0 {
            return Ok(());
        }
        let take = n.min(4096usize.saturating_sub(retained.len()));
        retained.extend_from_slice(chunk.get(..take).ok_or(ModelError::Read)?);
    }
}
/// Supervise a direct child. Normal outcomes reap it and join all workers. Exceptional Cleanup
/// reports OS/descendant pipe limits; portable std cannot interrupt arbitrary blocking pipe owners.
pub(crate) fn run(
    command: Command,
    config: Vec<u8>,
    control: &RequestControl,
) -> Result<String, ModelError> {
    run_inner(
        command,
        config,
        control,
        &Hooks::default(),
        &mut Report::default(),
    )
}
#[derive(Default)]
#[allow(clippy::struct_excessive_bools)] // Independent private fault-injection switches.
pub(crate) struct Hooks {
    pub(crate) fail_start: Option<usize>,
    pub(crate) missing_pipe: bool,
    pub(crate) fail_read: bool,
    pub(crate) fail_write: bool,
    pub(crate) fail_wait: bool,
}
#[derive(Default, Debug)]
pub(crate) struct Report {
    pub(crate) started: usize,
    pub(crate) joined: usize,
    pub(crate) reaped: bool,
}
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn run_test(
    command: Command,
    config: Vec<u8>,
    control: &RequestControl,
    hooks: &Hooks,
) -> (Result<String, ModelError>, Report) {
    let mut report = Report::default();
    let result = run_inner(command, config, control, hooks, &mut report);
    (result, report)
}
fn run_inner(
    mut command: Command,
    config: Vec<u8>,
    control: &RequestControl,
    hooks: &Hooks,
    report: &mut Report,
) -> Result<String, ModelError> {
    control.check()?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    control.check()?;
    let mut child = Guard(command.spawn().map_err(|_| ModelError::Spawn)?);
    let mut workers: Vec<JoinHandle<()>> = Vec::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let setup = (|| {
        if hooks.missing_pipe {
            return Err(ModelError::Pipe);
        }
        let stdin = child.0.stdin.take().ok_or(ModelError::Pipe)?;
        let stdout = child.0.stdout.take().ok_or(ModelError::Pipe)?;
        let stderr = child.0.stderr.take().ok_or(ModelError::Pipe)?;
        let fail_read = hooks.fail_read;
        let fail_write = hooks.fail_write;
        if hooks.fail_start == Some(0) {
            return Err(ModelError::ThreadStart);
        }
        let sender = tx.clone();
        workers.push(
            thread::Builder::new()
                .name("model-stdout".into())
                .spawn(move || {
                    let _ = sender.send(Event::Output(if fail_read {
                        Err(ModelError::Read)
                    } else {
                        read_output(stdout)
                    }));
                })
                .map_err(|_| ModelError::ThreadStart)?,
        );
        if hooks.fail_start == Some(1) {
            return Err(ModelError::ThreadStart);
        }
        let sender = tx.clone();
        workers.push(
            thread::Builder::new()
                .name("model-stderr".into())
                .spawn(move || {
                    let _ = sender.send(Event::Drained(drain(stderr)));
                })
                .map_err(|_| ModelError::ThreadStart)?,
        );
        if hooks.fail_start == Some(2) {
            return Err(ModelError::ThreadStart);
        }
        let sender = tx.clone();
        workers.push(
            thread::Builder::new()
                .name("model-stdin".into())
                .spawn(move || {
                    let mut stdin = stdin;
                    let result = if fail_write {
                        Err(ModelError::Stdin)
                    } else {
                        stdin.write_all(&config).map_err(|_| ModelError::Stdin)
                    };
                    drop(stdin);
                    let _ = sender.send(Event::Written(result));
                })
                .map_err(|_| ModelError::ThreadStart)?,
        );
        Ok(())
    })();
    report.started = workers.len();
    drop(tx);
    let mut error = setup.err();
    let mut status = None;
    let mut output = None;
    let mut events = 0;
    let mut cleanup = None;
    let mut fail_wait = hooks.fail_wait;
    loop {
        if error.is_none() {
            error = control.check().err();
        }
        while let Ok(event) = rx.try_recv() {
            events += 1;
            match event {
                Event::Output(Ok(bytes)) => output = Some(bytes),
                Event::Output(Err(e)) | Event::Written(Err(e)) | Event::Drained(Err(e))
                    if error.is_none() =>
                {
                    error = Some(e);
                }
                _ => {}
            }
        }
        if fail_wait {
            if error.is_none() {
                error = Some(ModelError::Wait);
            }
            fail_wait = false;
        }
        if status.is_none() {
            match child.0.try_wait() {
                Ok(Some(exit)) => {
                    status = Some(exit);
                    report.reaped = true;
                    if !exit.success() && error.is_none() {
                        error = Some(if exit.code() == Some(28) {
                            ModelError::Timeout
                        } else {
                            ModelError::ModelFailed
                        });
                    }
                }
                Ok(None) => {}
                Err(_) => {
                    if error.is_none() {
                        error = Some(ModelError::Wait);
                    }
                }
            }
        }
        let mut index = 0;
        while index < workers.len() {
            if workers.get(index).is_some_and(JoinHandle::is_finished) {
                let handle = workers.swap_remove(index);
                report.joined += 1;
                if handle.join().is_err() && error.is_none() {
                    error = Some(ModelError::Read);
                }
            } else {
                index += 1;
            }
        }
        if error.is_some() && cleanup.is_none() {
            drop(child.0.stdin.take());
            drop(child.0.stdout.take());
            drop(child.0.stderr.take());
            if status.is_none() {
                let _ = child.0.kill();
            }
            cleanup = Some(Instant::now());
        }
        if status.is_some() && workers.is_empty() && (error.is_some() || events == 3) {
            if let Some(error) = error {
                return Err(error);
            }
            if events != 3 {
                return Err(ModelError::Read);
            }
            control.check()?;
            return parse_output(output.ok_or(ModelError::Read)?);
        }
        if cleanup.is_some_and(|start| start.elapsed() >= Duration::from_secs(1)) {
            return Err(ModelError::Cleanup);
        }
        thread::sleep(Duration::from_millis(10));
    }
}
