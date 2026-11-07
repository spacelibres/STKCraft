//! Command-level configuration validation without touching the user's configuration.
//!
//! macOS uses HOME directly for its configuration root; exercise the same pure resolver there
//! through prefs unit tests rather than replacing HOME in this subprocess test.
#![cfg(any(target_os = "windows", target_os = "linux", target_os = "freebsd"))]
#![allow(clippy::expect_used)]

use std::process::Command;

#[test]
fn output_cap_is_configurable_and_invalid_values_preserve_settings() {
    let base = std::env::temp_dir()
        .canonicalize()
        .expect("temporary directory");
    let root = base.join(format!("stkcraft-output-cap-{}", std::process::id()));
    std::fs::create_dir(&root).expect("unique configuration root");
    let run = |arguments: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_stkcraft"))
            .args(arguments)
            .env("APPDATA", &root)
            .env("XDG_CONFIG_HOME", &root)
            .env_remove("DEEPSEEK_API_KEY")
            .env_remove("DEEPSEEK_BASE_URL")
            .env_remove("DEEPSEEK_MODEL")
            .env("DEEPSEEK_PROTOCOL", "deepseek")
            .env("DEEPSEEK_THINKING", "disabled")
            .env("DEEPSEEK_MAX_OUTPUT_TOKENS", "0")
            .output()
            .expect("configuration command")
    };
    let valid = run(&["model", "--max-output-tokens", "4096"]);
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    let path = root.join("stkcraft/ui.json");
    let saved = std::fs::read_to_string(&path).expect("saved settings");
    let json: serde_json::Value = serde_json::from_str(&saved).expect("settings JSON");
    assert_eq!(
        json.get("model_max_output_tokens")
            .and_then(serde_json::Value::as_str),
        Some("4096")
    );
    for invalid in ["0", "-1", "4294967296", "many"] {
        let result = run(&[
            "model",
            "--max-output-tokens",
            invalid,
            "--key",
            "test-only",
        ]);
        assert!(!result.status.success(), "invalid output cap accepted");
        assert_eq!(
            std::fs::read_to_string(&path).expect("preserved settings"),
            saved
        );
        assert!(!root.join("stkcraft/model.key").exists());
    }
    let resolved = root.canonicalize().expect("configuration root");
    assert!(resolved.starts_with(&base));
    std::fs::remove_dir_all(resolved).expect("remove isolated fixture");
}

#[test]
fn malformed_saved_model_options_preserve_the_connection_and_credential() {
    let base = std::env::temp_dir()
        .canonicalize()
        .expect("temporary directory");
    let invalid = [
        ("model_protocol", serde_json::json!(1)),
        ("model_protocol", serde_json::json!({})),
        ("model_protocol", serde_json::json!([])),
        ("model_protocol", serde_json::json!(true)),
        ("model_thinking", serde_json::json!(1)),
        ("model_thinking", serde_json::json!({})),
        ("model_thinking", serde_json::json!([])),
        ("model_thinking", serde_json::json!(true)),
        ("model_max_output_tokens", serde_json::json!(0)),
        ("model_max_output_tokens", serde_json::json!("0")),
        ("model_max_output_tokens", serde_json::json!({})),
        (
            "oversized_future_field",
            serde_json::json!("x".repeat(65_537)),
        ),
    ];
    for (index, (field, value)) in invalid.into_iter().enumerate() {
        let root = base.join(format!(
            "stkcraft-model-saved-validation-{}-{index}",
            std::process::id()
        ));
        assert!(root.starts_with(&base));
        std::fs::create_dir(&root).expect("unique configuration root");
        let directory = root.join("stkcraft");
        std::fs::create_dir(&directory).expect("settings directory");
        let key = directory.join("model.key");
        let preferences = directory.join("ui.json");
        let mut saved = serde_json::json!({
            "language": "en",
            "model_endpoint": "https://private.invalid/v1/chat/completions",
            "model": "private-model"
        });
        saved
            .as_object_mut()
            .expect("fixture object")
            .insert(field.to_owned(), value);
        let original = saved.to_string();
        std::fs::write(&key, "private-test-only").expect("fixture credential");
        std::fs::write(&preferences, &original).expect("fixture preferences");
        for arguments in [
            vec!["model"],
            vec!["model", "--key", "replacement-test-only"],
            vec!["model", "--clear"],
        ] {
            let result = Command::new(env!("CARGO_BIN_EXE_stkcraft"))
                .args(arguments)
                .env("APPDATA", &root)
                .env("XDG_CONFIG_HOME", &root)
                .env_remove("DEEPSEEK_API_KEY")
                .env_remove("DEEPSEEK_BASE_URL")
                .env_remove("DEEPSEEK_MODEL")
                .env("DEEPSEEK_PROTOCOL", "deepseek")
                .env("DEEPSEEK_THINKING", "disabled")
                .output()
                .expect("configuration command");
            assert!(
                !result.status.success(),
                "malformed saved {field} must not resolve defaults"
            );
            assert_eq!(
                std::fs::read_to_string(&preferences).expect("preserved preferences"),
                original
            );
            assert_eq!(
                std::fs::read_to_string(&key).expect("preserved credential"),
                "private-test-only"
            );
        }
        let resolved = root.canonicalize().expect("configuration root");
        assert!(resolved.starts_with(&base));
        std::fs::remove_dir_all(resolved).expect("remove isolated fixtures");
    }
}

