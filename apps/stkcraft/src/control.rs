//! The window's control channel: the protocol the CLI serves, answered by the window.
//!
//! A launcher drives a Craft app the way a person does — by running commands — and then looks at
//! what it did. That is two things, and only the first one worked: `stkcraft control --port` serves
//! the protocol from a *process*, but the process with the scenario a person is looking at is the
//! window, and a launcher could not reach it.
//!
//! So the window serves the same channel, and the requests split in two:
//!
//! - **the stateless half** — `version`, `engine.commands`, `knowledge.*` — is answered on the
//!   socket thread, which has its own session for exactly that: a command list is a command list,
//!   and the person's documents are files on disk.
//! - **the half that reads or changes the window** — `engine.execute`, `scenario.read`, `ui.state`,
//!   `ui.screenshot` — is forwarded to the thread that owns it. A launcher that renames the scenario
//!   and a person watching the screen must be looking at one scenario, not two copies that drift.
//!
//! The bridge is a channel of requests, each with its own reply sender; the window drains it once a
//! frame and answers. That keeps the socket thread from touching egui (which is single-threaded) and
//! keeps the protocol testable without a display, which is what [`crate::app`]'s own tests do.
//!
//! # Why a timeout
//!
//! A window that is minimised, or a picture that cannot be captured, has to answer *something*: a
//! client that waits for ever has no way to tell a slow window from a hung one. Every request carries
//! a deadline, and `ui.screenshot` reports the reason it could not capture rather than an empty image.

use std::sync::{Arc, mpsc};
use std::time::Duration;

use serde_json::Value;
use stkcraft_agent_control::UiHost;

/// How long a window method may take before the client is told the window did not answer.
///
/// A frame is a few milliseconds and a screenshot is one more, so this is not a performance budget:
/// it is the point at which "the window is not painting" becomes the answer.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// One request from the socket thread, waiting for the window to answer it.
pub struct Request {
    /// The method, one of [`stkcraft_agent_control::WINDOW_METHODS`].
    pub method: String,
    /// Its parameters, as the protocol received them.
    pub params: Value,
    /// Where the answer goes. Dropped unanswered if the window is gone, which the client sees as a
    /// closed channel rather than a hang.
    pub reply: mpsc::Sender<Result<Value, String>>,
}

impl Request {
    /// Answer the waiting client. A client that has already given up is not an error.
    pub fn answer(self, result: Result<Value, String>) {
        let _ = self.reply.send(result);
    }
}

/// The window's end of the channel: the requests waiting to be answered.
pub struct Bridge {
    /// Visible to the crate so a test can act as the socket thread and drive a window's methods
    /// without opening one.
    pub(crate) requests: mpsc::Receiver<Request>,
}

impl Bridge {
    /// Every request waiting, without blocking.
    ///
    /// Called once a frame, so a request is answered on the next frame after it arrived — which is
    /// what makes `ui.screenshot` mean "the frame this window paints next".
    pub fn pending(&self) -> Vec<Request> {
        self.requests.try_iter().collect()
    }
}

/// The socket thread's end: it hands a request over and waits for the answer.
struct Forward {
    sender: mpsc::Sender<Request>,
}

impl UiHost for Forward {
    fn request(&self, method: &str, params: &Value) -> Result<Value, String> {
        let (reply, answer) = mpsc::channel();
        self.sender
            .send(Request {
                method: method.to_owned(),
                params: params.clone(),
                reply,
            })
            .map_err(|_| "the window has closed; nothing can answer that method".to_owned())?;
        match answer.recv_timeout(REQUEST_TIMEOUT) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(format!(
                "the window did not answer `{method}` within {} s; it may be minimised or stuck",
                REQUEST_TIMEOUT.as_secs()
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("the window closed before it answered".to_owned())
            }
        }
    }
}

/// The socket thread's end, for a test that drives a window from another thread.
#[cfg(test)]
pub(crate) fn forward(sender: mpsc::Sender<Request>) -> Arc<dyn UiHost> {
    Arc::new(Forward { sender })
}

/// Start the channel on `port`, with this window answering the methods that read its state.
///
/// The socket thread is spawned here and its failure to bind is reported through the window's
/// stderr the way the CLI reports it, because a port already in use is a normal thing to hit.
///
/// # Errors
///
/// A message when this process cannot build the session the stateless half needs — which means the
/// scenario or the knowledge base could not be opened, both of which are reported rather than worked
/// around.
pub fn start(port: u16, server: stkcraft_agent_mcp::Server) -> Result<Bridge, String> {
    let (sender, receiver) = mpsc::channel();
    let control = stkcraft_agent_control::Control::new(server, stkcraft_agent_control::new_token())
        .with_ui(Arc::new(Forward { sender }));
    std::thread::Builder::new()
        .name("stkcraft-control".to_owned())
        .spawn(move || {
            // `serve` prints the port and the token file, and blocks until the process ends.
            if let Err(error) = stkcraft_agent_control::serve(port, control) {
                eprintln!("control: {error}");
            }
        })
        .map_err(|error| format!("could not start the control thread: {error}"))?;
    Ok(Bridge { requests: receiver })
}

