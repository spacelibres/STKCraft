//! Cooperative cancellation and sanitized model failures.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
/// Shared cooperative cancellation flag.
#[derive(Clone, Default, Debug)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    /// Signal cancellation to every clone.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
/// Safe classifications only: no provider payloads, URLs, credentials or OS messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelError {
    /// Invalid configuration; verify timeout, endpoint and protocol options.
    Configuration,
    /// Output token limit must be nonzero.
    InvalidOutputLimit,
    /// Thinking enabled requires the `DeepSeek` protocol.
    ThinkingProtocol,
    /// Retained tool history lacks the reasoning required for thinking replay.
    MissingHistoryReasoning,
    /// Provider terminated output at its length limit.
    TruncatedOutput,
    /// Could not start curl; verify curl is installed.
    Spawn,
    /// Could not start a pipe worker.
    ThreadStart,
    /// A subprocess pipe was unavailable.
    Pipe,
    /// Sending the request failed.
    Stdin,
    /// Reading the response failed.
    Read,
    /// Inspecting subprocess exit failed.
    Wait,
    /// The request deadline elapsed.
    Timeout,
    /// The caller cancelled the request.
    Cancelled,
    /// The endpoint returned this HTTP status.
    HttpStatus(u16),
    /// Response exceeds the application limit.
    ResponseTooLarge,
    /// Escaped request exceeds the application limit.
    RequestTooLarge,
    /// Invalid response protocol.
    Protocol,
    /// A legacy model failed without safe typed diagnostics.
    ModelFailed,
    /// The operation is unavailable on this platform.
    Unsupported,
    /// Cleanup exceeded its bounded grace; a process or pipe worker remains unresolved.
    Cleanup,
}
impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::InvalidOutputLimit => "max_output_tokens must be nonzero",
            Self::ThinkingProtocol => "thinking enabled requires the deepseek protocol",
            Self::MissingHistoryReasoning => {
                "cannot resume DeepSeek thinking with assistant tool calls missing reasoning_content; start a new session or explicitly select --thinking disabled"
            }
            Self::TruncatedOutput => {
                "model output was truncated; no tool calls from this response executed; increase max_output_tokens and retry"
            }
            Self::Configuration => {
                "invalid model configuration; check endpoint, timeout and protocol options"
            }
            Self::Spawn => "could not start curl; verify curl is installed",
            Self::ThreadStart => "could not start model pipe worker",
            Self::Pipe => "model subprocess pipe unavailable",
            Self::Stdin => "could not send model request",
            Self::Read => "could not read model response",
            Self::Wait => "could not inspect model subprocess",
            Self::Timeout => "model request timed out",
            Self::Cancelled => "model request cancelled",
            Self::HttpStatus(status) => return write!(f, "model endpoint returned HTTP {status}"),
            Self::ResponseTooLarge => "model response exceeds 8 MiB",
            Self::RequestTooLarge => "model request exceeds 8 MiB",
            Self::Protocol => {
                "invalid model response protocol; no tool calls from this response executed"
            }
            Self::ModelFailed => "model did not complete the request",
            Self::Unsupported => "model transport is unsupported on this platform",
            Self::Cleanup => "model request cleanup unresolved after grace period",
        };
        f.write_str(text)
    }
}
impl std::error::Error for ModelError {}
/// A cancellation token and an optional monotonic deadline, preserved across model requests.
#[derive(Clone, Debug, Default)]
pub struct RequestControl {
    token: CancellationToken,
    deadline: Option<Instant>,
}
impl RequestControl {
    /// Construct without consulting the platform clock.
    #[must_use]
    pub fn new(token: CancellationToken) -> Self {
        Self {
            token,
            deadline: None,
        }
    }
    /// Set a positive timeout. Browsers return Unsupported before consulting the clock.
    /// # Errors
    /// Unsupported platform, zero timeout, or deadline overflow.
    pub fn with_timeout(token: CancellationToken, timeout: Duration) -> Result<Self, ModelError> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (token, timeout);
            Err(ModelError::Unsupported)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if timeout.is_zero() {
                return Err(ModelError::Configuration);
            }
            let deadline = Instant::now()
                .checked_add(timeout)
                .ok_or(ModelError::Configuration)?;
            Ok(Self {
                token,
                deadline: Some(deadline),
            })
        }
    }
    /// Check cancellation before the deadline. A control without deadline needs no clock.
    /// # Errors
    /// Cancelled or Timeout (Unsupported for a browser deadline).
    pub fn check(&self) -> Result<(), ModelError> {
        if self.token.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        if let Some(deadline) = self.deadline {
            #[cfg(target_arch = "wasm32")]
            {
                let _ = deadline;
                return Err(ModelError::Unsupported);
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                if Instant::now() >= deadline {
                    return Err(ModelError::Timeout);
                }
            }
        }
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn bounded(&self, timeout: Duration) -> Result<Self, ModelError> {
        let mut bounded = Self::with_timeout(self.token.clone(), timeout)?;
        if let Some(deadline) = self.deadline {
            bounded.deadline = Some(bounded.deadline.map_or(deadline, |own| own.min(deadline)));
        }
        Ok(bounded)
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn remaining(&self) -> Result<Duration, ModelError> {
        self.check()?;
        self.deadline
            .and_then(|d| d.checked_duration_since(Instant::now()))
            .filter(|d| !d.is_zero())
            .ok_or(ModelError::Timeout)
    }
}
