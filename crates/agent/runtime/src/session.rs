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
    /// A note the harness wrote: compaction, so the reader knows the transcript has a past.
    Note(String),
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
            Self::Note(_) => None,
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
        Ok(Self {
            id,
            path,
            messages: Vec::new(),
            compacted: 0,
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
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(stored) = serde_json::from_str::<Stored>(line) else {
                continue;
            };
            if let Some(message) = stored.into_message() {
                messages.push(message);
            }
        }
        Ok(Self {
            id: id.to_owned(),
            path,
            messages,
            compacted: 0,
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
        let line = serde_json::to_string(&Stored::from(&message))
            .map_err(|error| format!("could not serialise a message: {error}"))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| format!("could not open {}: {error}", self.path.display()))?;
        writeln!(file, "{line}")
            .map_err(|error| format!("could not write {}: {error}", self.path.display()))?;
        self.messages.push(message);
        Ok(())
    }

    /// Write a note into the transcript: compaction happened, and how much.
    ///
    /// # Errors
    ///
    /// As [`Self::record`].
    pub fn note(&mut self, text: &str) -> Result<(), String> {
        use std::io::Write as _;
        let line = serde_json::to_string(&Stored::Note(text.to_owned()))
            .map_err(|error| format!("could not serialise a note: {error}"))?;
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
                Message::System(text) | Message::User(text) => text.chars().count(),
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
        if self.characters() <= CONTEXT_BUDGET_CHARS {
            return 0;
        }
        let system = self
            .messages
            .first()
            .filter(|message| matches!(message, Message::System(_)))
            .cloned();
        // The start of the oldest turn (a user message) and of the one after it.
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
        if keep_from == 0 {
            return 0;
        }
        let dropped = keep_from;
        let tail: Vec<Message> = self.messages.split_off(keep_from);
        self.messages = system.into_iter().chain(tail).collect();
        self.compacted += dropped;
        dropped
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
        // The file is the conversation, line by line.
        let text = std::fs::read_to_string(&session.path).expect("a transcript");
        assert_eq!(text.lines().count(), 5);
        assert!(text.contains("\"system\""), "{text}");

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
        // The drop is at a user message, so no tool result lost its call.
        assert!(
            matches!(session.messages.get(1), Some(Message::User(_))),
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
}
