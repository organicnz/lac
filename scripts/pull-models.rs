//! scripts/pull-models.rs — verified model fetcher (standalone, std-only).
//!
//! Build:  rustc -O scripts/pull-models.rs -o scripts/bin/pull-models
//!         (or: make scripts; run from the repo root — this file includes
//!         ../rust-src/src/common.rs for the audited PATH lookup)
//! Usage:  ./scripts/bin/pull-models
//!
//! A dependency-free reference fallback. Day to day, prefer
//! `lac models pull` (mount-gated library paths, fuller verification).
//! Unlike the old bash script, this fetcher verifies each pull with
//! `ollama show` and exits non-zero when anything is missing.

#[path = "../rust-src/src/common.rs"]
mod common;

use std::env;
use std::fs;
use std::process::Command;

fn which(cmd: &str) -> bool {
    common::which(cmd).is_some()
}

fn main() {
    if env::args().any(|a| a == "-h" || a == "--help") {
        println!("usage: pull-models");
        println!("  pulls qwen3.8-27b into Ollama and stages MLX cache dirs");
        return;
    }
    if !which("ollama") {
        eprintln!("error: ollama not found in PATH.");
        eprintln!("install: brew install ollama");
        std::process::exit(1);
    }

    let mut failed = 0;
    for model in ["qwen3.8-27b"] {
        eprintln!("Pulling Ollama model: {}", model);
        match Command::new("ollama").arg("pull").arg(model).status() {
            Ok(s) if s.success() => {
                match Command::new("ollama").args(["show", model]).output() {
                    Ok(o) if o.status.success() => eprintln!("  verified: {} present", model),
                    _ => {
                        eprintln!("  WARNING: pull exited 0 but `ollama show {}` failed", model);
                        failed += 1;
                    }
                }
            }
            Ok(s) => {
                eprintln!("  ollama pull exit: {}", s);
                failed += 1;
            }
            Err(e) => {
                eprintln!("  failed to pull {}: {}", model, e);
                failed += 1;
            }
        }
    }

    eprintln!("Ensuring MLX model cache directories exist");
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    for d in [
        format!("{}/.cache/huggingface/hub/models--mlx-community--Qwen3.8-27B-4bit", home),
        format!("{}/.cache/huggingface/hub/models--mlx-community--Qwen3.8-27B-8bit", home),
    ] {
        match fs::create_dir_all(&d) {
            Ok(()) => eprintln!("  ensured: {}", d),
            Err(e) => {
                eprintln!("  FAILED {}: {}", d, e);
                failed += 1;
            }
        }
    }

    if failed > 0 {
        eprintln!("Model pull complete with {} failure(s).", failed);
        std::process::exit(1);
    }
    eprintln!("Model pull complete.");
    eprintln!("Next: 1) lac route --daemon  2) lac serve mlx");
}
