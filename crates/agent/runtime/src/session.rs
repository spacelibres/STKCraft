//! A conversation that outlives the process.
//!
//! An analysis is not one question. "And the day after?" "What if the mask is 10°?" — a session is a
//! sequence of them, and a sequence that dies when the terminal closes is a sequence nobody can build
//! on. So a conversation is a **session**: an id, an append-only transcript on disk, and a resume.
//!
//! ```text
//! ~/.local/share/stkcraft/sessions/2026-11-06T18-31-07.jsonl
//! {"System":"You are STKCraft's analysis assistant. …"}
//! {"User":"When does the ISS pass over London?"}
//! {"Assistant":{"text":"","calls":[{"id":"call-1","name":"knowledge__search","arguments":{…}}]}}
//! {"Tool":{"call_id":"call-1","content":"…"}}
//! ```
//!
//! # Why one JSON object per line
//!
//! A transcript is written as it happens — a question can take a minute and a crash must not lose the
//! turns that already happened. An append-only line log is the format that survives truncation: a
//! half-written last line is dropped and everything before it is still a conversation.
//!
//! # The budget
//!
//! A model has a context window and a transcript grows past it. When the estimate exceeds
//! [`CONTEXT_BUDGET_CHARS`], the oldest *turns* are dropped — never half a turn, because a tool result
//! whose call is gone is a request a model API rejects — and a note records that they were dropped, so
//! the model knows the conversation has a past it cannot see rather than silently losing it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Message, ToolCall};

/// How many characters of conversation to keep before dropping the oldest turns.
///
/// A token is around four characters of English and less of Chinese; 120 000 characters is roughly
/// 30 000 tokens, which leaves room in a 64 k-token window for the system prompt, the skills a model
/// may read, and a long tool result.
pub const CONTEXT_BUDGET_CHARS: usize = 120_000;

/// How many turns survive a compaction, at least.
pub const KEEP_TURNS: usize = 6;

/// One message as it is stored: a tagged JSON object, so the file reads as a transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Stored {
    /// The instruction the model starts from.
    System(String),
    /// A question.
    User(String),
    /// An answer, possibly with tool calls.
    Assistant {
        /// Its text.
        text: String,
        /// The calls it asked for.
        calls: Vec<StoredCall>,
    },
    /// A tool's answer.
    Tool {
        /// The call being answered.
        call_id: String,
        /// Its output.
        content: String,
    },
    /// A summary the model wrote of turns that were compacted away.
    Summary(String),
    /// A note the harness wrote: compaction, so the reader knows the transcript has a past.
    Note(String),
    /// What this session is about, rewritten on every record so the last one wins.
    Meta {
        /// When it started.
        created: String,
        /// When it was last used.
        updated: String,
        /// A title, taken from the first question.
        title: Option<String>,
        /// The scenario it was about, when the caller said.
        scenario: Option<String>,
    },
}

/// What a session says about itself, for a list a person reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    /// When it started.
    pub created: String,
    /// When it was last used.
    pub updated: String,
    /// A title, taken from the first question.
    pub title: Option<String>,
    /// The scenario it was about, when the caller said.
    pub scenario: Option<String>,
}

/// One tool call as it is stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredCall {
    /// The provider's identifier.
    id: String,
    /// The tool name.
    name: String,
    /// Its arguments.
    arguments: serde_json::Value,
}

impl From<&Message> for Stored {
    fn from(message: &Message) -> Self {
        match message {
            Message::System(text) => Self::System(text.clone()),
            Message::Summary(text) => Self::Summary(text.clone()),
            Message::User(text) => Self::User(text.clone()),
            Message::Assistant { text, calls } => Self::Assistant {
                text: text.clone(),
                calls: calls
                    .iter()
                    .map(|call| StoredCall {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    })
                    .collect(),
            },
            Message::Tool { call_id, content } => Self::Tool {
                call_id: call_id.clone(),
                content: content.clone(),
            },
        }
    }
}