/// The PNG of an egui frame, and what it is.
///
/// `ColorImage` is straight-alpha `Color32`; a screenshot is written as 8-bit RGB because the window
/// is opaque, which is also what the offscreen verification paths write, so one encoder serves both.
pub struct Capture {
    /// The image's width in pixels.
    pub width: u32,
    /// The image's height in pixels.
    pub height: u32,
    /// Packed RGB, three bytes per pixel, row-major from the top.
    pub rgb: Vec<u8>,
}

impl Capture {
    /// Take the pixels out of an egui image.
    #[must_use]
    pub fn of(image: &eframe::egui::ColorImage) -> Self {
        let rgb = image
            .pixels
            .iter()
            .flat_map(|pixel| [pixel.r(), pixel.g(), pixel.b()])
            .collect();
        Self {
            width: image.width() as u32,
            height: image.height() as u32,
            rgb,
        }
    }

    /// The image as a PNG.
    ///
    /// # Errors
    ///
    /// A message when the encoder cannot write, which for an in-memory buffer means the allocation
    /// failed — reported rather than panicked on.
    pub fn png(&self) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        crate::screenshot::write_png(&mut bytes, self.width, self.height, &self.rgb)
            .map_err(|error| format!("could not encode the screenshot: {error}"))?;
        Ok(bytes)
    }
}

/// Base64, so a screenshot can cross the protocol as JSON.
///
/// Hand-written because a screenshot is the only thing in this program that needs it and a crate for
/// forty lines of table lookup is a dependency with a version, a licence and a supply chain.
#[must_use]
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = u32::from(chunk.first().copied().unwrap_or(0));
        let second = u32::from(chunk.get(1).copied().unwrap_or(0));
        let third = u32::from(chunk.get(2).copied().unwrap_or(0));
        let packed = (first << 16) | (second << 8) | third;
        for index in 0..4 {
            let shift = 18 - index * 6;
            let value = ((packed >> shift) & 0x3f) as usize;
            // A character is padding when it covers a byte the chunk does not have.
            let padding = chunk.len() < 3 && index > chunk.len();
            let character = if padding {
                b'='
            } else {
                ALPHABET.get(value).copied().unwrap_or(b'=')
            };
            text.push(char::from(character));
        }
    }
    text
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_published_vectors() {
        // RFC 4648 §10, the cases that pin the padding rules.
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn a_capture_is_rgb_in_row_order_and_encodes_as_a_png() {
        // Two pixels: the capture has to drop alpha and keep the order, and the PNG has to be a PNG
        // rather than a byte string that happens to be non-empty. The pixels are opaque because egui
        // stores premultiplied colour, and a translucent pixel's channels would test egui's arithmetic
        // rather than this function's.
        let image = eframe::egui::ColorImage::new(
            [2, 1],
            vec![
                eframe::egui::Color32::from_rgb(1, 2, 3),
                eframe::egui::Color32::from_rgb(4, 5, 6),
            ],
        );
        let capture = Capture::of(&image);
        assert_eq!((capture.width, capture.height), (2, 1));
        assert_eq!(capture.rgb, vec![1, 2, 3, 4, 5, 6]);
        let png = capture.png().expect("a PNG");
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    #[test]
    fn a_request_is_answered_through_its_own_channel() {
        // The bridge's contract: the window takes a request and answers *that* client, so a socket
        // thread and its reply cannot be crossed with another's.
        let (sender, receiver) = mpsc::channel::<Request>();
        let bridge = Bridge { requests: receiver };
        let (reply, answer) = mpsc::channel();
        sender
            .send(Request {
                method: "ui.state".to_owned(),
                params: Value::Null,
                reply,
            })
            .expect("sent");
        let waiting = bridge.pending();
        assert_eq!(waiting.len(), 1);
        let request = waiting.into_iter().next().expect("one request");
        assert_eq!(request.method, "ui.state");
        request.answer(Ok(serde_json::json!({"ok": true})));
        assert_eq!(
            answer.recv().expect("an answer"),
            Ok(serde_json::json!({"ok": true}))
        );
        // Nothing is left: a second drain finds an empty bridge rather than answering twice.
        assert!(bridge.pending().is_empty());
    }
}
