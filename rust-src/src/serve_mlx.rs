//! serve-mlx v2.6 — mlx_lm.server on :8080 (Q4 speed lane, native MTP).
//!
//! v2.6: binary preflight with install hint, readiness prober thread,
//! and a daemon retry loop that actually survives the night: unbounded
//! attempts with capped exponential backoff + jitter instead of giving
//! up after 3 tries (a "KeepAlive" that quits is not one).

mod common;

use std::env;
use std::net::{SocketAddr, TcpStream};
use std::process::Command;
use std::time::Duration;

// Ensure mlx and llama servers coexist without port conflict.
// mlx owns :8080, llama owns :8081. If the preferred port is free,
// we use it; otherwise we probe the next available port so both can
// run simultaneously and the /models picker routes correctly.
fn find_free_mlx_port() -> u16 {
    if let Ok(p) = env::var("MLX_PORT").or_else(|_| env::var("PORT")) {
        if let Ok(port) = p.parse::<u16>() {
            return port;
        }
    }
    // Preferred port 8080, then probe 8082, 8083 … (skip 8081 reserved for llama-server).
    for port in [8080, 8082, 8083, 8084, 8085, 8086, 8087, 8088, 8089, 8090].iter().copied() {
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            continue;
        }
        return port;
    }
    8080 // last-resort fallback
}

// ── Auto-restart daemon mode ─────────────────────────────────────────────
// LAUNCHDAEMON=1 (set by the launchd plist): retry forever with backoff
// capped at 60s plus jitter, so a transient OOM at 3am costs a minute,
// not the whole night. Without the flag: single-run, Ctrl+C stops.
const BACKOFF_BASE_MS: u64 = 2000;
const BACKOFF_CAP_MS: u64 = 60_000;

fn should_daemon() -> bool {
    env::var("LAUNCHDAEMON").map_or(false, |v| v == "1")
}

/// Deterministic jitter from pid + attempt (std-only, no rand crate).
fn jitter_ms(attempt: u32) -> u64 {
    let pid = std::process::id() as u64;
    (pid.wrapping_mul(2654435761).wrapping_add(attempt as u64 * 40503)) % 1000
}

fn backoff_ms(attempt: u32) -> u64 {
    let exp = BACKOFF_BASE_MS.saturating_mul(1u64 << attempt.min(5));
    exp.min(BACKOFF_CAP_MS) + jitter_ms(attempt)
}

fn spawn_server(model: &str, port: u16) -> std::io::Result<std::process::ExitStatus> {
    Command::new("mlx_lm.server")
        .arg("--model")
        .arg(model)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .status()
}

fn daemon_retry_loop(model: String, port: u16) {
    let mut attempt: u32 = 0;
    loop {
        eprintln!(
            "mlx-lm launch (attempt {}, model {} on port {})",
            attempt + 1,
            model,
            port
        );
        match spawn_server(&model, port) {
            Ok(s) if s.success() => {
                eprintln!("mlx-lm exited cleanly; daemon stopping.");
                return;
            }
            Ok(s) => {
                eprintln!("mlx-lm crashed ({}); backing off...", s);
            }
            Err(e) => {
                eprintln!("Failed to launch mlx_lm.server: {} (retrying)", e);
            }
        }
        let wait = backoff_ms(attempt);
        attempt = attempt.saturating_add(1);
        std::thread::sleep(Duration::from_millis(wait));
    }
}

fn which(cmd: &str) -> bool {
    common::which(cmd).is_some()
}

fn main() {
    eprintln!("=== LAC serve-mlx v2.7 (Rust, port-aware, auto-restart daemon) ===");

    if !which("mlx_lm.server") {
        eprintln!("mlx_lm.server not found in PATH.");
        eprintln!("Install: brew install mlx-lm  (or: uv tool install mlx-lm)");
        std::process::exit(1);
    }

    let model = env::args().nth(1).unwrap_or_else(|| {
        env::var("MLX_MODEL").unwrap_or_else(|_| "mlx-community/Qwen3.8-27B-4bit".to_string())
    });

    let port = find_free_mlx_port();
    eprintln!("mlx-lm server with model: {}", model);
    eprintln!("Listening on 127.0.0.1:{}", port);
    // Advertise the bound port so the router (and `lac status`) follows
    // drift to 8082+ instead of probing a dead :8080. The router still
    // readiness-gates, so a stale file can never route into the void.
    let _ = common::atomic_write(
        &format!("{}/.lac/mlx.port", common::home_dir()),
        &port.to_string(),
    );

    std::thread::spawn(move || {
        if common::wait_for_http(port, Duration::from_secs(300)) {
            eprintln!("serve-mlx READY on :{} (model loaded)", port);
        } else {
            eprintln!("serve-mlx: model not ready after 300s — check logs");
        }
    });

    if should_daemon() {
        daemon_retry_loop(model, port);
    } else {
        match spawn_server(&model, port) {
            Ok(s) => eprintln!("server exited: {}", s),
            Err(e) => eprintln!("Failed to start mlx_lm.server: {}", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_caps_and_grows() {
        assert!(backoff_ms(0) >= BACKOFF_BASE_MS);
        assert!(backoff_ms(0) < backoff_ms(3));
        assert!(backoff_ms(30) <= BACKOFF_CAP_MS + 1000);
        assert!(backoff_ms(100) <= BACKOFF_CAP_MS + 1000);
    }

    #[test]
    fn jitter_bounded() {
        for a in 0..10 {
            assert!(jitter_ms(a) < 1000);
        }
    }
}
