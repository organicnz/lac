//! Remote-auth regression tests for `lac-router` v2.8.
//!
//! Covers the fail-closed contract from docs/ARCHITECTURE.md:
//!   1. non-loopback bind (`LAC_BIND_ADDR=0.0.0.0`) without `LAC_API_TOKEN`
//!      refuses to start (exit 1, "Refusing" on stderr);
//!   2. loopback bind without a token stays open (`GET /lac/status` -> 200).
//! One sequential #[test]: process env is global, scenarios run in order.

use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

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
            use std::io::Write as _;
            let _ = s.read_to_end(&mut buf);
            if String::from_utf8_lossy(&buf).contains("200 OK") {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

#[test]
fn remote_auth_fail_closed_and_loopback_open() {
    use std::io::Write as _;
    let tmp_home = std::env::temp_dir().join(format!("lac-test-auth-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp_home);
    let orig_home = std::env::var("HOME").ok();
    let orig_bind = std::env::var("LAC_BIND_ADDR").ok();
    let orig_token = std::env::var("LAC_API_TOKEN").ok();
    let orig_port = std::env::var("LAC_ROUTER_PORT").ok();
    // Keep backends down: auth is decided before routing, no backend needed.
    let orig_mlx = std::env::var("MLX_PORT").ok();
    let orig_llama = std::env::var("LAC_LLAMA_PORT").ok();
    let orig_ollama = std::env::var("LAC_OLLAMA_PORT").ok();
    let dead_port = free_port().to_string();

    let router_bin = env!("CARGO_BIN_EXE_lac-router");

    unsafe {
        std::env::set_var("HOME", &tmp_home);
        std::env::set_var("MLX_PORT", &dead_port);
        std::env::set_var("LAC_LLAMA_PORT", &dead_port);
        std::env::set_var("LAC_OLLAMA_PORT", &dead_port);
    }

    // 1. Fail closed: 0.0.0.0 without a token must exit 1 with "Refusing".
    let refuse_port = free_port();
    unsafe {
        std::env::set_var("LAC_BIND_ADDR", "0.0.0.0");
        std::env::remove_var("LAC_API_TOKEN");
        std::env::set_var("LAC_ROUTER_PORT", refuse_port.to_string());
    }
    let out = std::process::Command::new(router_bin)
        .output()
        .expect("router binary runs");
    assert_eq!(out.status.code(), Some(1), "non-loopback bind without token must exit 1");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Refusing"),
        "stderr must explain refusal: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 2. Loopback without a token stays open for local dev.
    let open_port = free_port();
    unsafe {
        std::env::set_var("LAC_BIND_ADDR", "127.0.0.1");
        std::env::remove_var("LAC_API_TOKEN");
        std::env::set_var("LAC_ROUTER_PORT", open_port.to_string());
    }
    let mut child = std::process::Command::new(router_bin)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("router binary runs");
    let ok = wait_status(open_port, Duration::from_secs(15));
    let _ = child.kill();
    let _ = child.wait();
    assert!(ok, "loopback without token must serve /lac/status 200");

    unsafe {
        match orig_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        match orig_bind {
            Some(v) => std::env::set_var("LAC_BIND_ADDR", v),
            None => std::env::remove_var("LAC_BIND_ADDR"),
        }
        match orig_token {
            Some(v) => std::env::set_var("LAC_API_TOKEN", v),
            None => std::env::remove_var("LAC_API_TOKEN"),
        }
        match orig_port {
            Some(v) => std::env::set_var("LAC_ROUTER_PORT", v),
            None => std::env::remove_var("LAC_ROUTER_PORT"),
        }
        match orig_mlx {
            Some(v) => std::env::set_var("MLX_PORT", v),
            None => std::env::remove_var("MLX_PORT"),
        }
        match orig_llama {
            Some(v) => std::env::set_var("LAC_LLAMA_PORT", v),
            None => std::env::remove_var("LAC_LLAMA_PORT"),
        }
        match orig_ollama {
            Some(v) => std::env::set_var("LAC_OLLAMA_PORT", v),
            None => std::env::remove_var("LAC_OLLAMA_PORT"),
        }
    }
    let _ = std::fs::remove_dir_all(&tmp_home);
}
