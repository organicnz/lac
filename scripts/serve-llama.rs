//! scripts/serve-llama.rs — verified llama-server launcher (standalone, std-only).
//!
//! Build:  rustc -O scripts/serve-llama.rs -o scripts/bin/serve-llama
//!         (or: make scripts; run from the repo root — this file includes
//!         ../rust-src/src/common.rs for the audited PATH lookup)
//! Usage:  ./scripts/bin/serve-llama [MODEL] [CONTEXT] [--port PORT]
//!
//! A dependency-free reference fallback. Day to day, prefer
//! `lac serve llama` (thermal-aware context, readiness reporting).
//! Unlike the old bash script, this launcher preflights the binary,
//! the port, and local model files, and clamps the context window.

#[path = "../rust-src/src/common.rs"]
mod common;

use std::env;
use std::net::{SocketAddr, TcpStream};
use std::process::Command;
use std::time::Duration;

const DEFAULT_MODEL: &str = "unsloth/Qwen3.6-27B-MTP-GGUF:Q8_0";

fn which(cmd: &str) -> bool {
    common::which(cmd).is_some()
}

fn port_in_use(port: u16) -> bool {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

fn usage() -> ! {
    eprintln!("usage: serve-llama [MODEL] [CONTEXT] [--port PORT]");
    eprintln!("  MODEL   defaults to {} (or $LLAMA_MODEL)", DEFAULT_MODEL);
    eprintln!("  CONTEXT defaults to 16384, clamped to 2048..=65536");
    eprintln!("  PORT    defaults to 8081 (or $LAC_LLAMA_PORT)");
    std::process::exit(2);
}

fn looks_like_path(model: &str) -> bool {
    model.starts_with('/') || model.starts_with('.') || model.ends_with(".gguf")
}

fn main() {
    let mut model = env::var("LLAMA_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    let mut context: i32 = 16384;
    let mut port: u16 = env::var("LAC_LLAMA_PORT")
        .ok()
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or(8081);
    let mut positional = 0;

    let mut args = env::args().skip(1).peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => usage(),
            "--port" => {
                port = args.next().and_then(|p| p.parse().ok()).unwrap_or_else(|| usage());
            }
            s if s.starts_with('-') => usage(),
            m => {
                positional += 1;
                if positional == 1 {
                    model = m.to_string();
                } else if positional == 2 {
                    context = m.parse().unwrap_or_else(|_| usage());
                } else {
                    usage();
                }
            }
        }
    }
    context = context.clamp(2048, 65536);

    if !which("llama-server") {
        eprintln!("error: llama-server not found in PATH.");
        eprintln!("install: brew install llama.cpp");
        std::process::exit(1);
    }
    if looks_like_path(&model) && std::fs::metadata(&model).is_err() {
        eprintln!("error: model file not found: {}", model);
        std::process::exit(1);
    }
    if port_in_use(port) {
        eprintln!("error: 127.0.0.1:{} is already bound.", port);
        eprintln!("hint: another llama-server is up — route to it instead.");
        std::process::exit(1);
    }

    eprintln!("Starting llama-server with model: {}", model);
    eprintln!("Listening on http://127.0.0.1:{}/v1", port);
    eprintln!("Context window: {} tokens", context);
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
        Ok(s) => {
            eprintln!("llama-server stopped ({}).", s);
            std::process::exit(s.code().unwrap_or(0));
        }
        Err(e) => {
            eprintln!("failed to start llama-server: {}", e);
            std::process::exit(1);
        }
    }
}
