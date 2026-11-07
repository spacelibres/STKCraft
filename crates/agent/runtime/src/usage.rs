//! Provider-reported telemetry; absent usage is unknown, never a zero-cost claim.

use serde::{Deserialize, Serialize};

/// Reported completion breakdown (unknown extensions are accepted).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionTokensDetails {
    /// Reasoning tokens, included in completion tokens rather than added to them.
    pub reasoning_tokens: Option<u64>,
}

/// Counters from the chat-completions usage object.
///
/// Source: <https://api-docs.deepseek.com/api/create-chat-completion> (2026-11-07).
/// Totals are provider telemetry, not independently computed token estimates.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "serde_json::Value")]
pub struct Usage {
    /// Input tokens, including cache hits.
    pub prompt_tokens: u64,
    /// Total output tokens, including reasoning where reported.
    pub completion_tokens: u64,
    /// Provider-reported request total.
    pub total_tokens: u64,
    /// Input tokens served from the provider cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_hit_tokens: Option<u64>,
    /// Input tokens not served from the provider cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_miss_tokens: Option<u64>,
    /// Optional reasoning breakdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

impl TryFrom<serde_json::Value> for Usage {
    type Error = String;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        let object = value
            .as_object()
            .ok_or_else(|| "usage must be an object".to_owned())?;
        let required = |name: &str| {
            object
                .get(name)
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format!("usage.{name} must be an unsigned integer"))
        };
        let optional = |name: &str| match object.get(name) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(value) => value
                .as_u64()
                .map(Some)
                .ok_or_else(|| format!("usage.{name} must be an unsigned integer or null")),
        };
        let completion_tokens_details = match object.get("completion_tokens_details") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) if value.is_object() => {
                Some(serde_json::from_value(value.clone()).map_err(|_| {
                    "usage.completion_tokens_details must contain unsigned counters".to_owned()
                })?)
            }
            Some(_) => return Err("usage.completion_tokens_details must be an object".to_owned()),
        };
        Ok(Self {
            prompt_tokens: required("prompt_tokens")?,
            completion_tokens: required("completion_tokens")?,
            total_tokens: required("total_tokens")?,
            prompt_cache_hit_tokens: optional("prompt_cache_hit_tokens")?,
            prompt_cache_miss_tokens: optional("prompt_cache_miss_tokens")?,
            completion_tokens_details,
        })
    }
}

/// Known token sums and explicit accounting gaps for one class of requests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageAggregate {
    /// Number of requests attempted, including failures.
    pub requests: u64,
    /// Requests which failed or returned unusable/truncated output.
    pub failed_requests: u64,
    /// Requests whose usage could not be recovered.
    pub unknown_usage_requests: u64,
    /// At least one counter exceeded u64; known sums are saturated.
    pub overflowed: bool,
    /// Sums for reported requests only. Optional breakdowns are absent unless all known reports contain them.
    pub known: Usage,
}

fn add(left: u64, right: u64, overflowed: &mut bool) -> u64 {
    if let Some(value) = left.checked_add(right) {
        value
    } else {
        *overflowed = true;
        u64::MAX
    }
}

impl UsageAggregate {
    /// Whether every attempt supplied usage and the sums are exact (failures may still cost tokens).
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.unknown_usage_requests == 0 && !self.overflowed
    }

    pub(crate) fn record(&mut self, usage: Option<&Usage>, failed: bool) {
        let known_before = self.requests.saturating_sub(self.unknown_usage_requests);
        self.requests = add(self.requests, 1, &mut self.overflowed);
        self.failed_requests = add(
            self.failed_requests,
            u64::from(failed),
            &mut self.overflowed,
        );
        let Some(usage) = usage else {
            self.unknown_usage_requests = add(self.unknown_usage_requests, 1, &mut self.overflowed);
            return;
        };
        self.known.prompt_tokens = add(
            self.known.prompt_tokens,
            usage.prompt_tokens,
            &mut self.overflowed,
        );
        self.known.completion_tokens = add(
            self.known.completion_tokens,
            usage.completion_tokens,
            &mut self.overflowed,
        );
        self.known.total_tokens = add(
            self.known.total_tokens,
            usage.total_tokens,
            &mut self.overflowed,
        );
        for (sum, value) in [
            (
                &mut self.known.prompt_cache_hit_tokens,
                usage.prompt_cache_hit_tokens,
            ),
            (
                &mut self.known.prompt_cache_miss_tokens,
                usage.prompt_cache_miss_tokens,
            ),
        ] {
            *sum = if known_before == 0 {
                value
            } else {
                sum.zip(value).map(|(a, b)| add(a, b, &mut self.overflowed))
            };
        }
        let previous = self
            .known
            .completion_tokens_details
            .as_ref()
            .and_then(|details| details.reasoning_tokens);
        let next = usage
            .completion_tokens_details
            .as_ref()
            .and_then(|details| details.reasoning_tokens);
        self.known.completion_tokens_details = if known_before == 0 {
            next
        } else {
            previous
                .zip(next)
                .map(|(a, b)| add(a, b, &mut self.overflowed))
        }
        .map(|reasoning_tokens| CompletionTokensDetails {
            reasoning_tokens: Some(reasoning_tokens),
        });
    }
}

/// Usage for one answer, including the separate summary requests it triggered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnswerUsage {
    /// Normal answer/tool-loop requests.
    pub model: UsageAggregate,
    /// Compaction summary requests, including failures and missing usage.
    pub compaction: UsageAggregate,
}
