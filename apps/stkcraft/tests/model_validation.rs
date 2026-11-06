//! Command-level configuration validation without touching the user's configuration.
//!
//! macOS uses HOME directly for its configuration root; exercise the same pure resolver there
//! through prefs unit tests rather than replacing HOME in this subprocess test.
#![cfg(any(target_os = "windows", target_os = "linux", target_os = "freebsd"))]
#![allow(clippy::expect_used)]

use std::process::Command;

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