#[test]
fn invalid_protocol_options_cannot_partially_write_credentials() {
    let base = std::env::temp_dir()
        .canonicalize()
        .expect("temporary directory");
    for (index, arguments, protocol, thinking) in [
        (
            0,
            vec!["model", "--key", "replacement-test-only"],
            "typo",
            "disabled",
        ),
        (1, vec!["model", "--clear"], "deepseek", "turbo"),
    ] {
        let root = base.join(format!(
            "stkcraft-model-validation-{}-{index}",
            std::process::id()
        ));
        assert!(root.starts_with(&base));
        std::fs::create_dir(&root).expect("unique configuration root");
        let directory = root.join("stkcraft");
        std::fs::create_dir(&directory).expect("settings directory");
        let key = directory.join("model.key");
        let preferences = directory.join("ui.json");
        std::fs::write(&key, "existing-test-only").expect("fixture credential");
        std::fs::write(&preferences, "{\"language\":\"en\"}").expect("fixture preferences");
        let run = |arguments: &[&str]| {
            Command::new(env!("CARGO_BIN_EXE_stkcraft"))
                .args(arguments)
                .env("APPDATA", &root)
                .env("XDG_CONFIG_HOME", &root)
                .env_remove("DEEPSEEK_API_KEY")
                .env_remove("DEEPSEEK_BASE_URL")
                .env_remove("DEEPSEEK_MODEL")
                .env("DEEPSEEK_PROTOCOL", protocol)
                .env("DEEPSEEK_THINKING", thinking)
                .output()
                .expect("configuration command")
        };
        let result = run(&arguments);
        assert_eq!(
            std::fs::read_to_string(&key).ok().as_deref(),
            Some("existing-test-only")
        );
        assert_eq!(
            std::fs::read_to_string(&preferences).expect("unchanged preferences"),
            "{\"language\":\"en\"}"
        );
        assert!(!result.status.success(), "invalid options must fail");
        let valid = run(&["model", "--protocol", "deepseek", "--thinking", "disabled"]);
        assert!(
            valid.status.success(),
            "explicit saved options repair the configuration"
        );
        assert!(!String::from_utf8_lossy(&valid.stdout).contains("existing-test-only"));
        let saved: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&preferences).expect("saved preferences"),
        )
        .expect("preferences JSON");
        assert_eq!(
            saved
                .get("model_protocol")
                .and_then(serde_json::Value::as_str),
            Some("deepseek")
        );
        assert_eq!(
            saved
                .get("model_thinking")
                .and_then(serde_json::Value::as_str),
            Some("disabled")
        );
        assert!(
            run(&arguments).status.success(),
            "valid credential changes still work"
        );
        if index == 0 {
            assert_eq!(
                std::fs::read_to_string(&key).expect("updated fixture"),
                "replacement-test-only"
            );
        } else {
            assert!(!key.exists(), "valid clear removes the fixture");
        }
        let resolved = root.canonicalize().expect("configuration root");
        assert!(resolved.starts_with(&base));
        std::fs::remove_dir_all(&resolved).expect("remove isolated fixtures");
    }
}
