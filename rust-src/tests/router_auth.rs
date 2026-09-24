//! Remote-auth regression tests for `lac-router` v2.8.
//!
//! Covers the fail-closed contract from docs/ARCHITECTURE.md:
//!   1. non-loopback bind (`LAC_BIND_ADDR=0.0.0.0`) without `LAC_API_TOKEN`
//!      refuses to start (exit 1, "Refusing" on stderr);
//!   2. loopback bind without a token stays open (`GET /lac/status` -> 200).
//! One sequential #[test]: child environments are explicit and isolated.

mod support;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

fn wait_status(port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(mut s) = TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", port).parse().unwrap(),
            Duration::from_millis(300),
        ) {
            s.set_read_timeout(Some(Duration::from_millis(800))).ok();
            let _ = s.write_all(b"GET /lac/status HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            if String::from_utf8_lossy(&buf).contains("200 OK") {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

fn request_status(port: u16, authorization: Option<&str>, forwarded: bool) -> Option<u16> {
    let addr = format!("127.0.0.1:{}", port).parse().ok()?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(800))).ok()?;
    let auth = authorization
        .map(|token| format!("Authorization: Bearer {}\r\n", token))
        .unwrap_or_default();
    let forwarded = if forwarded { "X-Forwarded-For: 100.64.0.5\r\n" } else { "" };
    let request = format!(
        "GET /lac/status HTTP/1.1\r\nHost: x\r\n{}{}Connection: close\r\n\r\n",
        forwarded, auth
    );
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).ok()?;
    String::from_utf8_lossy(&response)
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[test]
fn remote_auth_fail_closed_and_loopback_open() {
    let _port_lock = support::PortLock::acquire();
    let tmp_home = std::env::temp_dir().join(format!("lac-test-auth-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp_home);
    let router_bin = env!("CARGO_BIN_EXE_lac-router");
    let dead_listener = TcpListener::bind(("127.0.0.1", 0)).expect("dead backend reservation");
    let dead_port = dead_listener.local_addr().expect("dead backend address").port();
    let dead_env = dead_port.to_string();
    let home_env = tmp_home.to_string_lossy().into_owned();

    // 1. Fail closed: 0.0.0.0 without a token must exit 1 with "Refusing".
    let mut refusal = std::process::Command::new(router_bin);
    refusal
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &home_env)
        .env("LAC_BIND_ADDR", "0.0.0.0")
        .env("LAC_ROUTER_PORT", "0")
        .env("MLX_PORT", &dead_env)
        .env("LAC_LLAMA_PORT", &dead_env)
        .env("LAC_OLLAMA_PORT", &dead_env);
    let out = refusal.output().expect("router binary runs");
    assert_eq!(out.status.code(), Some(1), "non-loopback bind without token must exit 1");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Refusing"),
        "stderr must explain refusal: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 2. Loopback without a token stays open for local dev.
    let open_router = support::spawn_router(
        router_bin,
        &[
            ("HOME", &home_env),
            ("LAC_BIND_ADDR", "127.0.0.1"),
            ("MLX_PORT", &dead_env),
            ("LAC_LLAMA_PORT", &dead_env),
            ("LAC_OLLAMA_PORT", &dead_env),
        ],
    );
    let ok = wait_status(open_router.port, Duration::from_secs(15));
    assert!(ok, "loopback without token must serve /lac/status 200");

    let token_router = support::spawn_router(
        router_bin,
        &[
            ("HOME", &home_env),
            ("LAC_BIND_ADDR", "127.0.0.1"),
            ("LAC_API_TOKEN", "test-secret"),
            ("MLX_PORT", &dead_env),
            ("LAC_LLAMA_PORT", &dead_env),
            ("LAC_OLLAMA_PORT", &dead_env),
        ],
    );
    let token_port = token_router.port;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10)
        && request_status(token_port, Some("test-secret"), false) != Some(200)
    {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(request_status(token_port, None, false), Some(401));
    assert_eq!(request_status(token_port, Some("wrong"), false), Some(401));
    assert_eq!(request_status(token_port, Some("test-secret"), false), Some(200));
    assert_eq!(request_status(token_port, Some("test-secret"), true), Some(200));
    assert_eq!(request_status(token_port, None, true), Some(401));
    let remote_router = support::spawn_router(
        router_bin,
        &[
            ("HOME", &home_env),
            ("LAC_BIND_ADDR", "0.0.0.0"),
            ("LAC_API_TOKEN", "test-secret"),
            ("LAC_ALLOW_INSECURE_BIND", "1"),
            ("MLX_PORT", &dead_env),
            ("LAC_LLAMA_PORT", &dead_env),
            ("LAC_OLLAMA_PORT", &dead_env),
        ],
    );
    let remote_port = remote_router.port;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10)
        && request_status(remote_port, Some("test-secret"), false) != Some(200)
    {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(request_status(remote_port, None, false), Some(401));
    assert_eq!(request_status(remote_port, Some("test-secret"), false), Some(200));
    drop(remote_router);
    drop(token_router);
    drop(open_router);
    let _ = std::fs::remove_dir_all(&tmp_home);
}
