//! scripts/serve-mlx.rs — verified mlx-lm launcher (standalone, std-only).
//!
//! Build:  rustc -O scripts/serve-mlx.rs -o scripts/bin/serve-mlx
//!         (or: make scripts; run from the repo root — this file includes
//!         ../rust-src/src/common.rs for the audited PATH lookup)
//! Usage:  ./scripts/bin/serve-mlx [MODEL] [--port PORT]
//!
//! A dependency-free reference fallback. Day to day, prefer
//! `lac serve mlx` (port-drift handling, thermal-aware context,
//! KeepAlive daemon, readiness reporting). Unlike the old bash script,
//! this launcher preflights the binary and the port and propagates the
//! server's exit code instead of masking failures.

#[path = "../rust-src/src/common.rs"]
mod common;

use std::env;
use std::net::{SocketAddr, TcpStream};
use std::process::Command;
use std::time::Duration;

const DEFAULT_MODEL: &str = "mlx-community/Qwen3.8-27B-4bit";

fn which(cmd: &str) -> bool {
    common::which(cmd).is_some()
}

fn port_in_use(port: u16) -> bool {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

fn usage() -> ! {
    eprintln!("usage: serve-mlx [MODEL] [--port PORT]");
    eprintln!("  MODEL defaults to {} (or $MLX_MODEL)", DEFAULT_MODEL);
    eprintln!("  PORT  defaults to 8080 (or $MLX_PORT)");
    std::process::exit(2);
}

fn main() {
    let mut model = env::var("MLX_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    let mut port: u16 = env::var("MLX_PORT")
        .ok()
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or(8080);

    let mut args = env::args().skip(1).peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => usage(),
            "--port" => {
                port = args.next().and_then(|p| p.parse().ok()).unwrap_or_else(|| usage());
            }
            s if s.starts_with('-') => usage(),
            m => model = m.to_string(),
        }
    }

    if !which("mlx_lm.server") {
        eprintln!("error: mlx_lm.server not found in PATH.");
        eprintln!("install: brew install mlx-lm   (or: uv tool install mlx-lm)");
        std::process::exit(1);
    }
    if port_in_use(port) {
        eprintln!("error: 127.0.0.1:{} is already bound.", port);
        eprintln!("hint: use --port, or run `lac serve mlx` (finds a free port).");
        std::process::exit(1);
    }

    eprintln!("Starting mlx-lm server with model: {}", model);
    eprintln!("Listening on http://127.0.0.1:{}/v1", port);
    let status = Command::new("mlx_lm.server")
        .arg("--model")
        .arg(&model)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .status();
    match status {
        Ok(s) => {
            eprintln!("mlx-lm server stopped ({}).", s);
            std::process::exit(s.code().unwrap_or(0));
        }
        Err(e) => {
            eprintln!("failed to start mlx_lm.server: {}", e);
            std::process::exit(1);
        }
    }
}
