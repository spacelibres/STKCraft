//! The one model call the developer tasks make.
//!
//! Translation — of interface catalogs and of the generated API reference — is a single completion:
//! send the strings, ask for JSON, read the JSON back. It is not the assistant's loop and it does not
//! belong in the app: this runs in `xtask`, on a developer's machine, with the developer's key.
//!
//! The transport is `curl` for the same reasons the assistant uses it — no TLS stack in the build, no
//! HTTP client to audit, and a request and response a person can read when a translation looks wrong.

/// The first JSON object in a model's answer, tolerating a fenced code block.
pub fn json_object_in(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end < start {
        return None;
    }
    serde_json::from_str(&trimmed[start..=end]).ok()
}

/// One translation completion through the shared provider and its protocol resolver.
pub fn chat_completion(key: &str, prompt: &str) -> Result<String, String> {
    let base_url = std::env::var("DEEPSEEK_BASE_URL")
        .unwrap_or_else(|_| stkcraft_agent_runtime::DEEPSEEK_BASE_URL.to_owned());
    let model = std::env::var("DEEPSEEK_MODEL")
        .unwrap_or_else(|_| stkcraft_agent_runtime::DEEPSEEK_MODEL.to_owned());
    let protocol = std::env::var("DEEPSEEK_PROTOCOL").ok();
    let thinking = std::env::var("DEEPSEEK_THINKING").ok();
    let provider = translation_provider(
        key,
        &base_url,
        &model,
        protocol.as_deref(),
        thinking.as_deref(),
    )?;
    completion_with(&provider, prompt)
}

fn translation_provider(
    key: &str,
    base_url: &str,
    model: &str,
    protocol: Option<&str>,
    thinking: Option<&str>,
) -> Result<stkcraft_agent_runtime::Provider, String> {
    use stkcraft_agent_runtime::{Provider, ProviderOptions, ProviderProtocol, ThinkingMode};
    let mut options = ProviderOptions::resolve(base_url, protocol, thinking)?;
    // Translation is a single JSON task, so the role explicitly selects ordinary mode.
    if options.protocol == ProviderProtocol::DeepSeek {
        options.thinking = ThinkingMode::Disabled;
    }
    let mut provider = Provider::new(base_url, model, key).with_options(options);
    provider.timeout_seconds = 300;
    Ok(provider)
}

fn completion_with(
    model: &impl stkcraft_agent_runtime::ChatModel,
    prompt: &str,
) -> Result<String, String> {
    use stkcraft_agent_runtime::Message;
    model.complete(&[
        Message::System("You translate software interfaces. You answer with JSON only, exactly as asked.".to_owned()),
        Message::User(prompt.to_owned()),
    ], &[]).map(|turn| turn.text)
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn translation_uses_explicit_ordinary_mode_and_preserves_custom_protocols() {
        use stkcraft_agent_runtime::{ProviderProtocol, ThinkingMode};
        let provider = translation_provider(
            "test-only-key",
            stkcraft_agent_runtime::DEEPSEEK_BASE_URL,
            stkcraft_agent_runtime::DEEPSEEK_MODEL,
            None,
            Some("enabled"),
        )
        .expect("translation provider");
        assert_eq!(provider.options.protocol, ProviderProtocol::DeepSeek);
        assert_eq!(provider.options.thinking, ThinkingMode::Disabled);
        assert_eq!(provider.timeout_seconds, 300);
        let provider = translation_provider(
            "test-only-key",
            "https://proxy.invalid/completions",
            "chosen-model",
            None,
            None,
        )
        .expect("generic provider");
        assert_eq!(
            provider.options.protocol,
            ProviderProtocol::OpenAiCompatible
        );
        assert_eq!(provider.model, "chosen-model");
        assert!(
            translation_provider(
                "test-only-key",
                "https://proxy.invalid/completions",
                "m",
                Some("typo"),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn translation_completion_exposes_only_the_requested_answer() {
        struct Script;
        impl stkcraft_agent_runtime::ChatModel for Script {
            fn complete(
                &self,
                messages: &[stkcraft_agent_runtime::Message],
                tools: &[serde_json::Value],
            ) -> Result<stkcraft_agent_runtime::Turn, String> {
                assert_eq!(tools, &[] as &[serde_json::Value]);
                assert!(
                    matches!(messages.last(), Some(stkcraft_agent_runtime::Message::User(text)) if text == "translate this")
                );
                Ok(stkcraft_agent_runtime::Turn {
                    text: "{\"Save\":\"Speichern\"}".to_owned(),
                    reasoning_content: Some("private-protocol-marker".to_owned()),
                    ..stkcraft_agent_runtime::Turn::default()
                })
            }
        }
        assert_eq!(
            completion_with(&Script, "translate this").expect("answer"),
            "{\"Save\":\"Speichern\"}"
        );
    }

    #[test]
    fn a_json_object_is_found_in_a_fenced_answer() {
        let answer = "```json\n{\"Save\": \"Speichern\"}\n```";
        let value = json_object_in(answer).expect("an object");
        assert_eq!(value["Save"], "Speichern");
        assert!(json_object_in("no json here").is_none());
    }
}
