//! The assistant panel: a question, a model, and the engine's own tools.
//!
//! This module owns no analysis. It asks a question, and a worker thread runs the same loop
//! `stkcraft ask` runs — the MCP tools, the skills, the model — reporting each tool call back so
//! the panel can show it while it happens. Anything the model does, a person could have done by
//! hand through a menu, because both go through the command registry.
//!
//! # Why a thread, and why one thread
//!
//! A question takes seconds to a minute and a half: the model is a network round trip and every
//! tool call inside it is real. Doing that in `update` would freeze the window, and freezing the
//! clock is worse than failing — the app would look like it had hung on a solution that was
//! working. So the question runs on a thread of its own and reports through channels the frame
//! drains without ever blocking.
//!
//! That thread lives for the session rather than for one question, because the *conversation* lives
//! on it: "and the day after?" is only answerable if the agent still remembers the scenario the
//! previous answer built. A thread per question would rebuild the model client and lose the history
//! every time, which is the one thing a chat interface cannot do.
//!
//! The worker keeps its own copy of the session, so nothing in the engine is shared across threads
//! and no lock exists. The window hands it the current scenario with every question and adopts what
//! came back only when the loop finishes, so a question in flight cannot half-edit the document
//! while a person keeps working.

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use eframe::egui;
use stkcraft_agent_runtime::{CancellationToken, RequestControl};
use stkcraft_astro_engine::Scenario;

/// What the panel is doing.
#[derive(Debug, Default)]
enum State {
    /// Nothing asked yet.
    #[default]
    Idle,
    /// A question is running on the worker thread.
    Running,
    /// Cancellation requested; retain the receivers until the worker finishes.
    Stopping,
    /// The cancelled result cannot be adopted.
    Cancelled,
    /// The platform could not start a background worker.
    WorkerUnavailable,
    /// The model answered.
    Answered {
        /// The model's final text.
        text: String,
    },
    /// The document changed while the assistant was working.
    Conflict,
    /// The loop failed, with the reason.
    Failed(String),
}

/// What one question produces: the answer, and the scenario the model left behind.
type Reply = Result<(String, Scenario), String>;

/// What the worker is asked to do.
enum Command {
    /// Answer a question.
    Ask(Box<Job>),
    /// Forget the conversation, keeping the document the tools built.
    Forget,
}

/// How a responder is built. The real one is [`deepseek`]; a test substitutes its own.
type Factory = fn(&std::path::Path, &Scenario) -> Result<Box<dyn Responder>, String>;
type WorkerSpawner = fn(Box<dyn FnOnce() + Send>) -> std::io::Result<std::thread::JoinHandle<()>>;
type ModelStatus = fn() -> Result<(String, String), String>;

fn spawn_worker(job: Box<dyn FnOnce() + Send>) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("stkcraft-assistant".to_owned())
        .spawn(job)
}

fn model_status() -> Result<(String, String), String> {
    crate::prefs::model_provider().map(|provider| (provider.model, provider.base_url))
}

/// The receivers and cancellation identity of exactly one accepted job.
struct ActiveRun {
    trace: Receiver<String>,
    result: Receiver<Reply>,
    token: CancellationToken,
}

/// One question for the worker.
struct Job {
    question: String,
    /// The document the question is asked about.
    scenario: Scenario,
    /// Where each tool-call line goes as it happens.
    trace: Sender<String>,
    /// Where the answer goes, once.
    result: Sender<Reply>,
    /// Shared with the panel; it remains valid even after a reply was queued.
    token: CancellationToken,
}

impl Command {
    /// The question this command carries, if it is a question.
    fn job(&self) -> Option<&Job> {
        match self {
            Self::Ask(job) => Some(job),
            Self::Forget => None,
        }
    }
}

/// Answers questions, keeping the conversation between them.
///
/// A trait rather than a function so a test can answer from a script: the panel's threading,
/// adoption and failure handling are then exercised with no model, no key and no socket.
pub trait Responder: Send {
    /// Answer one question about `scenario`, reporting each tool call to `trace`.
    fn ask(&mut self, scenario: &Scenario, question: &str, trace: &Sender<String>) -> Reply;

    /// Cooperative adapter for existing responders. Only an override can interrupt an in-flight call.
    fn ask_controlled(
        &mut self,
        scenario: &Scenario,
        question: &str,
        trace: &Sender<String>,
        control: &RequestControl,
    ) -> Reply {
        control.check().map_err(|error| error.to_string())?;
        let result = self.ask(scenario, question, trace);
        control.check().map_err(|error| error.to_string())?;
        result
    }

