//! pull-models v2.6 — fetch Qwen3.8 weights into the model library.
//!
//! v2.6: preflights ollama, verifies each pull with `ollama show`,
//! reports sizes, and stages cache dirs under the mount-gated library.

mod common;

use std::fs;
use std::process::Command;

fn main() {
    common::ignore_sigpipe();
    eprintln!("=== LAC pull-models v2.6 (Rust) ===");
    let home = common::home_dir();

    // 27B-class weights want headroom: warn below 32 GiB total, never fail.
    if let Some(note) = ram_fit_note(common::total_ram_gib()) {
        eprintln!("{}", note);
    }

    if let Some(p) = common::which("ollama") {
        if let Ok(o) = Command::new("ollama").arg("--version").output() {
            eprintln!("ollama: {} ({})", String::from_utf8_lossy(&o.stdout).trim(), p);
        }
    } else {
        eprintln!("ollama not found in PATH — install: brew install ollama");
        std::process::exit(1);
    }

    // qwen3.8-27B only — primary model everywhere.
    let models = ["qwen3.8-27b"];
    let mut failed = 0;
    for model in &models {
        eprintln!("Pulling Ollama model: {}", model);
        match Command::new("ollama").arg("pull").arg(model).status() {
            Ok(s) if s.success() => {
                // Verify the manifest actually landed.
                match Command::new("ollama").args(["show", model]).output() {
                    Ok(o) if o.status.success() => {
                        eprintln!("  verified: {} present", model);
                        let out = String::from_utf8_lossy(&o.stdout);
                        for line in out.lines().take(6) {
                            eprintln!("    {}", line);
                        }
                    }
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
                eprintln!("  Failed to pull {}: {}", model, e);
                failed += 1;
            }
        }
    }

    eprintln!("Setting up MLX model cache dirs...");
    let hf_hub = format!("{}/hf/hub", common::model_base());
    for d in [
        format!("{}/models--mlx-community--Qwen3.8-27B-4bit", hf_hub),
        format!("{}/models--mlx-community--Qwen3.8-27B-8bit", hf_hub),
        format!("{}/.cache/huggingface/hub/models--mlx-community--Qwen3.8-27B-4bit", home),
    ] {
        let _ = fs::create_dir_all(&d);
        eprintln!("  ensured: {}", d);
    }

    if failed > 0 {
        eprintln!("=== pull-models complete with {} failure(s) ===", failed);
        std::process::exit(1);
    }
    eprintln!("=== pull-models complete ===");
}

/// 27B-class weights need headroom (Q4 ~16GB weights + KV + OS): warn
/// below 32 GiB total RAM, never fail — the pull is still useful for a
/// bigger Mac or later. Pure in `total_gib` for host-independent tests.
fn ram_fit_note(total_gib: Option<f64>) -> Option<String> {
    match total_gib {
        Some(t) if t < 32.0 => Some(format!(
            "WARNING: {:.0} GiB total RAM — qwen3.8-27b wants 32GB+ headroom. Pull continues; prefer the Q4 lane and small contexts on this Mac.",
            t
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_on_small_macs_only() {
        assert!(ram_fit_note(Some(8.0)).is_some());
        assert!(ram_fit_note(Some(16.0)).is_some());
        assert!(ram_fit_note(Some(96.0)).is_none());
        assert!(ram_fit_note(None).is_none());
    }
}
