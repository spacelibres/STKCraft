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

/// One chat completion through `DeepSeek`, spoken over `curl`.
///
/// The same transport the assistant uses, for the same reasons: no TLS stack in the build, no HTTP
/// client to audit, and a request and response a person can read when a translation looks wrong.
pub fn chat_completion(key: &str, prompt: &str) -> Result<String, String> {
    let base_url = std::env::var("DEEPSEEK_BASE_URL")
        .unwrap_or_else(|_| "https://api.deepseek.com/v1/chat/completions".to_owned());
    let model = std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-chat".to_owned());
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "You translate software interfaces. You answer with JSON \
                                           only, exactly as asked."},
            {"role": "user", "content": prompt},
        ],
        "temperature": 0.0,
    })
    .to_string();
    let mut child = std::process::Command::new("curl")
        .args([
            "-sS",
            // The body is the only place a provider says what it disliked; `--fail` would throw it
            // away and leave a bare status code.
            "--fail-with-body",
            "--max-time",
            "300",
            "-X",
            "POST",
            &base_url,
            "-H",
            "Content-Type: application/json",
            "-H",
        ])
        .arg(format!("Authorization: Bearer {key}"))
        .args(["--data-binary", "@-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not run curl: {error}"))?;
    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write as _;
        stdin
            .write_all(body.as_bytes())
            .map_err(|error| format!("could not send the request: {error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("the translation request did not finish: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let body = String::from_utf8_lossy(&output.stdout);
        let body = body.trim();
        return Err(if body.is_empty() {
            format!("the model refused the request: {detail}")
        } else {
            format!("the model refused the request: {detail}: {body}")
        });
    }
    let answer: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("the model's answer is not JSON: {error}"))?;
    answer
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "the model's answer has no message".to_owned())
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_json_object_is_found_in_a_fenced_answer() {
        let answer = "```json\n{\"Save\": \"Speichern\"}\n```";
        let value = json_object_in(answer).expect("an object");
        assert_eq!(value["Save"], "Speichern");
        assert!(json_object_in("no json here").is_none());
    }
}