impl Stored {
    /// The message this line holds, when it is one the model sees.
    fn into_message(self) -> Option<Message> {
        match self {
            Self::System(text) => Some(Message::System(text)),
            Self::Summary(text) => Some(Message::Summary(text)),
            Self::User(text) => Some(Message::User(text)),
            Self::Assistant { text, calls } => Some(Message::Assistant {
                text,
                calls: calls
                    .into_iter()
                    .map(|call| ToolCall {
                        id: call.id,
                        name: call.name,
                        arguments: call.arguments,
                    })
                    .collect(),
            }),
            Self::Tool { call_id, content } => Some(Message::Tool { call_id, content }),
            // A note is written for a reader, not for the model: the transcript should show that
            // turns were dropped without the model re-reading the fact as a rule.
            Self::Note(_) | Self::Meta { .. } => None,
        }
    }
}

/// A conversation, and where it is stored.
#[derive(Debug, Clone)]
pub struct Session {
    /// The session's id, which is also its file name.
    pub id: String,
    /// Where the transcript is written.
    pub path: PathBuf,
    /// The messages the model reads, in order.
    pub messages: Vec<Message>,
    /// How many turns were dropped by the last compaction.
    pub compacted: usize,
    /// What this session says about itself: when it started, when it was last used, its title.
    pub meta: Meta,
}

impl Session {
    /// A new session in `directory`, named by the moment it started.
    ///
    /// # Errors
    ///
    /// A message when the directory cannot be created.
    pub fn create(directory: &Path) -> Result<Self, String> {
        let id = session_id();
        let path = directory.join(format!("{id}.jsonl"));
        std::fs::create_dir_all(directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
        let now = session_id();
        Ok(Self {
            id,
            path,
            messages: Vec::new(),
            compacted: 0,
            meta: Meta {
                created: now.clone(),
                updated: now,
                title: None,
                scenario: None,
            },
        })
    }

    /// Open a session by id, reading its transcript.
    ///
    /// A line that cannot be read is skipped, not fatal: a crash during a write must cost the last
    /// line, not the conversation.
    ///
    /// # Errors
    ///
    /// A message when the directory or the file cannot be read.
    pub fn open(directory: &Path, id: &str) -> Result<Self, String> {
        let path = directory.join(format!("{id}.jsonl"));
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        let mut messages = Vec::new();
        let mut meta = Meta::default();
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(stored) = serde_json::from_str::<Stored>(line) else {
                continue;
            };
            // The last `Meta` line wins: the file is a log, and the newest statement about the
            // session is the one that describes it.
            if let Stored::Meta {
                created,
                updated,
                title,
                scenario,
            } = &stored
            {
                meta = Meta {
                    created: created.clone(),
                    updated: updated.clone(),
                    title: title.clone(),
                    scenario: scenario.clone(),
                };
            }
            if let Some(message) = stored.into_message() {
                messages.push(message);
            }
        }
        Ok(Self {
            id: id.to_owned(),
            path,
            messages,
            compacted: 0,
            meta,
        })
    }

