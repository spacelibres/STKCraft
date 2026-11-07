//! Offline controlled model regression suite.
#![allow(clippy::expect_used, clippy::default_trait_access)]
#[cfg(not(target_arch = "wasm32"))]
use stkcraft_agent_runtime::{ModelError, RequestControl};
#[cfg(not(target_arch = "wasm32"))]
#[path = "../src/transport.rs"]
mod transport;
#[cfg(target_arch = "wasm32")]
fn main() {}
#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Some(mode) = std::env::args().nth(2) {
        fixture(&mode);
        return;
    }
    checks();
    let config = transport::encode_config(
        "https://example.invalid",
        "poison",
        &serde_json::json!({"x":"@星\n\\\""}),
        std::time::Duration::from_secs(1),
    )
    .expect("config");
    assert!(
        String::from_utf8(config)
            .expect("utf8")
            .contains("data-raw")
    );
    assert_eq!(
        transport::parse_output(b"hello\nSTKCRAFT_HTTP_STATUS:200\n".to_vec()),
        Ok("hello".into())
    );
    assert_eq!(
        transport::parse_output(b"poison\nSTKCRAFT_HTTP_STATUS:401\n".to_vec()),
        Err(ModelError::HttpStatus(401))
    );
    let exe = std::env::current_exe().expect("exe");
    for mode in ["ok", "stderr", "flood", "hang", "unread", "exit"] {
        let mut command = std::process::Command::new(&exe);
        command.args(["--fixture", mode]);
        let control =
            RequestControl::with_timeout(Default::default(), std::time::Duration::from_millis(500))
                .expect("control");
        let result = transport::run(command, vec![b'x'; 1024 * 1024], &control);
        match mode {
            "ok" | "stderr" => assert_eq!(result, Ok("{}".into())),
            "flood" => assert_eq!(result, Err(ModelError::ResponseTooLarge)),
            "hang" | "unread" => assert_eq!(result, Err(ModelError::Timeout)),
            _ => assert!(result.is_err()),
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::zombie_processes)] // Intentionally orphaned finite fixture demonstrates portable cleanup limits.
fn fixture(mode: &str) {
    use std::io::{Read, Write};
    match mode {
        "hold" => std::thread::sleep(std::time::Duration::from_millis(1800)),
        "unread" => std::thread::sleep(std::time::Duration::from_secs(30)),
        "exit" => std::process::exit(7),
        _ => {
            let mut chunk = [0; 8192];
            while std::io::stdin().read(&mut chunk).expect("stdin") != 0 {}
            match mode {
                "hang" => std::thread::sleep(std::time::Duration::from_secs(30)),
                "flood" => {
                    let block = [b'x'; 8192];
                    for _ in 0..2048 {
                        if std::io::stdout().write_all(&block).is_err() {
                            return;
                        }
                    }
                }
                _ => {
                    if mode == "descendant" {
                        let _child =
                            std::process::Command::new(std::env::current_exe().expect("exe"))
                                .args(["--fixture", "hold"])
                                .spawn()
                                .expect("descendant");
                    }
                    if mode == "stderr" {
                        for _ in 0..256 {
                            std::io::stderr().write_all(&[b'p'; 8192]).expect("stderr");
                        }
                    }
                    print!("{{}}\nSTKCRAFT_HTTP_STATUS:200\n");
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn checks() {
    use std::time::Duration;
    use transport::{Hooks, LIMIT};
    let encode = |u: &str, k: &str, b: &serde_json::Value| {
        transport::encode_config(u, k, b, Duration::from_secs(1))
    };
    for (u, k) in [
        ("https://x\nheader=x", "key"),
        ("https://x", "key\r\npoison"),
        ("file:///x", "key"),
    ] {
        assert_eq!(
            encode(u, k, &serde_json::json!({})),
            Err(ModelError::Configuration)
        );
    }
    assert_eq!(
        encode("https://x", "key", &serde_json::json!("x".repeat(LIMIT))),
        Err(ModelError::RequestTooLarge)
    );
    for body in [
        b"".as_slice(),
        b"\nSTKCRAFT_HTTP_STATUS:000\n",
        b"\nSTKCRAFT_HTTP_STATUS:20x\n",
        b"\nSTKCRAFT_HTTP_STATUS:200\ntrailing",
        b"\xff\nSTKCRAFT_HTTP_STATUS:200\n",
    ] {
        assert_eq!(
            transport::parse_output(body.to_vec()),
            Err(ModelError::Protocol)
        );
    }
    let mut exact = vec![b'x'; LIMIT];
    exact.extend_from_slice(b"\nSTKCRAFT_HTTP_STATUS:200\n");
    assert_eq!(transport::parse_output(exact).expect("exact").len(), LIMIT);
    let mut extra = vec![b'x'; LIMIT + 1];
    extra.extend_from_slice(b"\nSTKCRAFT_HTTP_STATUS:200\n");
    assert_eq!(
        transport::parse_output(extra),
        Err(ModelError::ResponseTooLarge)
    );
    for status in [401, 429, 500] {
        assert_eq!(
            transport::parse_output(
                format!("poison\nSTKCRAFT_HTTP_STATUS:{status}\n").into_bytes()
            ),
            Err(ModelError::HttpStatus(status))
        );
    }
    let exe = std::env::current_exe().expect("exe");
    let control =
        RequestControl::with_timeout(Default::default(), Duration::from_secs(5)).expect("control");
    for hooks in [
        Hooks {
            fail_start: Some(0),
            ..Hooks::default()
        },
        Hooks {
            fail_start: Some(1),
            ..Hooks::default()
        },
        Hooks {
            fail_start: Some(2),
            ..Hooks::default()
        },
        Hooks {
            missing_pipe: true,
            ..Hooks::default()
        },
        Hooks {
            fail_read: true,
            ..Hooks::default()
        },
        Hooks {
            fail_write: true,
            ..Hooks::default()
        },
        Hooks {
            fail_wait: true,
            ..Hooks::default()
        },
    ] {
        let mut command = std::process::Command::new(&exe);
        command.args(["--fixture", "unread"]);
        let (result, report) =
            transport::run_test(command, vec![b'x'; 1024 * 1024], &control, &hooks);
        assert!(result.is_err());
        assert_ne!(result, Err(ModelError::Cleanup), "{report:?}");
        assert!(report.reaped);
        assert_eq!(report.started, report.joined);
    }
    let token = stkcraft_agent_runtime::CancellationToken::default();
    token.cancel();
    let (result, report) = transport::run_test(
        std::process::Command::new("executable-that-does-not-exist"),
        vec![],
        &RequestControl::new(token),
        &Hooks::default(),
    );
    assert_eq!(result, Err(ModelError::Cancelled));
    assert_eq!(report.started, 0);
    assert!(!report.reaped);
    for mode in ["unread", "hang"] {
        let token = stkcraft_agent_runtime::CancellationToken::default();
        let cancel = token.clone();
        let control = RequestControl::with_timeout(token, Duration::from_secs(5)).expect("timeout");
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            cancel.cancel();
        });
        let mut command = std::process::Command::new(&exe);
        command.args(["--fixture", mode]);
        let (result, report) = transport::run_test(
            command,
            vec![b'x'; 1024 * 1024],
            &control,
            &Hooks::default(),
        );
        worker.join().expect("cancel thread");
        assert_eq!(result, Err(ModelError::Cancelled));
        assert!(report.reaped);
        assert_eq!(report.started, report.joined);
    }
    let control = RequestControl::with_timeout(Default::default(), Duration::from_millis(300))
        .expect("deadline");
    let mut command = std::process::Command::new(&exe);
    command.args(["--fixture", "descendant"]);
    let (result, report) = transport::run_test(command, vec![], &control, &Hooks::default());
    assert_eq!(result, Err(ModelError::Cleanup));
    assert!(report.reaped);
    assert!(report.joined < report.started);
    // Finite fixture descendant exits autonomously; let its capped detached readers finish.
    std::thread::sleep(Duration::from_millis(800));
}
