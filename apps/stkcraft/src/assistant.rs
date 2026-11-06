//! The assistant panel: a question, a model, and the engine's own tools.
//!
//! This module owns no analysis. It asks a question, and a worker thread runs the same loop
//! `stkcraft ask` runs — the MCP tools, the skills, the model — reporting each tool call back so
//! the panel can show it while it happens. Anything the model does, a person could have done by
//! hand through a menu, because both go through the command registry.
//!
//! # Why a thread
//!
//! A question takes seconds to a minute and a half: the model is a network round trip and every
//! tool call inside it is real. Doing that in `update` would freeze the window, and freezing the
//! clock is worse than failing — the app would look like it had hung on a solution that was
//! working. So the loop runs on a thread of its own and reports through channels the frame drains
//! without ever blocking.
//!
//! The thread builds its own server from a cloned scenario, so nothing in the engine is shared
//! across threads and no lock exists. The copy is the point as well as the mechanism: the question
//! is answered against the scenario it was asked about, and the result is adopted only when the
//! loop finishes, so a question in flight cannot half-edit the window while a person keeps working.

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use eframe::egui;
use stkcraft_engine::Scenario;

/// What the panel is doing.
#[derive(Debug, Default)]
enum State {
    /// Nothing asked yet.
    #[default]
    Idle,
    /// A question is running on the worker thread.
    Running,
    /// The model answered.
    Answered {
        /// The model's final text.
        text: String,
    },
    /// The loop failed, with the reason.
    Failed(String),
}

/// One question in flight.
struct Run {
    /// Tool-call lines as they arrive, `name(args) → summary`.
    trace: Receiver<String>,
    /// The final result: the answer, and the scenario the model left behind.
    result: Receiver<Result<(String, Scenario), String>>,
}

/// The function that runs one question. A type alias rather than a trait because a test needs to
/// substitute it and nothing else ever will; the real one is [`run`].
type Worker =
    fn(&Scenario, &std::path::Path, &str, &Sender<String>) -> Result<(String, Scenario), String>;

/// The assistant panel's state.
pub struct Assistant {
    /// Whether the panel is open.
    pub open: bool,
    /// The question being typed.
    pub question: String,
    state: State,
    trace: Vec<String>,
    run: Option<Run>,
    /// A scenario the model produced, waiting to be adopted by the window exactly once.
    adopt: Option<Scenario>,
    /// How a question is run. The real path is [`run`]; a test substitutes its own.
    worker: Worker,
}

impl Default for Assistant {
    fn default() -> Self {
        Self {
            open: false,
            question: String::new(),
            state: State::default(),
            trace: Vec::new(),
            run: None,
            adopt: None,
            worker: run,
        }
    }
}

impl Assistant {
    /// Whether a question is currently running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self.state, State::Running)
    }

    /// Ask the model, on a worker thread.
    ///
    /// The model is configured when the question is asked rather than when the app started: a
    /// person who exports a key while the window is open should not have to restart it, and one who
    /// has not gets the sentence that says so rather than a button that does nothing.
    fn start(&mut self, scenario: &Scenario, skills: &std::path::Path) {
        let question = self.question.trim().to_owned();
        if question.is_empty() {
            self.state = State::Failed("Type a question first.".to_owned());
            return;
        }
        if self.is_running() {
            return;
        }
        let scenario = scenario.clone();
        let skills = skills.to_path_buf();
        let (trace_tx, trace_rx) = channel();
        let (result_tx, result_rx) = channel();
        self.trace.clear();
        self.state = State::Running;
        self.run = Some(Run {
            trace: trace_rx,
            result: result_rx,
        });
        let worker = self.worker;
        std::thread::spawn(move || {
            let outcome = worker(&scenario, &skills, &question, &trace_tx);
            // A closed channel means the window closed mid-question; the loop still finishes and
            // its result is dropped, which is cheaper than teaching the agent a cancellation path.
            let _ = result_tx.send(outcome);
        });
    }

    /// Drain whatever the worker has reported. Never blocks.
    fn poll(&mut self) {
        let Some(run) = &self.run else {
            return;
        };
        while let Ok(line) = run.trace.try_recv() {
            self.trace.push(line);
        }
        match run.result.try_recv() {
            Ok(Ok((text, scenario))) => {
                self.state = State::Answered { text };
                self.adopt = Some(scenario);
                self.run = None;
            }
            Ok(Err(message)) => {
                self.state = State::Failed(message);
                self.run = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.state = State::Failed("The assistant stopped without an answer.".to_owned());
                self.run = None;
            }
        }
    }

    /// Draw the panel, returning the scenario to adopt if the model changed it.
    ///
    /// The scenario is handed back rather than written in place because adoption is the window's
    /// decision: it owns the document, and it is the only place that knows a document change has to
    /// be followed by a frame rebuild.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        scenario: &Scenario,
        skills: &std::path::Path,
    ) -> Option<Scenario> {
        self.poll();
        ui.heading("Assistant");
        match stkcraft_agent::DeepSeek::from_environment() {
            Ok(model) => {
                let host = model
                    .base_url
                    .split('/')
                    .nth(2)
                    .unwrap_or("the configured endpoint");
                ui.label(format!("{} at {host}", model.model));
            }
            Err(message) => {
                ui.colored_label(egui::Color32::from_rgb(230, 170, 120), message);
                ui.label(
                    "The assistant answers by running the engine's commands, so it needs a model \
                     to choose them.",
                );
            }
        }
        ui.separator();

        let running = self.is_running();
        let mut submit = false;
        ui.add_enabled_ui(!running, |ui| {
            let response = ui.add(
                egui::TextEdit::multiline(&mut self.question)
                    .desired_rows(3)
                    .hint_text("When can the station see the satellite?"),
            );
            // Enter asks; Shift+Enter starts a new line. A question is usually one line, and a
            // second keystroke to send it is a keystroke too many.
            submit = (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                || ui.button("Ask").clicked();
        });
        if submit {
            self.start(scenario, skills);
        }

        if self.is_running() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Running the engine's commands…");
            });
        }

        if !self.trace.is_empty() {
            ui.separator();
            ui.label("What it did");
            egui::ScrollArea::vertical()
                .id_salt("assistant-trace")
                .max_height(200.0)
                .show(ui, |ui| {
                    for line in &self.trace {
                        ui.label(egui::RichText::new(line).monospace().size(11.0));
                    }
                });
        }

        match &self.state {
            State::Idle | State::Running => {}
            State::Answered { text } => {
                ui.separator();
                ui.label("Answer");
                ui.label(text);
            }
            State::Failed(message) => {
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(230, 120, 120), message);
            }
        }
        self.adopt.take()
    }
}

