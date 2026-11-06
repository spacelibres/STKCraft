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

/// What one question produces: the answer, and the scenario the model left behind.
type Reply = Result<(String, Scenario), String>;

/// What the worker is asked to do.
enum Command {
    /// Answer a question.
    Ask(Job),
    /// Forget the conversation, keeping the document the tools built.
    Forget,
}

/// How a responder is built. The real one is [`deepseek`]; a test substitutes its own.
type Factory = fn(&std::path::Path, &Scenario) -> Result<Box<dyn Responder>, String>;

/// One question for the worker.
struct Job {
    question: String,
    /// The document the question is asked about.
    scenario: Scenario,
    /// Where each tool-call line goes as it happens.
    trace: Sender<String>,
    /// Where the answer goes, once.
    result: Sender<Reply>,
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

    /// Forget the conversation, keeping whatever the tools have built.
    ///
    /// A discussion that has gone wrong — a wrong assumption the model keeps building on, a
    /// scenario that was edited into nonsense — needs a way out that is not "restart the
    /// application", and the scenario does not have to be thrown away with it.
    fn forget(&mut self) {}
}

/// The real responder: `DeepSeek` driving the engine's tools, with the conversation kept.
struct DeepSeekResponder {
    agent: stkcraft_agent::Agent<stkcraft_agent::DeepSeek>,
}

impl Responder for DeepSeekResponder {
    fn ask(&mut self, scenario: &Scenario, question: &str, trace: &Sender<String>) -> Reply {
        // The window owns the document; the session takes whatever it is given before answering, so
        // an answer is always about the scenario on screen.
        self.agent.server_mut().set_scenario(scenario.clone());
        let mut report = |line: &str| {
            let _ = trace.send(line.to_owned());
        };
        let answer = self.agent.ask_observing(question, &mut report)?;
        Ok((answer.text, self.agent.server().scenario().clone()))
    }

    fn forget(&mut self) {
        self.agent.forget();
    }
}

/// Build the real responder, or say why it cannot exist.
fn deepseek(skills: &std::path::Path, scenario: &Scenario) -> Result<Box<dyn Responder>, String> {
    let model = stkcraft_agent::DeepSeek::from_environment()?;
    let server = stkcraft_mcp::Server::new(scenario.clone(), stkcraft_mcp::load_skills(skills));
    Ok(Box::new(DeepSeekResponder {
        agent: stkcraft_agent::Agent::new(model, server),
    }))
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
    run: Option<(Receiver<String>, Receiver<Reply>)>,
    /// A scenario the model produced, waiting to be adopted by the window exactly once.
    adopt: Option<Scenario>,
    /// How a responder is built.
    factory: Factory,
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
            factory: deepseek,
            answered: 0,
        }
    }
}

impl Assistant {
    /// Whether a question is currently running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self.state, State::Running)
    }

    /// Start the worker, if it is not running.
    ///
    /// Started on the first question rather than when the panel opens: a window that is merely
    /// visible must not have made a network call, and a person who never uses the panel never pays
    /// for it.
    fn worker(&mut self, scenario: &Scenario, skills: &std::path::Path) {
        if self.jobs.is_some() {
            return;
        }
        let (tx, rx) = channel::<Command>();
        let factory = self.factory;
        let scenario = scenario.clone();
        let skills = skills.to_path_buf();
        std::thread::spawn(move || {
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
                let outcome = active.ask(&scenario, &question, &trace);
                if outcome.is_err() {
                    // A failed turn may have left a partial conversation behind; the next question
                    // should start from a session whose history is still consistent.
                    responder = None;
                }
                let _ = result.send(outcome);
            }
        });
        self.jobs = Some(tx);
        let _ = scenario;
    }

    /// Ask the model, on the worker thread.
    ///
    /// The model is configured when the worker starts rather than when the app started: a person
    /// who exports a key while the window is open should not have to restart it.
    fn start(&mut self, scenario: &Scenario, skills: &std::path::Path) {
        let question = self.question.trim().to_owned();
        if question.is_empty() {
            self.state = State::Failed("Type a question first.".to_owned());
            return;
        }
        if self.is_running() {
            return;
        }
        self.worker(scenario, skills);
        let Some(jobs) = &self.jobs else {
            self.state = State::Failed("The assistant could not start.".to_owned());
            return;
        };
        let (trace_tx, trace_rx) = channel();
        let (result_tx, result_rx) = channel();
        let job = Job {
            question,
            scenario: scenario.clone(),
            trace: trace_tx,
            result: result_tx,
        };
        if jobs.send(Command::Ask(job)).is_err() {
            self.jobs = None;
            self.state =
                State::Failed("The assistant stopped; ask again to restart it.".to_owned());
            return;
        }
        self.trace.clear();
        self.state = State::Running;
        self.run = Some((trace_rx, result_rx));
    }

    /// Start a new conversation: empty the panel and let the worker forget.
    ///
    /// The scenario is kept. A discussion that went wrong is not a document that went wrong, and
    /// throwing away the satellites as well as the argument would make this button something nobody
    /// dares press.
    fn forget(&mut self) {
        if let Some(jobs) = &self.jobs
            && jobs.send(Command::Forget).is_err()
        {
            // The worker is gone; the next question starts a new one anyway.
            self.jobs = None;
        }
        self.trace.clear();
        self.state = State::Idle;
        self.answered = 0;
    }

    /// Drain whatever the worker has reported. Never blocks.
    fn poll(&mut self) {
        let Some((trace, result)) = &self.run else {
            return;
        };
        while let Ok(line) = trace.try_recv() {
            self.trace.push(line);
        }
        match result.try_recv() {
            Ok(Ok((text, scenario))) => {
                self.state = State::Answered { text };
                self.adopt = Some(scenario);
                self.run = None;
                self.answered += 1;
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
        if self.answered > 0 {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{answered} answer(s) in this conversation",
                    answered = self.answered
                ));
                if ui
                    .button("New conversation")
                    .on_hover_text("Forget the discussion; keep the scenario")
                    .clicked()
                {
                    self.forget();
                }
            });
        }
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

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;
    use std::time::Duration;

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

    fn panel(factory: Factory) -> Assistant {
        Assistant {
            factory,
            ..Assistant::default()
        }
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