    /// Forget the conversation, keeping whatever the tools have built.
    ///
    /// A discussion that has gone wrong — a wrong assumption the model keeps building on, a
    /// scenario that was edited into nonsense — needs a way out that is not "restart the
    /// application", and the scenario does not have to be thrown away with it.
    fn forget(&mut self) {}
}

/// The real responder: `DeepSeek` driving the engine's tools, with the conversation kept.
struct DeepSeekResponder {
    agent: stkcraft_agent_runtime::Agent<stkcraft_agent_runtime::Provider>,
}

impl Responder for DeepSeekResponder {
    fn ask(&mut self, scenario: &Scenario, question: &str, trace: &Sender<String>) -> Reply {
        self.ask_controlled(scenario, question, trace, &RequestControl::default())
    }

    fn ask_controlled(
        &mut self,
        scenario: &Scenario,
        question: &str,
        trace: &Sender<String>,
        control: &RequestControl,
    ) -> Reply {
        // The window owns the document; the session takes whatever it is given before answering, so
        // an answer is always about the scenario on screen.
        self.agent.server_mut().set_scenario(scenario.clone());
        let mut report = |line: &str| {
            let _ = trace.send(line.to_owned());
        };
        let answer = self
            .agent
            .ask_observing_controlled(question, &mut report, control)
            .map_err(|error| error.to_string())?;
        Ok((answer.text, self.agent.server().scenario().clone()))
    }

    fn forget(&mut self) {
        self.agent.forget();
    }
}

/// Build the real responder, or say why it cannot exist.
fn deepseek(skills: &std::path::Path, scenario: &Scenario) -> Result<Box<dyn Responder>, String> {
    let model = crate::prefs::model_provider()?;
    let server = session_server(skills, scenario, crate::knowledge::library)?;
    Ok(Box::new(DeepSeekResponder {
        agent: stkcraft_agent_runtime::Agent::new(model, server),
    }))
}

/// Construct the assistant's tool session without creating a model client.
fn session_server(
    skills: &std::path::Path,
    scenario: &Scenario,
    load_library: impl FnOnce() -> Result<stkcraft_agent_knowledge::Library, String>,
) -> Result<stkcraft_agent_mcp::Server, String> {
    Ok(
        stkcraft_agent_mcp::Server::new(scenario.clone(), stkcraft_agent_mcp::load_skills(skills))
            .with_knowledge(load_library()?),
    )
}

/// The assistant panel's state.
pub struct Assistant {
    /// Whether the panel is open.
    pub open: bool,
    /// The question being typed.
    pub question: String,
    state: State,
    trace: Vec<String>,
    /// Where questions go, once the worker exists.
    jobs: Option<Sender<Command>>,
    /// The answer in flight: its trace lines and its one result.
    run: Option<ActiveRun>,
    /// A scenario the model produced, waiting to be adopted by the window exactly once.
    adopt: Option<Scenario>,
    /// Complete document captured when the current request was accepted.
    base: Option<Scenario>,
    /// Keep the job identity until adoption, closing the reply/Stop race.
    adoption_token: Option<CancellationToken>,
    forget_after_stop: bool,
    /// How a responder is built.
    factory: Factory,
    spawn: WorkerSpawner,
    model_status: ModelStatus,
    /// How many questions the session has answered, for the panel's own readout.
    answered: usize,
}

impl Default for Assistant {
    fn default() -> Self {
        Self {
            open: false,
            question: String::new(),
            state: State::default(),
            trace: Vec::new(),
            jobs: None,
            run: None,
            adopt: None,
            base: None,
            adoption_token: None,
            forget_after_stop: false,
            factory: deepseek,
            spawn: spawn_worker,
            model_status,
            answered: 0,
        }
    }
}