/// The worker: the same loop the command line runs.
fn run(
    scenario: &Scenario,
    skills: &std::path::Path,
    question: &str,
    trace: &Sender<String>,
) -> Result<(String, Scenario), String> {
    let model = stkcraft_agent::DeepSeek::from_environment()?;
    let server = stkcraft_mcp::Server::new(scenario.clone(), stkcraft_mcp::load_skills(skills));
    let mut agent = stkcraft_agent::Agent::new(model, server);
    let mut report = |line: &str| {
        let _ = trace.send(line.to_owned());
    };
    let answer = agent.ask_observing(question, &mut report)?;
    Ok((answer.text, agent.server().scenario().clone()))
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A worker that reports two tool calls and answers, so the panel's threading and adoption are
    /// exercised without a model, a key or a socket.
    #[allow(clippy::unnecessary_wraps)]
    fn scripted(
        scenario: &Scenario,
        _skills: &std::path::Path,
        question: &str,
        trace: &Sender<String>,
    ) -> Result<(String, Scenario), String> {
        trace
            .send("scenario.rename(\"Gemini\") → renamed".to_owned())
            .ok();
        trace
            .send("access.compute({}) → 3 pass(es)".to_owned())
            .ok();
        let mut produced = scenario.clone();
        produced.name = format!("{question} answered");
        Ok(("Three passes.".to_owned(), produced))
    }

    /// A worker that fails, the way a missing credential does.
    fn refuses(
        _scenario: &Scenario,
        _skills: &std::path::Path,
        _question: &str,
        _trace: &Sender<String>,
    ) -> Result<(String, Scenario), String> {
        Err("DEEPSEEK_API_KEY is not set".to_owned())
    }

    fn panel(worker: Worker) -> Assistant {
        Assistant {
            worker,
            ..Assistant::default()
        }
    }

    /// Wait for the worker to finish. The loop is bounded so a worker that never reports fails the
    /// test instead of hanging the suite.
    fn settle(assistant: &mut Assistant) {
        for _ in 0..2_000 {
            if !assistant.is_running() {
                return;
            }
            assistant.poll();
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("the panel never left the running state");
    }

    #[test]
    fn a_question_reports_its_trace_and_offers_the_scenario_once() {
        let mut assistant = panel(scripted);
        assistant.question = "When is the pass?".to_owned();
        assistant.start(&Scenario::empty("window"), std::path::Path::new("skills"));
        settle(&mut assistant);

        assert_eq!(assistant.trace.len(), 2);
        let first = assistant.trace.first().map_or("", String::as_str);
        assert!(first.starts_with("scenario.rename("), "{first}");
        match &assistant.state {
            State::Answered { text } => assert_eq!(text, "Three passes."),
            other => panic!("expected an answer, got {other:?}"),
        }
        let Some(adopted) = assistant.adopt.take() else {
            panic!("the panel produced no scenario for the window to adopt");
        };
        assert_eq!(adopted.name, "When is the pass? answered");
        // The window adopts once: a second frame must not re-apply a stale scenario.
        assert!(assistant.adopt.take().is_none());
    }

    #[test]
    fn a_missing_model_becomes_a_message_rather_than_a_dead_button() {
        let mut assistant = panel(refuses);
        assistant.question = "anything".to_owned();
        assistant.start(&Scenario::empty("window"), std::path::Path::new("skills"));
        settle(&mut assistant);
        match &assistant.state {
            State::Failed(message) => assert!(message.contains("DEEPSEEK_API_KEY")),
            other => panic!("expected a failure, got {other:?}"),
        }
        assert_eq!(assistant.trace, Vec::<String>::new());
    }

    #[test]
    fn an_empty_question_is_refused_without_starting_a_thread() {
        let mut assistant = panel(scripted);
        assistant.question = "   ".to_owned();
        assistant.start(&Scenario::empty("window"), std::path::Path::new("skills"));
        // Still idle: nothing was spawned, so nothing can arrive later.
        assert!(!assistant.is_running());
        match &assistant.state {
            State::Failed(message) => assert!(message.contains("Type a question")),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}