    /// The ids of the sessions in a directory, newest first.
    #[must_use]
    pub fn list(directory: &Path) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".jsonl").map(str::to_owned)
            })
            .collect();
        // The id is a timestamp, so the reverse order is newest first.
        ids.sort();
        ids.reverse();
        ids
    }

    /// Append a message: to the conversation and to the file.
    ///
    /// # Errors
    ///
    /// A message when the transcript cannot be written. The conversation keeps the message either
    /// way — a failed write costs the resume, not the answer.
    pub fn record(&mut self, message: Message) -> Result<(), String> {
        use std::io::Write as _;
        // The title is the first thing the person asked: a session list of timestamps tells nobody
        // which conversation is which.
        if self.meta.title.is_none()
            && let Message::User(text) = &message
        {
            let title: String = text.chars().take(60).collect();
            self.meta.title = Some(title.replace('\n', " "));
        }
        self.meta.updated = session_id();
        let line = serde_json::to_string(&Stored::from(&message))
            .map_err(|error| format!("could not serialise a message: {error}"))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| format!("could not open {}: {error}", self.path.display()))?;
        writeln!(file, "{line}")
            .map_err(|error| format!("could not write {}: {error}", self.path.display()))?;
        if let Ok(meta) = serde_json::to_string(&Stored::Meta {
            created: self.meta.created.clone(),
            updated: self.meta.updated.clone(),
            title: self.meta.title.clone(),
            scenario: self.meta.scenario.clone(),
        }) {
            let _ = writeln!(file, "{meta}");
        }
        self.messages.push(message);
        Ok(())
    }

    /// Write a note into the transcript: compaction happened, and how much.
    ///
    /// # Errors
    ///
    /// As [`Self::record`].
    pub fn note(&mut self, text: &str) -> Result<(), String> {
        self.write_stored(&Stored::Note(text.to_owned()))
    }

    /// Append one stored line to the transcript.
    fn write_stored(&mut self, stored: &Stored) -> Result<(), String> {
        use std::io::Write as _;
        let line = serde_json::to_string(stored)
            .map_err(|error| format!("could not serialise a transcript line: {error}"))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| format!("could not open {}: {error}", self.path.display()))?;
        writeln!(file, "{line}")
            .map_err(|error| format!("could not write {}: {error}", self.path.display()))?;
        Ok(())
    }

    /// How many characters of conversation the messages hold, which is the budget's unit.
    #[must_use]
    pub fn characters(&self) -> usize {
        self.messages
            .iter()
            .map(|message| match message {
                Message::System(text) | Message::Summary(text) | Message::User(text) => {
                    text.chars().count()
                }
                Message::Assistant { text, calls } => {
                    text.chars().count()
                        + calls
                            .iter()
                            .map(|call| call.name.len() + call.arguments.to_string().len())
                            .sum::<usize>()
                }
                Message::Tool { content, .. } => content.chars().count(),
            })
            .sum()
    }

    /// Drop the oldest turns until the conversation fits the budget.
    ///
    /// The system prompt stays, the last [`KEEP_TURNS`] turns stay, and the drop happens at a *user*
    /// message so a tool result never loses the call it answers. Returns how many messages went.
    pub fn compact(&mut self) -> usize {
        let Some(dropped) = self.plan_compaction() else {
            return 0;
        };
        self.apply_compaction(&dropped, None)
    }

    /// The messages a compaction would drop, or `None` when the conversation fits.
    ///
    /// This is the span the model is asked to summarise, so it has to be decided before anything is
    /// removed: a summary written after the drop would describe a transcript nobody kept.
    #[must_use]
    pub fn plan_compaction(&self) -> Option<Vec<Message>> {
        if self.characters() <= CONTEXT_BUDGET_CHARS {
            return None;
        }
        let starts: Vec<usize> = self
            .messages
            .iter()
            .enumerate()
            .filter(|(_, message)| matches!(message, Message::User(_)))
            .map(|(index, _)| index)
            .collect();
        let keep_from = starts
            .get(starts.len().saturating_sub(KEEP_TURNS))
            .copied()
            .unwrap_or(0);
        // The pinned prefix — the system prompt and any earlier summary — is never part of the span.
        let pinned = self
            .messages
            .iter()
            .take_while(|message| matches!(message, Message::System(_) | Message::Summary(_)))
            .count();
        (keep_from > pinned).then(|| {
            self.messages
                .iter()
                .skip(pinned)
                .take(keep_from - pinned)
                .cloned()
                .collect()
        })
    }

    /// Replace the planned span with a summary, and return how many messages went.
    ///
    /// `summary` is the model's own account of the span; `None` keeps a note that says a span was
    /// dropped, which is the honest fallback when the summary request failed.
    pub fn apply_compaction(&mut self, dropped: &[Message], summary: Option<&str>) -> usize {
        let pinned = self
            .messages
            .iter()
            .take_while(|message| matches!(message, Message::System(_) | Message::Summary(_)))
            .cloned()
            .collect::<Vec<_>>();
        let remaining = dropped.len();
        if remaining == 0 {
            return 0;
        }
        let tail = self.messages.split_off(pinned.len() + remaining);
        let replacement = match summary.map(str::trim).filter(|text| !text.is_empty()) {
            Some(text) => vec![Message::Summary(text.to_owned())],
            None => vec![Message::Summary(format!(
                "{remaining} earlier message(s) of this conversation were condensed to fit the \
                 context window, and the summary of them is not available."
            ))],
        };
        // The summary is part of the transcript, not just of the conversation in memory: a resumed
        // session has to start with the same memory a live one has, or the second session is amnesiac
        // about everything the first one compacted.
        for message in &replacement {
            if let Message::Summary(text) = message {
                let _ = self.write_stored(&Stored::Summary(text.clone()));
            }
        }
        self.messages = pinned.into_iter().chain(replacement).chain(tail).collect();
        self.compacted += remaining;
        remaining
    }
    /// The note that tells the model its transcript has a past.
    #[must_use]
    pub fn compaction_note(&self) -> String {
        format!(
            "{} earlier message(s) of this conversation were condensed to fit the context window. \
             Ask again if you need something from them.",
            self.compacted
        )
    }
}