impl Assistant {
    /// Whether a question is currently running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.run.is_some()
    }

    /// Start the worker, if it is not running.
    ///
    /// Started on the first question rather than when the panel opens: a window that is merely
    /// visible must not have made a network call, and a person who never uses the panel never pays
    /// for it.
    fn worker(&mut self, skills: &std::path::Path) -> Result<(), std::io::Error> {
        if self.jobs.is_some() {
            return Ok(());
        }
        let (tx, rx) = channel::<Command>();
        let factory = self.factory;
        let skills = skills.to_path_buf();
        (self.spawn)(Box::new(move || {
            // Built once per worker, so the conversation survives between questions. A failure
            // (no key, say) is reported per question rather than ending the thread: a person may
            // export a key and ask again without restarting the window.
            let mut responder: Option<Box<dyn Responder>> = None;
            let mut last_error: Option<String> = None;
            while let Ok(command) = rx.recv() {
                if matches!(command, Command::Forget) {
                    if let Some(active) = responder.as_mut() {
                        active.forget();
                    }
                    continue;
                }
                let Some(job) = command.job() else {
                    continue;
                };
                if job.token.is_cancelled() {
                    responder = None;
                    let _ = job
                        .result
                        .send(Err("The assistant request was cancelled.".to_owned()));
                    continue;
                }
                if responder.is_none() {
                    match factory(&skills, &job.scenario) {
                        Ok(built) => {
                            responder = Some(built);
                            last_error = None;
                        }
                        Err(message) => {
                            let _ = job.result.send(Err(message.clone()));
                            last_error = Some(message);
                            continue;
                        }
                    }
                }
                let Some(active) = responder.as_mut() else {
                    let message = last_error
                        .clone()
                        .unwrap_or_else(|| "the assistant could not start".to_owned());
                    let _ = job.result.send(Err(message));
                    continue;
                };
                let (scenario, question, trace, result) = (
                    job.scenario.clone(),
                    job.question.clone(),
                    job.trace.clone(),
                    job.result.clone(),
                );
                let control = RequestControl::new(job.token.clone());
                let mut outcome = active.ask_controlled(&scenario, &question, &trace, &control);
                if job.token.is_cancelled() {
                    outcome = Err("The assistant request was cancelled.".to_owned());
                }
                if outcome.is_err() {
                    // A failed turn may have left a partial conversation behind; the next question
                    // should start from a session whose history is still consistent.
                    responder = None;
                }
                let _ = result.send(outcome);
            }
        }))?;
        self.jobs = Some(tx);
        Ok(())
    }

    /// Ask the model, on the worker thread.
    ///
    /// The model is configured when the worker starts rather than when the app started: a person
    /// who exports a key while the window is open should not have to restart it.
    fn start(&mut self, scenario: &Scenario, skills: &std::path::Path) {
        if self.is_running() {
            return;
        }
        let question = self.question.trim().to_owned();
        if question.is_empty() {
            self.state = State::Failed("Type a question first.".to_owned());
            return;
        }
        self.base = None;
        self.adopt = None;
        self.adoption_token = None;
        self.forget_after_stop = false;
        if self.worker(skills).is_err() {
            self.state = State::WorkerUnavailable;
            return;
        }
        let Some(jobs) = &self.jobs else {
            self.state = State::Failed("The assistant could not start.".to_owned());
            return;
        };
        let (trace_tx, trace_rx) = channel();
        let (result_tx, result_rx) = channel();
        let token = CancellationToken::default();
        let job = Job {
            question,
            scenario: scenario.clone(),
            trace: trace_tx,
            result: result_tx,
            token: token.clone(),
        };
        if jobs.send(Command::Ask(Box::new(job))).is_err() {
            self.jobs = None;
            self.state =
                State::Failed("The assistant stopped; ask again to restart it.".to_owned());
            return;
        }
        self.base = Some(scenario.clone());
        self.trace.clear();
        self.state = State::Running;
        self.run = Some(ActiveRun {
            trace: trace_rx,
            result: result_rx,
            token,
        });
    }

    /// Cancel this job, retaining its receivers so a late success cannot become a new result.
    fn stop(&mut self) {
        if let Some(run) = &self.run {
            run.token.cancel();
        }
        if let Some(token) = self.adoption_token.take() {
            token.cancel();
        }
        if self.adopt.take().is_some() {
            self.answered = self.answered.saturating_sub(1);
        }
        self.base = None;
        self.state = if self.run.is_some() {
            State::Stopping
        } else {
            State::Cancelled
        };
    }

    /// Start a new conversation: empty the panel and let the worker forget.
    ///
    /// The scenario is kept. A discussion that went wrong is not a document that went wrong, and
    /// throwing away the satellites as well as the argument would make this button something nobody
    /// dares press.
    fn forget(&mut self) {
        self.stop();
        self.forget_after_stop = self.run.is_some();
        if let Some(jobs) = &self.jobs
            && jobs.send(Command::Forget).is_err()
        {
            // The worker is gone; the next question starts a new one anyway.
            self.jobs = None;
        }
        self.trace.clear();
        if self.run.is_none() {
            self.state = State::Idle;
        }
        self.answered = 0;
    }

    /// Drain whatever the worker has reported. Never blocks.
    fn poll(&mut self) {
        let Some(run) = &self.run else {
            return;
        };
        while let Ok(line) = run.trace.try_recv() {
            if !run.token.is_cancelled() {
                self.trace.push(line);
            }
        }
        let cancelled = run.token.is_cancelled();
        let token = run.token.clone();
        let result = run.result.try_recv();
        if cancelled && !matches!(&result, Err(TryRecvError::Empty)) {
            self.run = None;
            self.adopt = None;
            self.base = None;
            self.adoption_token = None;
            self.state = if self.forget_after_stop {
                State::Idle
            } else {
                State::Cancelled
            };
            self.forget_after_stop = false;
            return;
        }
        match result {
            Ok(Ok((text, scenario))) => {
                self.state = State::Answered { text };
                self.adopt = Some(scenario);
                self.adoption_token = Some(token);
                self.run = None;
                self.answered += 1;
            }
            Ok(Err(message)) => {
                self.state = State::Failed(message);
                self.base = None;
                self.run = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.base = None;
                self.state = State::Failed("The assistant stopped without an answer.".to_owned());
                self.run = None;
            }
        }
    }

    /// Consume a result only if the window still contains the request's complete base document.
    fn take_candidate(&mut self, current: &Scenario) -> Option<Scenario> {
        let candidate = self.adopt.take()?;
        let token = self.adoption_token.take()?;
        let base = self.base.take();
        if token.is_cancelled() {
            self.state = State::Cancelled;
            return None;
        }
        if base.as_ref() == Some(current) {
            Some(candidate)
        } else {
            self.state = State::Conflict;
            None
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
        translations: &stkcraft_framework_i18n::Translations,
        language: &str,
    ) -> Option<Scenario> {
        self.poll();
        // The panel translates its own chrome from the same catalogs the rest of the window uses;
        // the answer itself is the model's text and stays in whatever language it answered in.
        macro_rules! tr {
            ($source:literal) => {
                stkcraft_framework_i18n::tr!(translations, language, $source)
            };
        }
        ui.heading(tr!("Assistant"));
        if self.answered > 0 || self.is_running() {
            ui.horizontal(|ui| {
                ui.label(stkcraft_framework_i18n::trn!(
                    translations,
                    language,
                    "{n} answer in this conversation",
                    "{n} answers in this conversation",
                    self.answered as u64
                ));
                if ui
                    .button(tr!("New conversation"))
                    .on_hover_text(tr!("Forget the discussion; keep the scenario"))
                    .clicked()
                {
                    self.forget();
                }
            });
        }
        match (self.model_status)() {
            Ok((model, base_url)) => {
                let host = base_url
                    .split('/')
                    .nth(2)
                    .unwrap_or("the configured endpoint");
                ui.label(format!("{model} at {host}"));
            }
            Err(message) => {
                ui.colored_label(egui::Color32::from_rgb(230, 170, 120), message);
                ui.label(tr!(
                    "The assistant answers by running the engine's commands, so it needs a model \
                     to choose them."
                ));
            }
        }
        ui.separator();

        let running = self.is_running();
        let mut submit = false;
        ui.add_enabled_ui(!running, |ui| {
            let response = ui.add(
                egui::TextEdit::multiline(&mut self.question)
                    .desired_rows(3)
                    .hint_text(tr!("When can the station see the satellite?")),
            );
            // Enter asks; Shift+Enter starts a new line. A question is usually one line, and a
            // second keystroke to send it is a keystroke too many.
            submit = (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                || ui.button(tr!("Ask")).clicked();
        });
        if submit {
            self.start(scenario, skills);
        }

        if self.is_running() || self.adopt.is_some() {
            ui.horizontal(|ui| {
                // Keep Stop in the same position if a queued answer arrives in this frame.
                if ui
                    .add_enabled(
                        !matches!(self.state, State::Stopping),
                        egui::Button::new(tr!("Stop")),
                    )
                    .clicked()
                {
                    self.stop();
                }
                if self.is_running() {
                    ui.spinner();
                    if matches!(self.state, State::Stopping) {
                        ui.label(tr!("Stopping…"));
                    } else {
                        ui.label(tr!("Running the engine's commands…"));
                    }
                }
            });
        }

        if !self.trace.is_empty() {
            ui.separator();
            ui.label(tr!("What it did"));
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
            State::Idle | State::Running | State::Stopping => {}
            State::Cancelled => {
                ui.label(tr!(
                    "The assistant request was cancelled. Its result was not applied."
                ));
            }
            State::WorkerUnavailable => {
                ui.label(tr!("The assistant could not start its worker. Try again."));
            }
            State::Answered { text } => {
                ui.separator();
                ui.label(tr!("Answer"));
                ui.label(text);
            }
            State::Conflict => {
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(230, 170, 120), tr!("The scenario changed while the assistant was working. Its result was not applied. Ask again using the current scenario."));
            }
            State::Failed(message) => {
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(230, 120, 120), message);
            }
        }
        self.take_candidate(scenario)
    }
}

