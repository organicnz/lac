//! serve-llama v2.6 — llama-server on :8081 (Q8 quality lane).
//!
//! v2.6: thermal probed once (old code spawned pmset 3x and double-
//! shrank Fair 8192->4096 by accident), port overridable via
//! LAC_LLAMA_PORT, binary/model preflight with actionable errors,
//! context clamped to a sane range, and a readiness prober thread so
//! `lac status` stops racing server boot.

mod common;

use std::env;
use std::net::{SocketAddr, TcpStream};
use std::process::Command;
use std::time::Duration;

fn port_in_use(port: u16) -> bool {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_ok()
}

fn server_port() -> u16 {
    env::var("LAC_LLAMA_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8081)
}

/// Context default for one thermal reading. Fair/Critical shrink the
/// window so the server stays under the throttle threshold.
fn thermal_default_context(thermal: &str) -> i32 {
    match thermal {
        "Critical" => 4096,
        "Serious" | "Fair" => 8192,
        _ => 16384,
    }
}

fn which(cmd: &str) -> bool {
    common::which(cmd).is_some()
}

fn looks_like_path(model: &str) -> bool {
    model.starts_with('/') || model.starts_with('.') || model.ends_with(".gguf")
}

fn main() {
    eprintln!("=== LAC serve-llama v2.6 (Rust, port-coexistent, thermal-aware) ===");

    if !which("llama-server") {
        eprintln!("llama-server not found in PATH.");
        eprintln!("Install: brew install llama.cpp");
        std::process::exit(1);
    }

    let model = env::args().nth(1).unwrap_or_else(|| {
        env::var("LLAMA_MODEL").unwrap_or_else(|_| "unsloth/Qwen3.6-27B-MTP-GGUF:Q8_0".to_string())
    });
    if looks_like_path(&model) && std::fs::metadata(&model).is_err() {
        eprintln!("Model file not found: {}", model);
        eprintln!("Hint: pass an HF ref (e.g. unsloth/Qwen3.6-27B-MTP-GGUF:Q8_0) or a valid -m path.");
        std::process::exit(1);
    }

    let thermal = common::thermal_state();
    let context: i32 = env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| thermal_default_context(&thermal));
    let context = context.clamp(2048, 65536);

    let port = server_port();
    eprintln!("llama-server with model: {}", model);
    eprintln!("Context window: {} tokens (thermal={})", context, thermal);
    eprintln!("Listening on 127.0.0.1:{}", port);

    if port_in_use(port) {
        eprintln!("port {} already bound — another llama-server is up; exiting; route to it via /v1/models.", port);
        std::process::exit(0);
    }

    // Readiness prober: the router's health gate needs /v1/models == 200,
    // which lags bind by the model-load time. Report it instead of racing.
    std::thread::spawn(move || {
        if common::wait_for_http(port, Duration::from_secs(300)) {
            eprintln!("serve-llama READY on :{} (model loaded)", port);
        } else {
            eprintln!("serve-llama: model not ready after 300s — check logs");
        }
    });

    let status = Command::new("llama-server")
        .arg("-m")
        .arg(&model)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("-c")
        .arg(context.to_string())
        .arg("-fa")
        .arg("on")
        .arg("-b")
        .arg("2048")
        .arg("-ub")
        .arg("256")
        .arg("-np")
        .arg("4")
        .arg("-n-gpu-layers")
        .arg("999")
        .status();

    match status {
        Ok(s) => eprintln!("server exited: {}", s),
        Err(e) => eprintln!("Failed to start llama-server: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thermal_context_steps_down() {
        assert_eq!(thermal_default_context("Nominal"), 16384);
        assert_eq!(thermal_default_context("Fair"), 8192);
        assert_eq!(thermal_default_context("Serious"), 8192);
        assert_eq!(thermal_default_context("Critical"), 4096);
    }

    #[test]
    fn path_detection() {
        assert!(looks_like_path("/models/foo.gguf"));
        assert!(looks_like_path("./x.gguf"));
        assert!(looks_like_path("qwen.gguf"));
        assert!(!looks_like_path("unsloth/Qwen3.6-27B-MTP-GGUF:Q8_0"));
        assert!(!looks_like_path("qwen3.8-27b"));
    }
}