/// A session id from the clock: sortable, readable, and unique enough for one person's history.
#[must_use]
pub fn session_id() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let days = seconds / 86_400;
    let of_day = seconds % 86_400;
    let (year, month, day) = crate::session::civil_from_days(i64::try_from(days).unwrap_or(0));
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}-{:02}-{:02}",
        of_day / 3_600,
        (of_day % 3_600) / 60,
        of_day % 60
    )
}

/// Days since 1970-01-01 to a Gregorian date, after Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_piece = (5 * day_of_year + 2) / 153;
    let day = u32::try_from(day_of_year - (153 * month_piece + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if month_piece < 10 {
        month_piece + 3
    } else {
        month_piece - 9
    })
    .unwrap_or(1);
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a directory");
        directory
    }

    fn exchange(session: &mut Session, question: &str) {
        session
            .record(Message::User(question.to_owned()))
            .expect("recorded");
        session
            .record(Message::Assistant {
                text: String::new(),
                calls: vec![ToolCall {
                    id: "call-1".to_owned(),
                    name: "knowledge__search".to_owned(),
                    arguments: json!({"query": "off-nadir"}),
                }],
            })
            .expect("recorded");
        session
            .record(Message::Tool {
                call_id: "call-1".to_owned(),
                content: "a passage".to_owned(),
            })
            .expect("recorded");
        session
            .record(Message::Assistant {
                text: "45 degrees".to_owned(),
                calls: Vec::new(),
            })
            .expect("recorded");
    }

    #[test]
    fn a_session_is_written_as_it_happens_and_reads_back() {
        let directory = scratch("stkcraft-session-roundtrip");
        let mut session = Session::create(&directory).expect("created");
        session
            .record(Message::System("be careful".to_owned()))
            .expect("recorded");
        exchange(&mut session, "What is the angle?");
        // The file is the conversation, line by line, with a `meta` line after each message so a
        // session list can say what the conversation was about.
        let text = std::fs::read_to_string(&session.path).expect("a transcript");
        assert_eq!(text.lines().count(), 10);
        assert!(text.contains("\"system\""), "{text}");
        assert!(text.contains("\"meta\""), "{text}");
        assert!(text.contains("What is the angle?"), "{text}");

        let reopened = Session::open(&directory, &session.id).expect("opened");
        assert_eq!(reopened.messages, session.messages);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_crash_during_a_write_costs_the_last_line_not_the_conversation() {
        let directory = scratch("stkcraft-session-truncated");
        let mut session = Session::create(&directory).expect("created");
        exchange(&mut session, "first");
        let mut text = std::fs::read_to_string(&session.path).expect("a transcript");
        // Simulate a write that stopped halfway through the last line.
        text.push_str("{\"user\":\"a question that nev");
        std::fs::write(&session.path, text).expect("written");
        let reopened = Session::open(&directory, &session.id).expect("opened");
        assert_eq!(reopened.messages.len(), 4, "{:?}", reopened.messages);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn sessions_are_listed_newest_first() {
        let directory = scratch("stkcraft-session-list");
        for id in ["2026-01-01T00-00-00", "2026-02-01T00-00-00"] {
            std::fs::write(directory.join(format!("{id}.jsonl")), "").expect("written");
        }
        assert_eq!(
            Session::list(&directory),
            vec![
                "2026-02-01T00-00-00".to_owned(),
                "2026-01-01T00-00-00".to_owned()
            ]
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn compaction_keeps_the_system_prompt_and_the_recent_turns() {
        let directory = scratch("stkcraft-session-compact");
        let mut session = Session::create(&directory).expect("created");
        session
            .record(Message::System("be careful".to_owned()))
            .expect("recorded");
        // Enough long turns to exceed the budget.
        for index in 0..(KEEP_TURNS + 4) {
            let filler = "x".repeat(CONTEXT_BUDGET_CHARS / 4);
            session
                .record(Message::User(format!("question {index} {filler}")))
                .expect("recorded");
            session
                .record(Message::Assistant {
                    text: format!("answer {index}"),
                    calls: Vec::new(),
                })
                .expect("recorded");
        }
        let before = session.messages.len();
        let dropped = session.compact();
        assert!(dropped > 0, "nothing was dropped");
        assert!(session.messages.len() < before);
        // The system prompt survives, and the newest turn is intact.
        assert!(matches!(session.messages.first(), Some(Message::System(_))));
        assert!(matches!(
            session.messages.last(),
            Some(Message::Assistant { text, .. }) if text.contains("answer 9")
        ));
        // The dropped span is replaced by a summary, and the conversation after it resumes at a
        // question — so no tool result lost the call it answers.
        assert!(
            matches!(session.messages.get(1), Some(Message::Summary(_))),
            "the dropped span left no summary"
        );
        assert!(
            matches!(session.messages.get(2), Some(Message::User(_))),
            "the conversation does not resume at a question"
        );
        assert!(session.compaction_note().contains("condensed"));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_short_conversation_is_not_compacted() {
        let directory = scratch("stkcraft-session-short");
        let mut session = Session::create(&directory).expect("created");
        exchange(&mut session, "a question");
        assert_eq!(session.compact(), 0);
        assert_eq!(session.compacted, 0);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_session_id_sorts_and_reads_as_a_time() {
        let id = session_id();
        assert_eq!(id.len(), 19, "{id}");
        assert!(id.starts_with("20"), "{id}");
        let other = "2026-02-01T00-00-00";
        assert!(
            id.as_str() > other || id.as_str() < other,
            "ids compare as text"
        );
    }

    #[test]
    fn the_summary_replaces_the_span_and_keeps_the_facts_the_model_chose() {
        // The whole point of compaction: what was dropped is *described*, not erased. A summary the
        // model wrote is the only record of the span, so it has to survive as a message the model
        // reads on the next request — and the span it replaces has to be decided before it is written.
        let directory = scratch("stkcraft-session-summary");
        let mut session = Session::create(&directory).expect("created");
        session
            .record(Message::System("be careful".to_owned()))
            .expect("recorded");
        let filler = "y".repeat(CONTEXT_BUDGET_CHARS / 4);
        // More turns than `KEEP_TURNS`, so there is a span older than the ones that survive.
        for index in 0..(KEEP_TURNS + 4) {
            session
                .record(Message::User(format!("question {index} {filler}")))
                .expect("recorded");
            session
                .record(Message::Assistant {
                    text: format!("answer {index}"),
                    calls: Vec::new(),
                })
                .expect("recorded");
        }
        let span = session.plan_compaction().expect("a span to drop");
        assert_ne!(span.len(), 0, "an empty span was planned");
        // Nothing is removed until the summary exists: the span is a plan, not a deletion.
        let before = session.messages.len();
        let dropped = session.apply_compaction(&span, Some("the sensor swath is 1200 km"));
        assert_eq!(dropped, span.len());
        assert!(session.messages.len() < before);
        assert!(matches!(session.messages.first(), Some(Message::System(_))));
        assert!(
            matches!(
                session.messages.get(1),
                Some(Message::Summary(text)) if text.contains("1200 km")
            ),
            "the summary did not survive: {:?}",
            session.messages.get(1)
        );
        // A resumed session reads the same summary back from the transcript.
        session
            .record(Message::User("and the swath?".to_owned()))
            .expect("recorded");
        let reopened = Session::open(&directory, &session.id).expect("opened");
        assert!(
            reopened.messages.iter().any(
                |message| matches!(message, Message::Summary(text) if text.contains("1200 km"))
            ),
            "the summary was not in the transcript"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }
}