impl Drop for Assistant {
    fn drop(&mut self) {
        if let Some(run) = &self.run {
            run.token.cancel();
        }
        if let Some(token) = &self.adoption_token {
            token.cancel();
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn pending() -> (Assistant, Job, Scenario) {
        let base = Scenario::empty("window");
        let (jobs, receiver) = channel();
        let mut assistant = panel(twice);
        assistant.jobs = Some(jobs);
        assistant.question = "compute".into();
        assistant.start(&base, std::path::Path::new("skills"));
        let Ok(Command::Ask(job)) = receiver.recv() else {
            panic!("missing job")
        };
        // Keep the fake worker alive, as a real worker would be while computing.
        assistant.jobs = None;
        (assistant, *job, base)
    }

    #[test]
    fn stop_keeps_the_request_active_until_a_late_success_is_discarded() {
        let (mut assistant, job, base) = pending();
        assistant.stop();
        assert!(job.token.is_cancelled());
        assert!(assistant.is_running());
        assert!(matches!(assistant.state, State::Stopping));
        assistant.start(&base, std::path::Path::new("skills"));
        assert!(matches!(assistant.state, State::Stopping));
        assert!(
            job.result
                .send(Ok(("late".into(), Scenario::empty("unwanted"))))
                .is_ok()
        );
        assistant.poll();
        assert!(!assistant.is_running());
        assert!(matches!(assistant.state, State::Cancelled));
        assert!(assistant.take_candidate(&base).is_none());
        assert_eq!(assistant.answered, 0);
        assistant.start(&base, std::path::Path::new("skills"));
        settle(&mut assistant);
        assert!(assistant.take_candidate(&base).is_some());
    }

    #[test]
    fn stop_invalidates_success_already_waiting_for_adoption() {
        let (mut assistant, job, base) = pending();
        assert!(
            job.result
                .send(Ok(("queued".into(), Scenario::empty("changed"))))
                .is_ok()
        );
        assistant.poll();
        assert!(assistant.adopt.is_some());
        assistant.stop();
        assert!(job.token.is_cancelled());
        assert!(assistant.take_candidate(&base).is_none());
        assert_eq!(assistant.answered, 0);
        assert!(matches!(assistant.state, State::Cancelled));
    }

    #[test]
    fn adoption_rechecks_cancellation_even_after_a_successful_poll() {
        let (mut assistant, job, base) = pending();
        assert!(
            job.result
                .send(Ok(("queued".into(), Scenario::empty("changed"))))
                .is_ok()
        );
        assistant.poll();
        job.token.cancel();
        assert!(assistant.take_candidate(&base).is_none());
    }

    #[test]
    fn dropping_the_panel_cancels_its_active_job() {
        let (assistant, job, _) = pending();
        drop(assistant);
        assert!(job.token.is_cancelled());
    }

    #[test]
    fn worker_start_failure_leaves_no_job_or_candidate() {
        let mut assistant = panel(twice);
        assistant.spawn = |_| Err(std::io::Error::other("injected worker start failure"));
        assistant.question = "compute".into();
        assistant.start(&Scenario::empty("window"), std::path::Path::new("skills"));
        assert!(!assistant.is_running());
        assert!(assistant.jobs.is_none());
        assert!(assistant.base.is_none());
        assert!(assistant.adopt.is_none());
        assert!(matches!(assistant.state, State::WorkerUnavailable));
    }

    #[test]
    fn a_stop_click_discards_a_success_arriving_in_the_same_frame() {
        let (mut assistant, job, base) = pending();
        let ctx = egui::Context::default();
        let translations =
            stkcraft_framework_i18n::Translations::from_embedded(crate::catalogs::EMBEDDED);
        let mut frame = |events: Vec<egui::Event>| {
            let mut candidate = None;
            let capture = events.is_empty();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(960.0, 600.0),
                    )),
                    ..egui::RawInput::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        candidate = assistant.ui(
                            ui,
                            &base,
                            std::path::Path::new("skills"),
                            &translations,
                            "en",
                        );
                    });
                },
            );
            if capture
                && let Ok(path) = std::env::var("STKCRAFT_TEST_PANEL_PNG")
                && let Err(error) = crate::screenshot::scripted_ui(&ctx, output.clone(), &path)
            {
                panic!("could not render the scripted Stop panel: {error}");
            }
            output.textures_delta.clear();
            (candidate, output)
        };
        let (_, initial) = frame(Vec::new());
        let Some(position) = initial.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "Stop" => {
                Some(text.visual_bounding_rect().center())
            }
            _ => None,
        }) else {
            panic!("Stop was not drawn")
        };
        let button = |pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = frame(vec![egui::Event::PointerMoved(position), button(true)]);
        assert!(
            job.result
                .send(Ok(("ready".into(), Scenario::empty("changed"))))
                .is_ok()
        );
        let (candidate, _) = frame(vec![button(false)]);
        assert!(
            candidate.is_none(),
            "a success selected before Stop escaped cancellation"
        );
        assert!(
            job.token.is_cancelled(),
            "the Stop click did not reach the request"
        );
        assert!(matches!(assistant.state, State::Cancelled));
    }

    struct Waiting;

    impl Responder for Waiting {
        fn ask(&mut self, _: &Scenario, _: &str, _: &Sender<String>) -> Reply {
            Err("controlled responder required".to_owned())
        }

        fn ask_controlled(
            &mut self,
            scenario: &Scenario,
            _: &str,
            trace: &Sender<String>,
            control: &RequestControl,
        ) -> Reply {
            let _ = trace.send("entered controlled responder".to_owned());
            for _ in 0..3_000 {
                if control.check().is_err() {
                    return Ok(("late legacy success".to_owned(), scenario.clone()));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err("test responder did not receive cancellation".to_owned())
        }
    }

    #[test]
    fn stop_reaches_the_worker_and_discards_even_an_ok_reply() {
        let mut assistant = panel(|_, _| Ok(Box::new(Waiting)));
        let base = Scenario::empty("window");
        assistant.question = "compute".into();
        assistant.start(&base, std::path::Path::new("skills"));
        for _ in 0..3_000 {
            assistant.poll();
            if !assistant.trace.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            assistant.trace.first().map(String::as_str),
            Some("entered controlled responder")
        );
        assistant.stop();
        settle(&mut assistant);
        assert!(assistant.take_candidate(&base).is_none());
        assert!(matches!(assistant.state, State::Cancelled));
    }

    #[test]
    fn forgetting_discards_a_queued_success() {
        let base = Scenario::empty("window");
        let mut assistant = panel(twice);
        assistant.question = "compute".into();
        assistant.start(&base, std::path::Path::new("skills"));
        settle(&mut assistant);
        assistant.forget();
        assert_eq!(draw(&mut assistant, &base), None);
        assert_eq!(assistant.answered, 0);
    }

    #[test]
    fn forgetting_in_flight_discards_a_late_success() {
        let base = Scenario::empty("window");
        let (jobs, receiver) = channel();
        let mut assistant = panel(twice);
        assistant.jobs = Some(jobs);
        assistant.question = "compute".into();
        assistant.start(&base, std::path::Path::new("skills"));
        let Ok(Command::Ask(job)) = receiver.recv() else {
            panic!("missing job")
        };
        assistant.forget();
        assert!(
            job.result
                .send(Ok(("late".into(), Scenario::empty("unwanted"))))
                .is_ok()
        );
        assistant.poll();
        assert_eq!(assistant.take_candidate(&base), None);
        assert_eq!(assistant.answered, 0);
    }

    /// A responder that answers from a script, so the panel's threading, adoption and restart are
    /// exercised without a model, a key or a socket.
    struct Scripted {
        answers: std::cell::RefCell<Vec<Result<(String, Scenario), String>>>,
        questions: std::cell::RefCell<Vec<String>>,
    }

    impl Scripted {
        fn new(answers: Vec<Result<(String, Scenario), String>>) -> Self {
            Self {
                answers: std::cell::RefCell::new(answers),
                questions: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl Responder for Scripted {
        fn ask(
            &mut self,
            scenario: &Scenario,
            question: &str,
            trace: &Sender<String>,
        ) -> Result<(String, Scenario), String> {
            self.questions.borrow_mut().push(question.to_owned());
            let _ = trace.send(format!("access.compute({{}}) → asked `{question}`"));
            let mut answers = self.answers.borrow_mut();
            if answers.is_empty() {
                return Err("the script ran out".to_owned());
            }
            let answer = answers.remove(0);
            answer.map(|(text, mut produced)| {
                produced.name = scenario.name.clone();
                (text, produced)
            })
        }
    }

    /// A factory that answers the first question, then remembers it was asked twice, which is what
    /// proves the worker thread survives between questions instead of being rebuilt.
    #[allow(clippy::unnecessary_wraps)]
    fn twice(
        _skills: &std::path::Path,
        _scenario: &Scenario,
    ) -> Result<Box<dyn Responder>, String> {
        Ok(Box::new(Scripted::new(vec![
            Ok(("First answer.".to_owned(), Scenario::empty("worker"))),
            Ok(("Second answer.".to_owned(), Scenario::empty("worker"))),
        ])))
    }

    /// A factory whose model is missing, the way a missing key behaves.
    fn refuses(
        _skills: &std::path::Path,
        _scenario: &Scenario,
    ) -> Result<Box<dyn Responder>, String> {
        Err("DEEPSEEK_API_KEY is not set".to_owned())
    }

    // A Drop type cannot use struct-update syntax to move the default receivers.
    #[allow(clippy::field_reassign_with_default)]
    fn panel(factory: Factory) -> Assistant {
        let mut assistant = Assistant::default();
        assistant.factory = factory;
        assistant.model_status = || Err("Scripted responder; no provider connection.".to_owned());
        assistant
    }

    /// Wait for the worker to finish. The loop is bounded so a worker that never reports fails the
    /// test instead of hanging the suite.
    fn settle(assistant: &mut Assistant) {
        for _ in 0..3_000 {
            assistant.poll();
            if !assistant.is_running() {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("the panel never left the running state");
    }

    fn draw(assistant: &mut Assistant, scenario: &Scenario) -> Option<Scenario> {
        let ctx = egui::Context::default();
        let translations =
            stkcraft_framework_i18n::Translations::from_embedded(crate::catalogs::EMBEDDED);
        let mut candidate = None;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                candidate = assistant.ui(
                    ui,
                    scenario,
                    std::path::Path::new("skills"),
                    &translations,
                    "en",
                );
            });
        });
        output.textures_delta.clear();
        candidate
    }

    #[test]
    fn edits_during_request_survive_and_next_question_succeeds() {
        let base = Scenario::empty("window");
        let mut assistant = panel(twice);
        assistant.question = "compute".into();
        assistant.start(&base, std::path::Path::new("skills"));
        let mut current = base;
        current.duration_seconds += 60.0;
        let preserved = current.clone();
        settle(&mut assistant);
        assert_eq!(draw(&mut assistant, &current), None);
        assert_eq!(current, preserved);
        assert!(matches!(assistant.state, State::Conflict));
        assert!(assistant.base.is_none());
        assert_eq!(draw(&mut assistant, &current), None);
        assert!(matches!(assistant.state, State::Conflict));
        assistant.start(&current, std::path::Path::new("skills"));
        settle(&mut assistant);
        assert!(draw(&mut assistant, &current).is_some());
        assert_eq!(draw(&mut assistant, &current), None);
    }

    #[test]
    fn renamed_scenario_rejects_pending_result() {
        let base = Scenario::empty("original");
        let mut assistant = panel(twice);
        assistant.question = "compute".into();
        assistant.start(&base, std::path::Path::new("skills"));
        settle(&mut assistant);
        let mut current = base;
        current.name = "edited".into();
        assert_eq!(draw(&mut assistant, &current), None);
    }

    #[test]
    fn object_edits_reject_pending_result() {
        let Ok(base) = crate::app::demo_scenario() else {
            panic!("invalid demo");
        };
        let mut assistant = panel(twice);
        assistant.question = "compute".into();
        assistant.start(&base, std::path::Path::new("skills"));
        let mut current = base;
        let Some(satellite) = current.satellites.first_mut() else {
            panic!("demo must contain a satellite");
        };
        satellite.color = [1, 2, 3];
        settle(&mut assistant);
        assert_eq!(draw(&mut assistant, &current), None);
    }

    #[test]
    fn unchanged_scenario_accepts_once() {
        let base = Scenario::empty("window");
        let mut assistant = panel(twice);
        assistant.question = "compute".into();
        assistant.start(&base, std::path::Path::new("skills"));
        settle(&mut assistant);
        assert!(draw(&mut assistant, &base).is_some());
        assert_eq!(draw(&mut assistant, &base), None);
    }

    #[test]
    fn failed_enqueue_does_not_retain_a_base_or_candidate() {
        let (jobs, receiver) = channel();
        drop(receiver);
        let mut assistant = panel(twice);
        assistant.jobs = Some(jobs);
        assistant.question = "compute".into();
        assistant.start(&Scenario::empty("window"), std::path::Path::new("skills"));
        assert!(matches!(assistant.state, State::Failed(_)));
        assert!(assistant.base.is_none());
        assert!(assistant.adopt.is_none());
    }

    #[test]
    fn disconnected_reply_clears_accepted_base() {
        let (jobs, receiver) = channel();
        let mut assistant = panel(twice);
        assistant.jobs = Some(jobs);
        assistant.question = "compute".into();
        assistant.start(&Scenario::empty("window"), std::path::Path::new("skills"));
        assert!(assistant.base.is_some());
        drop(receiver);
        assistant.poll();
        assert!(matches!(assistant.state, State::Failed(_)));
        assert!(assistant.base.is_none());
        assert!(assistant.adopt.is_none());
    }

    #[test]
    fn two_questions_go_to_the_same_worker() {
        // The property that makes it a conversation: the second question must reach the responder
        // the first one used, or the history is thrown away between answers.
        let mut assistant = panel(twice);
        assistant.question = "When is the pass?".to_owned();
        assistant.start(&Scenario::empty("window"), std::path::Path::new("skills"));
        settle(&mut assistant);
        assistant.question = "And tomorrow?".to_owned();
        assistant.start(&Scenario::empty("window"), std::path::Path::new("skills"));
        settle(&mut assistant);

        match &assistant.state {
            State::Answered { text } => assert_eq!(text, "Second answer."),
            other => panic!("expected an answer, got {other:?}"),
        }
        assert_eq!(assistant.answered, 2);
        assert_eq!(assistant.trace.len(), 1);
        assert!(assistant.adopt.take().is_some());
    }

    #[test]
    fn a_new_conversation_forgets_the_answer_and_keeps_the_scenario() {
        let mut assistant = panel(twice);
        assistant.question = "When is the pass?".to_owned();
        assistant.start(&Scenario::empty("window"), std::path::Path::new("skills"));
        settle(&mut assistant);
        assert_eq!(assistant.answered, 1);
        assert!(assistant.adopt.take().is_some());

        assistant.forget();
        assert_eq!(assistant.answered, 0);
        assert_eq!(assistant.trace, Vec::<String>::new());
        match &assistant.state {
            State::Idle => {}
            other => panic!("expected an idle panel after forgetting, got {other:?}"),
        }
        // The next question starts a fresh exchange rather than resuming a stale one.
        assert!(!assistant.is_running());
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
        assert!(assistant.base.is_none());
        assert_eq!(assistant.adopt.take(), None);
    }

    #[test]
    fn an_empty_question_is_refused_without_starting_a_worker() {
        let mut assistant = panel(twice);
        assistant.question = "   ".to_owned();
        assistant.start(&Scenario::empty("window"), std::path::Path::new("skills"));
        assert!(!assistant.is_running());
        match &assistant.state {
            State::Failed(message) => assert!(message.contains("Type a question")),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod knowledge_tests {
    use super::*;
    #[test]
    fn initialization_attaches_library_without_model() {
        let mut server = session_server(
            std::path::Path::new("missing-skills"),
            &Scenario::empty("test"),
            || {
                let doc = stkcraft_agent_knowledge::Document {
                    path: "sensor.md".into(),
                    bytes: 40,
                    pages: None,
                    limited: false,
                    ocr_pages: 0,
                    formulas: vec![],
                    chunks: stkcraft_agent_knowledge::chunk::chunk(
                        "Maximum off-nadir angle is 45 degrees.",
                    ),
                };
                Ok(stkcraft_agent_knowledge::Library::from_documents(
                    std::path::Path::new("."),
                    vec![doc],
                ))
            },
        )
        .unwrap();
        let response = server.handle(&serde_json::json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":"knowledge.search", "arguments":{"query":"maximum off-nadir angle"}}})).unwrap();
        let text = response
            .pointer("/result/content/0/text")
            .and_then(serde_json::Value::as_str)
            .unwrap();
        assert!(text.contains("sensor.md:lines 1-1"), "{text}");
        assert!(text.contains("45 degrees"), "{text}");
        assert_eq!(
            response.pointer("/result/structuredContent/missed"),
            Some(&serde_json::json!(false))
        );
    }
    #[test]
    fn initialization_surfaces_loader_error() {
        let result = session_server(
            std::path::Path::new("missing-skills"),
            &Scenario::empty("test"),
            || Err("library unavailable".into()),
        );
        assert_eq!(result.err().unwrap(), "library unavailable");
    }
}
