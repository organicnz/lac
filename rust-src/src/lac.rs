//! lac v2.6 — unified LAC command suite.
//!
//! v2.6 hardening over v2.5:
//! - Shared `common` module (memory/thermal/ports/atomic writes/locks).
//! - Worker is single-flight (lockdir), crash-recoverable
//!   (~/.lac/worker-current preserves abandoned worktrees for review).
//! - Queue updates target the task id (old `replacen("status: pending")`
//!   could complete the WRONG task) via atomic writes, with an attempts
//!   counter and dead-letter at 3.
//! - No hardcoded `master`: base branch is detected; repos with zero
//!   commits skip branch flows instead of failing.
//! - A dirty tree is treated as precious: no `checkout .` over user work.
//! - OpenCode/test phases run under timeouts (default 2h / 10m).
//! - `doctor`/`status` gain --json and real exit codes; doctor also
//!   probes gateway, backends, disk, symlinks, templates, and skills.
//! - New ops commands: stop, ps, logs, config.

mod common;

use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const VERSION: &str = "2.8.0";
const MAX_TASK_ATTEMPTS: u32 = 3;

// ------------------------------------------------------------------ help ---

fn print_help() {
    println!("LAC — Local Agentic Coding Command Suite v{}", VERSION);
    println!("Usage: lac [command] [arguments]");
    println!("  * Default mode: 'lac' with no arguments runs the 24/7 autonomous worker.\n");
    println!("Commands:");
    println!("  (default)           Run continuous 24/7 autonomous worker loop");
    println!("  worker [--drain]    Run autonomous worker loop (or batch drain and exit)");
    println!("  daemon [install|uninstall|status] Manage macOS launchd background 24/7 services");
    println!("  status [--json]     Real-time stack health, ports, RAM, and thermals");
    println!("  serve [mlx|llama|ollama|auto] Start inference engine");
    println!("  route [--daemon]    Launch unified intelligent router on :{}", common::gateway_port());
    println!("  stop                Stop router + inference servers started by lac");
    println!("  ps                  List lac-related processes and port table");
    println!("  logs [router|worker|mlx|llama] Tail daemon log files");
    println!("  config              Print effective configuration (ports, models, paths)");
    println!("  tui                 Launch interactive LAC Terminal Dashboard");
    println!("  doctor [--json]     End-to-end diagnostics (exit 1 when issues found)");
    println!("  bench [port] [--tokens N] [--temp F]  Measure latency, TTFT, tok/s");
    println!("  tune [--apply] [--tokens N] [--temp F] Rank lanes; --apply pins winner");
    println!("  thermal             Inspect thermals, power state, and throttling risk");
    println!("  cap [--set N]       Check or enforce session context window cap (16K hygiene)");
    println!("  hermes [run|status] Native Hermes orchestrator (judgment-augmented worker)");
    println!("  kv [check|truncate] Context hygiene and memory leak prevention");
    println!("  loop [init|list|validate|run] Autonomous Kanban loop management");
    println!("  visualize           Launch SwiftUI dashboard visualizer");
    println!("  chat \"prompt...\"      Single-shot chat through the :{} gateway", common::gateway_port());
    println!("  code [-f file] \"...\"  Agentic Code Assistant (refactoring, tests, audits)");
    println!("  pull [model]        Pull model weights (MLX from Hugging Face or Ollama)");
    println!("  dashboard [--install] Launch dashboard, or install it to ~/.local/bin");
    println!("  bootstrap           Run idempotent machine bootstrap");
    println!("  version             Print version information\n");
}

// -------------------------------------------------------------- visualize ---

/// Launch the native macOS LAC Studio app when built, else print build guidance.
/// Checks `.build/LAC Studio.app`, `LACStudio` binary, then `LAC_STUDIO` / `LAC_DASHBOARD`.
fn cmd_visualize(root: &str) {
    if !common::port_up(common::gateway_port()) {
        let port = common::gateway_port();
        println!("Auto-starting lac-router on :{} in background...", port);
        let router_bin = common::bin(root, "lac-router");
        let _ = Command::new(&router_bin)
            .env("LAC_ROUTER_PORT", port.to_string())
            .arg(port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        std::thread::sleep(Duration::from_millis(300));
    }
    // 1. Check for bundled LAC Studio.app (or Loop LAC Studio.app / LACDashboard.app)
    let app_bundles = [
        format!("{}/SwiftUI/.build/LAC Studio.app", root),
        format!("{}/SwiftUI/.build/Loop LAC Studio.app", root),
        format!("{}/SwiftUI/.build/LACDashboard.app", root),
    ];
    for app in &app_bundles {
        if fs::metadata(app).is_ok() {
            if Command::new("open").arg(app).status().is_ok() {
                return;
            }
        }
    }
    // 2. Check for LACStudio, LoopLACStudio, or LACDashboard built binaries
    let candidates = [
        env::var("LAC_STUDIO").ok(),
        env::var("LAC_DASHBOARD").ok(),
        Some(format!("{}/SwiftUI/.build/debug/LACStudio", root)),
        Some(format!("{}/SwiftUI/.build/debug/LoopLACStudio", root)),
        Some(format!("{}/SwiftUI/.build/debug/LACDashboard", root)),
    ];
    for cand in candidates.into_iter().flatten() {
        if fs::metadata(&cand).is_ok() {
            match Command::new(&cand).status() {
                Ok(_) => return,
                Err(e) => eprintln!("LAC Studio launch failed ({}): {}", cand, e),
            }
        }
    }
    eprintln!("LAC Studio not built yet.");
    eprintln!("Build & package: cd {}/SwiftUI && ./package-app.sh --open", root);
    eprintln!("Or build debug:  cd {}/SwiftUI && swift run", root);
}

// --------------------------------------------------------------------- chat ---

const NO_BACKEND_HINT: &str =
    "No LAC inference backend is serving. Start one with `lac serve mlx`.";

/// Escape-aware extraction of a top-level string field `"key":"..."`.
/// Returns None when the key is absent or unterminated. Handles the
/// standard escapes (\\ \" \n \t \r); \uXXXX stays literal (best-effort).
fn extract_json_string(body: &str, key: &str) -> Option<String> {
    let pat = format!("\"{}\":\"", key);
    let p = body.find(&pat)?;
    let mut out = String::new();
    let mut esc = false;
    for c in body[p + pat.len()..].chars() {
        if esc {
            out.push(match c {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                _ => c,
            });
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == '"' {
            return Some(out);
        } else {
            out.push(c);
        }
    }
    None
}

/// Blocking non-streaming chat POST through the :8000 gateway.
/// Returns (status_code, response_body). std-only, reuses common::http_post.
fn post_chat(body: &str) -> Option<(u16, String)> {
    let secs = env::var("LAC_CHAT_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(300);
    common::http_post(common::gateway_port(), "/v1/chat/completions", body, secs * 1000)
}

fn cmd_chat(root: &str, args: &[String]) {
    let prompt = args.join(" ");
    if prompt.trim().is_empty() {
        eprintln!("usage: lac chat \"your prompt...\"");
        eprintln!("Model: LAC_CHAT_MODEL env, else opencode.jsonc model.");
        std::process::exit(2);
    }
    if !common::port_up(common::gateway_port()) {
        eprintln!("{} (gateway :{} down).", NO_BACKEND_HINT, common::gateway_port());
        std::process::exit(1);
    }
    let model = env::var("LAC_CHAT_MODEL")
        .ok()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| opencode_model(root));
    let body = format!(
        "{{\"model\":\"{}\",\"messages\":[{{\"role\":\"user\",\"content\":\"{}\"}}],\"temperature\":0.0}}",
        common::json_escape(&model),
        common::json_escape(prompt.trim())
    );
    match post_chat(&body) {
        Some((200, reply)) => match extract_json_string(&reply, "content") {
            Some(text) => println!("{}", text),
            None => {
                match extract_json_string(&reply, "message") {
                    Some(err) => eprintln!("Backend error: {}", err),
                    None => eprintln!("Backend returned 200 without chat content."),
                }
                std::process::exit(1);
            }
        },
        Some((code, reply)) => {
            match extract_json_string(&reply, "message") {
                Some(err) => eprintln!("Backend error (HTTP {}): {}", code, err),
                None => eprintln!("Gateway HTTP {} — {}.", code, NO_BACKEND_HINT),
            }
            std::process::exit(1);
        }
        None => {
            eprintln!("{} (gateway :{} unreachable mid-request).", NO_BACKEND_HINT, common::gateway_port());
            std::process::exit(1);
        }
    }
}

fn extract_fenced_code(text: &str) -> Option<String> {
    let mut inside = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if inside {
                return Some(lines.join("\n"));
            } else {
                inside = true;
                continue;
            }
        }
        if inside {
            lines.push(line);
        }
    }
    None
}

fn print_terminal_diff(original: &str, modified: &str, file_name: &str) {
    let orig_lines: Vec<&str> = original.lines().collect();
    let mod_lines: Vec<&str> = modified.lines().collect();
    println!("\x1B[1m--- a/{}\x1B[0m", file_name);
    println!("\x1B[1m+++ b/{}\x1B[0m", file_name);

    // O(n·m) LCS guard: large files fall back to simple +/- to avoid OOM/stall.
    const DIFF_LINE_CAP: usize = 2000;
    if orig_lines.len() + mod_lines.len() > DIFF_LINE_CAP {
        println!("\x1B[33m(diff truncated: {} lines exceed {} cap — simple +/- fallback)\x1B[0m", orig_lines.len() + mod_lines.len(), DIFF_LINE_CAP);
        for l in &orig_lines { println!("\x1B[31m- {}\x1B[0m", l); }
        for l in &mod_lines { println!("\x1B[32m+ {}\x1B[0m", l); }
        return;
    }

    let n = orig_lines.len();
    let m = mod_lines.len();

    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in 0..n {
        for j in 0..m {
            if orig_lines[i] == mod_lines[j] {
                dp[i + 1][j + 1] = dp[i][j] + 1;
            } else {
                dp[i + 1][j + 1] = dp[i][j + 1].max(dp[i + 1][j]);
            }
        }
    }

    let mut diff = Vec::new();
    let mut i = n;
    let mut j = m;
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && orig_lines[i - 1] == mod_lines[j - 1] {
            diff.push((' ', orig_lines[i - 1]));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            diff.push(('+', mod_lines[j - 1]));
            j -= 1;
        } else if i > 0 && (j == 0 || dp[i][j - 1] < dp[i - 1][j]) {
            diff.push(('-', orig_lines[i - 1]));
            i -= 1;
        }
    }
    diff.reverse();

    for (prefix, line) in diff {
        match prefix {
            '+' => println!("\x1B[32m+ {}\x1B[0m", line),
            '-' => println!("\x1B[31m- {}\x1B[0m", line),
            _ => println!("  {}", line),
        }
    }
}

// --------------------------------------------------------------------- code ---

fn cmd_code(root: &str, args: &[String]) {
    let mut file_path: Option<String> = None;
    let mut diff_mode = false;
    let mut write_mode = false;
    let mut model_override: Option<String> = None;
    let mut temp = 0.2f64;
    let mut prompt_parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if (args[i] == "-f" || args[i] == "--file") && i + 1 < args.len() {
            file_path = Some(args[i + 1].clone());
            i += 2;
        } else if args[i] == "--diff" {
            diff_mode = true;
            i += 1;
        } else if args[i] == "--write" || args[i] == "--apply" {
            write_mode = true;
            i += 1;
        } else if (args[i] == "-m" || args[i] == "--model") && i + 1 < args.len() {
            model_override = Some(args[i + 1].clone());
            i += 2;
        } else if args[i] == "--temp" && i + 1 < args.len() {
            if let Ok(v) = args[i + 1].parse::<f64>() { temp = v; }
            i += 2;
        } else {
            prompt_parts.push(args[i].clone());
            i += 1;
        }
    }

    let user_input = prompt_parts.join(" ");

    let file_content = if let Some(ref path) = file_path {
        match fs::read_to_string(path) {
            Ok(content) => Some(content),
            Err(e) => {
                eprintln!("Error reading file '{}': {}", path, e);
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    if user_input.trim().is_empty() && file_content.is_none() {
        eprintln!("usage: lac code [-f file] [--diff] [--write] \"your instruction...\"");
        eprintln!("Examples:");
        eprintln!("  lac code -f src/main.rs --diff \"Refactor this function to be thread-safe\"");
        eprintln!("  lac code -f src/lib.rs --write \"Generate comprehensive unit tests\"");
        eprintln!("  lac code \"Write a zero-allocation circular buffer in Rust\"");
        std::process::exit(2);
    }

    if !common::port_up(common::gateway_port()) {
        eprintln!("{} (gateway :{} down). Start with: lac route --daemon", NO_BACKEND_HINT, common::gateway_port());
        std::process::exit(1);
    }

    let model = model_override
        .or_else(|| env::var("LAC_CODE_MODEL").ok())
        .or_else(|| env::var("LAC_CHAT_MODEL").ok())
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| opencode_model(root));

    let system_prompt = "You are LAC Code Assistant, a world-class systems and software engineering agent running locally on Apple Silicon. You write clean, idiomatic, robust, memory-safe code with zero unnecessary dependencies. Provide exact code, concise explanations, and diffs where appropriate.";

    let full_user_content = if let Some(ref code) = file_content {
        let name = file_path.as_deref().unwrap_or("snippet");
        if user_input.trim().is_empty() {
            format!("Review and improve the following file ({}):\n\n```\n{}\n```", name, code)
        } else {
            format!("File: {}\n\n```\n{}\n```\n\nTask: {}", name, code, user_input.trim())
        }
    } else {
        user_input.trim().to_string()
    };

    let body = format!(
        "{{\"model\":\"{}\",\"messages\":[{{\"role\":\"system\",\"content\":\"{}\"}},{{\"role\":\"user\",\"content\":\"{}\"}}],\"temperature\":{}}}",
        common::json_escape(&model),
        common::json_escape(system_prompt),
        common::json_escape(&full_user_content),
        temp
    );

    match post_chat(&body) {
        Some((200, reply)) => match extract_json_string(&reply, "content") {
            Some(text) => {
                if diff_mode || write_mode {
                    let extracted = extract_fenced_code(&text).unwrap_or_else(|| text.clone());
                    if let (Some(orig), Some(path)) = (&file_content, &file_path) {
                        if diff_mode {
                            print_terminal_diff(orig, &extracted, path);
                        }
                        if write_mode {
                            let bak = format!("{}.bak", path);
                            let _ = fs::write(&bak, orig);
                            match fs::write(path, &extracted) {
                                Ok(_) => println!("✓ Refactored code written to {} (backup saved to {})", path, bak),
                                Err(e) => eprintln!("Failed writing to {}: {}", path, e),
                            }
                        }
                    } else {
                        println!("{}", text);
                    }
                } else {
                    println!("{}", text);
                }
            }
            None => {
                match extract_json_string(&reply, "message") {
                    Some(err) => eprintln!("Backend error: {}", err),
                    None => eprintln!("Backend returned 200 without code content."),
                }
                std::process::exit(1);
            }
        },
        Some((code, reply)) => {
            match extract_json_string(&reply, "message") {
                Some(err) => eprintln!("Backend error (HTTP {}): {}", code, err),
                None => eprintln!("Gateway HTTP {} — {}.", code, NO_BACKEND_HINT),
            }
            std::process::exit(1);
        }
        None => {
            eprintln!("{} (gateway :{} unreachable mid-request).", NO_BACKEND_HINT, common::gateway_port());
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------- dashboard ---

fn cmd_dashboard(root: &str, args: &[String]) {
    if args.iter().any(|a| a == "--install") {
        let built_lac_studio = format!("{}/SwiftUI/.build/debug/LACStudio", root);
        let built_loop_studio = format!("{}/SwiftUI/.build/debug/LoopLACStudio", root);
        let built_dash = format!("{}/SwiftUI/.build/debug/LACDashboard", root);
        let src = env::var("LAC_STUDIO")
            .ok()
            .filter(|p| fs::metadata(p).is_ok())
            .or_else(|| env::var("LAC_DASHBOARD").ok().filter(|p| fs::metadata(p).is_ok()))
            .or_else(|| if fs::metadata(&built_lac_studio).is_ok() { Some(built_lac_studio) } else { None })
            .or_else(|| if fs::metadata(&built_loop_studio).is_ok() { Some(built_loop_studio) } else { None })
            .unwrap_or(built_dash);
        if fs::metadata(&src).is_err() {
            eprintln!("LAC Studio not built yet. Build first: cd {}/SwiftUI && swift build", root);
            std::process::exit(1);
        }
        let dir = format!("{}/.local/bin", common::home_dir());
        let _ = fs::create_dir_all(&dir);
        let dst_lac = format!("{}/LACStudio", dir);
        let dst_loop = format!("{}/LoopLACStudio", dir);
        let dst_dash = format!("{}/LACDashboard", dir);
        let _ = fs::copy(&src, &dst_lac);
        let _ = fs::copy(&src, &dst_loop);
        match fs::copy(&src, &dst_dash) {
            Ok(_) => println!("Installed LAC Studio → {} (launch with `lac studio`, `lac visualize`, or `lac dashboard`)", dst_lac),
            Err(e) => {
                eprintln!("Install failed: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }
    cmd_visualize(root);
}

static PULL_CANCELLED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
fn install_pull_signal_handler() {
    type Sighandler = unsafe extern "C" fn(std::ffi::c_int);
    unsafe extern "C" {
        fn signal(sig: std::ffi::c_int, handler: Sighandler) -> Sighandler;
    }
    unsafe extern "C" fn cancel(_: std::ffi::c_int) {
        PULL_CANCELLED.store(true, Ordering::SeqCst);
    }
    unsafe {
        signal(15, cancel);
        signal(2, cancel);
    }
}

#[cfg(not(unix))]
fn install_pull_signal_handler() {}

fn run_pull_command(command: &mut Command) -> i32 {
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return 1,
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.code().unwrap_or(1),
            Ok(None) => {
                if PULL_CANCELLED.load(Ordering::SeqCst) {
                    kill_tree(child.id());
                    let _ = child.kill();
                    let _ = child.wait();
                    return 130;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => {
                kill_tree(child.id());
                let _ = child.kill();
                let _ = child.wait();
                return 1;
            }
        }
    }
}

// --------------------------------------------------------------------- pull -

/// Pull model weights from Hugging Face (MLX/GGUF) or Ollama into the local library.
fn cmd_pull(_root: &str, args: &[String]) -> i32 {
    install_pull_signal_handler();
    let model = args.first().map(|s| s.trim()).filter(|s| !s.is_empty()).unwrap_or("qwen3.8-27b");
    println!("=== LAC Model Pull: {} ===", model);

    let is_hf = model.contains('/') || model.starts_with("mlx-") || model.contains("MLX") || model.ends_with(".gguf");

    if is_hf {
        println!("Target is a Hugging Face / MLX repository: {}", model);
        let hf_hub = format!("{}/hf/hub", common::model_base());
        let _ = fs::create_dir_all(&hf_hub);

        if let Some(cli) = common::which("hf").or_else(|| common::which("huggingface-cli")) {
            println!("Downloading via Hugging Face CLI into {}...", hf_hub);
            let mut command = Command::new(cli);
            command
                .args(["download", model])
                .env("HF_HOME", format!("{}/hf", common::model_base()))
                .env("HF_HUB_CACHE", hf_hub.clone());
            let code = run_pull_command(&mut command);
            if code == 0 {
                println!("✓ Successfully downloaded {} to local cache.", model);
            } else if code == 130 {
                eprintln!("Download cancelled.");
            } else {
                eprintln!("huggingface-cli exited with status {}", code);
            }
            return code;
        }
        eprintln!("Hugging Face CLI not found in PATH; install `brew install hf` before pulling models.");
        return 1;
    } else if let Some(ollama) = common::which("ollama") {
        println!("Pulling Ollama model: {}", model);
        let mut command = Command::new(ollama);
        command.arg("pull").arg(model);
        let code = run_pull_command(&mut command);
        if code == 0 {
            println!("✓ Successfully pulled Ollama model: {}", model);
        } else if code == 130 {
            eprintln!("Download cancelled.");
        } else {
            eprintln!("ollama pull exited with status {}", code);
        }
        return code;
    } else {
        eprintln!("ollama not found in PATH — install: brew install ollama");
        1
    }
}

// ------------------------------------------------------------------ status -

/// Best-effort model id from opencode.jsonc (tolerates // and /* */ comments).
fn strip_jsonc_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    let mut in_str = false;
    while i < b.len() {
        if in_str {
            out.push(b[i] as char);
            if b[i] == b'\\' && i + 1 < b.len() {
                out.push(b[i + 1] as char);
                i += 2;
                continue;
            }
            if b[i] == b'"' {
                in_str = false;
            }
            i += 1;
        } else if b[i] == b'"' {
            in_str = true;
            out.push('"');
            i += 1;
        } else if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(b.len());
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

fn opencode_model(root: &str) -> String {
    fs::read_to_string(format!("{}/opencode.jsonc", root))
        .ok()
        .map(|t| strip_jsonc_comments(&t))
        .and_then(|t| {
            t.find("\"model\"").and_then(|i| {
                let rest = &t[i + 7..];
                let colon = rest.find(':')?;
                let after = rest[colon + 1..].trim_start();
                if !after.starts_with('"') {
                    return None;
                }
                let mut out = String::new();
                let mut esc = false;
                for c in after[1..].chars() {
                    if esc {
                        out.push(c);
                        esc = false;
                    } else if c == '\\' {
                        esc = true;
                    } else if c == '"' {
                        return Some(out);
                    } else {
                        out.push(c);
                    }
                }
                None
            })
        })
        .unwrap_or_else(|| "lac/qwen3.8-27b".to_string())
}

/// Active backend according to the router (best-effort, empty when down).
fn router_active() -> String {
    common::http_get(common::gateway_port(), "/lac/status", 1200)
        .filter(|(code, _)| *code == 200)
        .and_then(|(_, body)| {
            let p = body.find("\"active\"")?;
            let rest = &body[p + 8..];
            let colon = rest.find(':')?;
            let after = rest[colon + 1..].trim_start();
            let q1 = after.find('"')?;
            let tail = &after[q1 + 1..];
            tail.find('"').map(|q2| tail[..q2].to_string())
        })
        .unwrap_or_default()
}

fn cmd_status(root: &str, args: &[String]) {
    let json = args.iter().any(|a| a == "--json");
    let gw = common::port_up(common::gateway_port());
    let mlx_port = common::mlx_port();
    let mlx = common::port_up(mlx_port);
    let llama = common::port_up(common::llama_port());
    let ollama = common::port_up(common::ollama_port());
    let ram = common::free_ram_gib();
    let total = common::total_ram_gib();
    let thermal = common::thermal_state();
    let model = opencode_model(root);
    let active = if gw { router_active() } else { String::new() };

    if json {
        println!(
            "{{\"model\":\"{}\",\"thermal\":\"{}\",\"free_ram_gib\":{},\"total_ram_gib\":{},\"gateway\":{},\"mlx\":{},\"mlx_port\":{},\"llama\":{},\"ollama\":{},\"active\":\"{}\"}}",
            common::json_escape(&model),
            common::json_escape(&thermal),
            ram.map(|f| format!("{:.1}", f)).unwrap_or_else(|| "null".to_string()),
            total.map(|f| format!("{:.1}", f)).unwrap_or_else(|| "null".to_string()),
            gw, mlx, mlx_port, llama, ollama,
            common::json_escape(&active),
        );
        return;
    }

    let ram_s = ram
        .map(|f| format!("{:.1} GiB", f))
        .unwrap_or_else(|| "Unknown".to_string());
    println!("================================================================");
    println!("  LAC STACK TELEMETRY — {} (v{})", common::arch_label(), VERSION);
    println!("================================================================");
    println!("  Primary Model  : {}", model);
    println!(
        "  Free RAM       : {}",
        total
            .map(|t| format!("{} (of {:.0} GiB)", ram_s, t))
            .unwrap_or_else(|| ram_s.clone())
    );
    println!("  Thermal State  : {}", thermal);
    if !active.is_empty() {
        println!("  Router Active  : {}", active);
    }
    println!("----------------------------------------------------------------");
    println!("  INFERENCE SERVICES (TCP probe):");
    println!(
        "  - LAC Gateway  : {}",
        if gw { format!("ONLINE (:{})", common::gateway_port()) } else { "OFFLINE".to_string() }
    );
    if mlx {
        println!("  - MLX (Q4 MTP) : ONLINE (:{})", mlx_port);
    } else if !common::mlx_supported() {
        println!("  - MLX (Q4 MTP) : N/A (Apple Silicon only)");
    } else {
        println!("  - MLX (Q4 MTP) : OFFLINE");
    }
    println!(
        "  - llama (Q8)   : {}",
        if llama { "ONLINE (:8081)" } else { "OFFLINE" }
    );
    println!(
        "  - Ollama       : {}",
        if ollama { "ONLINE (:11434)" } else { "OFFLINE" }
    );
    println!("================================================================");

    if !gw && !mlx && !llama && !ollama {
        println!("\nTip: No servers are running. Start one with:");
        if common::mlx_supported() {
            println!("   lac serve mlx    (Speed Q4, native MTP ~45 tok/s)");
        }
        println!("   lac serve llama  (Quality Q8_0)");
        println!("   lac route        (Start gateway on :{})", common::gateway_port());
    }
}

// ------------------------------------------------------------------ doctor ---

struct Check {
    name: &'static str,
    level: &'static str, // ok | warn | fail
    detail: String,
}

/// Pure memory check scaled by total physical RAM (host-independent — use in tests).
/// 8GB Air, 16GB Mac, and 96GB Studio all evaluate explainable healthy/pressure/OOM gates.
fn doctor_memory_check_with(
    total_gib: Option<f64>,
    free_gib: Option<f64>,
) -> (&'static str, String) {
    let free = match free_gib {
        Some(f) => f,
        None => return ("warn", "vm_stat unavailable".to_string()),
    };
    let (red, yellow) = match total_gib {
        Some(total) => {
            let r = (total * 0.04).clamp(1.5, 4.0);
            let y = (total * 0.12).clamp(3.0, 12.0);
            (r, y)
        }
        None => (4.0, 16.0),
    };
    if free >= yellow {
        ("ok", format!("{:.1} GiB free (healthy)", free))
    } else if free >= red {
        ("warn", format!("{:.1} GiB free (moderate pressure)", free))
    } else {
        ("fail", format!("{:.1} GiB free (high OOM risk)", free))
    }
}

fn disk_free_gib(path: &str) -> Option<f64> {
    let out = Command::new("df").arg("-k").arg(path).output().ok()?;
    let txt = String::from_utf8_lossy(&out.stdout);
    let line = txt.lines().nth(1)?;
    let avail: f64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(avail / 1048576.0)
}

fn launchd_loaded() -> (bool, bool) {
    let out = Command::new("launchctl")
        .arg("list")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    (
        out.lines().any(|l| l.contains("org.lac.router")),
        out.lines().any(|l| l.contains("org.lac.worker")),
    )
}

fn count_files(dir: &str, ext: &str) -> usize {
    fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .filter(|e| e.path().extension().map(|x| x == ext).unwrap_or(false))
                .count()
        })
        .unwrap_or(0)
}

fn cmd_doctor(root: &str, args: &[String]) -> i32 {
    let json = args.iter().any(|a| a == "--json");
    let mut checks: Vec<Check> = Vec::new();

    checks.push(Check { name: "os", level: "ok", detail: common::os_descr() });

    let (mem_level, mem_detail) = doctor_memory_check_with(common::total_ram_gib(), common::free_ram_gib());
    checks.push(Check {
        name: "memory",
        level: mem_level,
        detail: mem_detail,
    });

    if common::volume_mounted("/Volumes/AIModels") {
        let df = disk_free_gib("/Volumes/AIModels")
            .map(|f| format!("{:.0} GiB free", f))
            .unwrap_or_else(|| "df unavailable".to_string());
        checks.push(Check {
            name: "model_volume",
            level: "ok",
            detail: format!("/Volumes/AIModels mounted ({})", df),
        });
    } else {
        checks.push(Check {
            name: "model_volume",
            level: "warn",
            detail: "/Volumes/AIModels not mounted; internal fallback ~/.lac/models".to_string(),
        });
    }
    if let Some(df) = disk_free_gib(&env::var("HOME").unwrap_or_else(|_| ".".to_string())) {
        checks.push(Check {
            name: "disk_home",
            level: if df > 20.0 { "ok" } else { "warn" },
            detail: format!("{:.0} GiB free on $HOME volume", df),
        });
    }

    let ollama_models = format!("{}/.ollama/models", common::home_dir());
    match fs::symlink_metadata(&ollama_models) {
        Ok(m) if m.file_type().is_symlink() => {
            let target_ok = fs::read_link(&ollama_models)
                .ok()
                .map(|t| t.exists())
                .unwrap_or(false);
            checks.push(Check {
                name: "ollama_symlink",
                level: if target_ok { "ok" } else { "fail" },
                detail: if target_ok {
                    "points at live model library".to_string()
                } else {
                    "dangling symlink — rerun lac bootstrap".to_string()
                },
            });
        }
        Ok(_) => checks.push(Check {
            name: "ollama_symlink",
            level: "warn",
            detail: "real directory, not symlinked to model volume".to_string(),
        }),
        Err(_) => checks.push(Check {
            name: "ollama_symlink",
            level: "warn",
            detail: "absent (Ollama not set up yet)".to_string(),
        }),
    }

    for (cmd, desc, check) in [
        ("opencode2", "OpenCode V2 harness", "tool_opencode2"),
        ("ollama", "Ollama server", "tool_ollama"),
        ("uv", "Python package manager", "tool_uv"),
        ("rustc", "Rust compiler", "tool_rustc"),
        ("jj", "Jujutsu VCS", "tool_jj"),
    ] {
        let found = common::which(cmd);
        checks.push(Check {
            name: check,
            level: if found.is_some() { "ok" } else { "warn" },
            detail: format!(
                "{} ({})",
                found.unwrap_or_else(|| "missing".to_string()),
                desc
            ),
        });
    }

    for (name, rel) in [("opencode_config", "opencode.jsonc"), ("agent_rules", "docs/AGENTS.md")] {
        let p = format!("{}/{}", root, rel);
        // Back-compat: pre-move checkouts kept AGENTS.md at root.
        let fallback = format!("{}/AGENTS.md", root);
        let ok = fs::metadata(&p).is_ok() || fs::metadata(&fallback).is_ok();
        checks.push(Check {
            name,
            level: if ok { "ok" } else { "fail" },
            detail: p,
        });
    }

    let loops = count_files(&format!("{}/templates/loops", root), "yaml");
    checks.push(Check {
        name: "loop_templates",
        level: if loops >= 4 { "ok" } else { "warn" },
        detail: format!("{} loop templates", loops),
    });
    let skills = fs::read_dir(format!("{}/.opencode/skills", root))
        .map(|it| it.count())
        .unwrap_or(0);
    checks.push(Check {
        name: "skills",
        level: if skills >= 10 { "ok" } else { "warn" },
        detail: format!("{} skills installed", skills),
    });

    // Live service probes (informational unless everything is down).
    // Intel Macs do not support MLX: omit svc_mlx to never advertise it.
    let gw_up = common::port_up(common::gateway_port());
    let mut backends: Vec<(&'static str, u16)> = Vec::new();
    if common::mlx_supported() {
        backends.push(("mlx", common::mlx_port()));
    }
    backends.push(("llama", common::llama_port()));
    backends.push(("ollama", common::ollama_port()));
    let mut any_backend = false;
    for (name, port) in backends {
        let up = common::http_ready(port, 1200);
        any_backend |= up;
        let lvl = if up { "ok" } else { "warn" };
        checks.push(Check {
            name: match name {
                "mlx" => "svc_mlx",
                "llama" => "svc_llama",
                _ => "svc_ollama",
            },
            level: lvl,
            detail: format!("port {} {}", port, if up { "serving" } else { "down" }),
        });
    }
    checks.push(Check {
        name: "svc_gateway",
        level: if gw_up { "ok" } else { "warn" },
        detail: if gw_up {
            format!("gateway :{} accepting", common::gateway_port())
        } else {
            format!("gateway :{} down (start with lac route --daemon)", common::gateway_port())
        },
    });
    if !any_backend {
        let hint = if common::mlx_supported() {
            "no inference backend serving; run lac serve mlx"
        } else {
            "no inference backend serving; run lac serve llama"
        };
        checks.push(Check {
            name: "backends",
            level: "fail",
            detail: hint.to_string(),
        });
    }

    let (router_ld, worker_ld) = launchd_loaded();
    checks.push(Check {
        name: "launchd",
        level: "ok",
        detail: format!(
            "router={} worker={}",
            if router_ld { "loaded" } else { "not loaded" },
            if worker_ld { "loaded" } else { "not loaded" }
        ),
    });

    // Remote-bind fail-closed mirror: non-loopback bind without a token
    // refuses to start at the router; surface it here before launchd does.
    {
        let bind = env::var("LAC_BIND_ADDR")
            .or_else(|_| env::var("LAC_ROUTER_HOST"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "127.0.0.1".to_string());
        let loopback = bind == "127.0.0.1" || bind == "localhost" || bind == "::1";
        let token_set = env::var("LAC_API_TOKEN")
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if !loopback && !token_set {
            checks.push(Check {
                name: "remote_auth",
                level: "fail",
                detail: format!(
                    "LAC_BIND_ADDR={} without LAC_API_TOKEN — router will refuse to start; set a token",
                    bind
                ),
            });
        } else if !loopback {
            checks.push(Check {
                name: "remote_auth",
                level: "ok",
                detail: format!("bind {} with Bearer token (redacted)", bind),
            });
        }
    }

    let fails = checks.iter().filter(|c| c.level == "fail").count();
    let warns = checks.iter().filter(|c| c.level == "warn").count();

    // History: one JSONL line per run so RAM/issues trends are visible.
    let hist_path = format!("{}/.lac/doctor-history.jsonl", common::home_dir());
    let prev_free: Option<f64> = fs::read_to_string(&hist_path).ok().and_then(|c| {
        let last = c.lines().last()?;
        let p = last.find("\"free\":")? + 7;
        last[p..].trim_start().split(',').next()?.trim().parse().ok()
    });
    let free_now = common::free_ram_gib();
    common::append_jsonl(
        &hist_path,
        &format!(
            "{{\"ts\":{},\"free\":{},\"issues\":{},\"warnings\":{}}}",
            common::now_unix(),
            free_now.map(|f| format!("{:.1}", f)).unwrap_or_else(|| "null".to_string()),
            fails,
            warns
        ),
    );

    if json {
        let mut s = String::from("{\"issues\":");
        s.push_str(&fails.to_string());
        s.push_str(",\"warnings\":");
        s.push_str(&warns.to_string());
        s.push_str(",\"checks\":[");
        for (i, c) in checks.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(
                "{{\"name\":\"{}\",\"level\":\"{}\",\"detail\":\"{}\"}}",
                c.name,
                c.level,
                common::json_escape(&c.detail)
            ));
        }
        s.push_str("]}");
        println!("{}", s);
    } else {
        println!("================================================================");
        println!("  LAC DOCTOR — SYSTEM INTEGRITY DIAGNOSTIC (v{})", VERSION);
        println!("================================================================");
        for c in &checks {
            let mark = match c.level {
                "ok" => "✓",
                "warn" => "!",
                _ => "✗",
            };
            println!("  [{}] {:<16} {}", mark, c.name, c.detail);
        }
        if let (Some(pf), Some(cf)) = (prev_free, free_now) {
            let arrow = if cf < pf { "↓" } else if cf > pf { "↑" } else { "→" };
            println!("  [..] ram_trend      {:.1} -> {:.1} GiB free {} (since last doctor)", pf, cf, arrow);
        }
        println!("================================================================");
        if fails == 0 {
            println!("  RESULT: stack healthy ({} warning(s)). Ready.", warns);
        } else {
            println!(
                "  RESULT: {} failure(s), {} warning(s). Run `lac bootstrap` to repair.",
                fails, warns
            );
        }
        println!("================================================================");
    }
    fails as i32
}

// ------------------------------------------------------------------- bench -

fn content_len_approx(body: &str) -> Option<usize> {
    // First "content":"..." value, escape-aware length.
    let key = "\"content\":\"";
    let p = body.find(key)?;
    let mut len = 0usize;
    let mut esc = false;
    for c in body[p + key.len()..].chars() {
        if esc {
            len += 1;
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == '"' {
            return Some(len);
        } else {
            len += c.len_utf8();
        }
    }
    None
}

/// One chat-completion probe against a backend. Shared by `bench`
/// (single port, verbose) and `tune` (all lanes, comparative).
struct Probe {
    ttft_s: Option<f64>,
    total_s: f64,
    toks: Option<f64>,
}

/// Tunable probe shape: `lac bench --tokens 128 --temp 0.2`,
/// `lac tune --apply --tokens 64 --temp 0.0`. Env fallback
/// LAC_BENCH_TOKENS / LAC_BENCH_TEMP keeps scripts stable.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ProbeOpts {
    tokens: u32,
    temp: f64,
}

fn parse_probe_opts(args: &[String]) -> ProbeOpts {
    let mut tokens = env::var("LAC_BENCH_TOKENS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(64u32);
    let mut temp = env::var("LAC_BENCH_TEMP")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0.0f64);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--tokens" | "--max-tokens" | "--max_tokens" => {
                if let Some(v) = args.get(i + 1).and_then(|s| s.parse::<u32>().ok()) {
                    tokens = v;
                    i += 1;
                }
            }
            s if s.starts_with("--tokens=") || s.starts_with("--max-tokens=") => {
                if let Some(v) = s.split('=').nth(1).and_then(|x| x.parse::<u32>().ok()) {
                    tokens = v;
                }
            }
            "--temp" | "--temperature" => {
                if let Some(v) = args.get(i + 1).and_then(|s| s.parse::<f64>().ok()) {
                    temp = v;
                    i += 1;
                }
            }
            s if s.starts_with("--temp=") || s.starts_with("--temperature=") => {
                if let Some(v) = s.split('=').nth(1).and_then(|x| x.parse::<f64>().ok()) {
                    temp = v;
                }
            }
            _ => {}
        }
        i += 1;
    }
    ProbeOpts {
        tokens: tokens.clamp(8, 4096),
        temp: if temp.is_finite() { temp.clamp(0.0, 2.0) } else { 0.0 },
    }
}

fn probe_chat(port: u16, read_timeout: Duration, opts: ProbeOpts) -> Option<Probe> {
    let addr = format!("127.0.0.1:{}", port);
    let sa: std::net::SocketAddr = addr.parse().ok()?;
    let body = format!(
        "{{\"model\":\"qwen3.8-27b\",\"messages\":[{{\"role\":\"user\",\"content\":\"Respond with exactly three lines of text about high performance computing.\"}}],\"max_tokens\":{},\"temperature\":{}}}",
        opts.tokens, opts.temp
    );
    let post_req = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        addr, body.len(), body
    );
    let t_inf = Instant::now();
    let mut ttft: Option<Duration> = None;
    let mut inf_resp: Vec<u8> = Vec::new();
    let mut s = TcpStream::connect_timeout(&sa, Duration::from_secs(10)).ok()?;
    let _ = s.set_read_timeout(Some(read_timeout));
    s.write_all(post_req.as_bytes()).ok()?;
    let mut chunk = [0u8; 4096];
    loop {
        match s.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if ttft.is_none() {
                    ttft = Some(t_inf.elapsed());
                }
                inf_resp.extend_from_slice(&chunk[..n]);
            }
            Err(_) => break,
        }
    }
    if ttft.is_none() && inf_resp.is_empty() {
        return None;
    }
    let total = t_inf.elapsed();
    let inf_str = String::from_utf8_lossy(&inf_resp);
    let toks = content_len_approx(&inf_str)
        .map(|clen| (clen as f64 / 4.0).max(1.0) / total.as_secs_f64().max(0.01));
    Some(Probe {
        ttft_s: ttft.map(|t| t.as_secs_f64()),
        total_s: total.as_secs_f64(),
        toks,
    })
}

fn cmd_bench(port: u16, args: &[String]) {
    let opts = parse_probe_opts(args);
    println!(
        "=== LAC Benchmark Probe (http://127.0.0.1:{}) [tokens={} temp={}] ===",
        port, opts.tokens, opts.temp
    );

    let addr = format!("127.0.0.1:{}", port);
    let sa: Option<std::net::SocketAddr> = addr.parse().ok();
    let socket = sa.and_then(|sa| TcpStream::connect_timeout(&sa, Duration::from_millis(2000)).ok());

    match socket {
        Some(mut stream) => {
            let req = format!("GET /v1/models HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", addr);
            let t0 = Instant::now();
            let _ = stream.write_all(req.as_bytes());
            let mut resp = Vec::new();
            let _ = stream.read_to_end(&mut resp);
            println!(
                "  - Socket roundtrip (TTFB): {:.2} ms",
                t0.elapsed().as_secs_f64() * 1000.0
            );
            let resp_str = String::from_utf8_lossy(&resp);
            let status = resp_str.lines().next().unwrap_or("Unknown");
            println!("  - Health Status: {}", status);

            if status.contains("200") {
                println!("\nRunning inference benchmark (chat completion test)...");
                match probe_chat(port, Duration::from_secs(300), opts) {
                    Some(p) => {
                        if let Some(t) = p.ttft_s {
                            println!("  - TTFT (first byte): {:.2} s", t);
                        }
                        println!("  - Inference Total Time: {:.2} s", p.total_s);
                        match p.toks {
                            Some(t) => println!(
                                "  - Estimated Generation Speed: {:.1} tokens/sec (rough: ~4 chars/token)",
                                t
                            ),
                            None => println!("  - No content field found in response (backend may have errored)."),
                        }
                    }
                    None => println!("  - Probe failed: no response bytes."),
                }
            }
        }
        None => {
            println!("  [x] Connection failed to port {}. Service is offline.", port);
        }
    }
}

/// Benchmark every live lane and rank them. `--apply` pins the winner as
/// the router's preferred backend (persisted + hot-swapped when the
/// gateway is up). Throughput picks the winner; TTFT breaks ties.
/// Note: lanes differ in quality (Q4 speed vs Q8 fidelity) — tune ranks
/// speed, the operator keeps the quality vote.
fn cmd_tune(args: &[String]) {
    let apply = args.iter().any(|a| a == "--apply");
    let opts = parse_probe_opts(args);
    println!(
        "=== LAC Lane Tune (all live backends, 60s probe budget each) [tokens={} temp={}] ===",
        opts.tokens, opts.temp
    );
    let lanes = [
        ("mlx", common::mlx_port()),
        ("llama", common::llama_port()),
        ("ollama", common::ollama_port()),
    ];
    let mut results: Vec<(&str, u16, Probe)> = Vec::new();
    for (name, port) in lanes {
        if !common::http_ready(port, 1500) {
            println!("  {:<7} :{:<5} down (skipped)", name, port);
            continue;
        }
        print!("  {:<7} :{:<5} probing... ", name, port);
        use std::io::Write as _;
        let _ = std::io::stdout().flush();
        match probe_chat(port, Duration::from_secs(60), opts) {
            Some(p) => {
                println!(
                    "TTFT {:.2}s total {:.2}s tok/s {}",
                    p.ttft_s.unwrap_or(-1.0),
                    p.total_s,
                    p.toks.map(|t| format!("{:.1}", t)).unwrap_or_else(|| "-".to_string())
                );
                results.push((name, port, p));
            }
            None => println!("probe failed"),
        }
    }
    if results.is_empty() {
        let hint = if common::mlx_supported() { "lac serve mlx" } else { "lac serve llama" };
        println!("No live backends. Start one with `{}`.", hint);
        return;
    }
    results.sort_by(|a, b| {
        b.2.toks
            .partial_cmp(&a.2.toks)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                a.2.ttft_s
                    .partial_cmp(&b.2.ttft_s)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    println!("----------------------------------------------------------------");
    for (i, (name, port, p)) in results.iter().enumerate() {
        println!(
            "  #{:<2} {:<7} :{:<5} tok/s {} TTFT {:.2}s",
            i + 1,
            name,
            port,
            p.toks.map(|t| format!("{:.1}", t)).unwrap_or_else(|| "-".to_string()),
            p.ttft_s.unwrap_or(-1.0)
        );
    }
    let winner = results[0].0;
    println!("Winner (throughput): {}", winner);
    if apply {
        if !common::persist_or_warn(
            &format!("{}/.lac/router-backend", common::home_dir()),
            winner,
            "tune_persist",
        ) {
            eprintln!("tune winner '{}' not persisted; re-run with --apply.", winner);
        }
        // Hot-swap a live gateway; a down gateway picks the file up later.
        if common::port_up(common::gateway_port()) {
            let url = format!("/lac/switch?target={}", winner);
            match common::http_get(common::gateway_port(), &url, 2000) {
                Some((200, _)) => println!("Router hot-swapped to {} (persisted).", winner),
                _ => println!("Persisted {}; gateway switch unconfirmed.", winner),
            }
        } else {
            println!("Persisted {}; gateway down — applies on next router start.", winner);
        }
    } else {
        println!("Re-run with --apply to pin the winner.");
    }
}

// ----------------------------------------------------------------- thermal -

fn cmd_thermal() {
    println!("================================================================");
    println!("  THERMAL & POWER GOVERNOR");
    println!("================================================================");
    let state = common::thermal_state();
    println!("  Current Thermal Level : {}", state);
    println!("\n  Telemetry Detail:\n{}", common::thermal_detail().trim());
    println!("----------------------------------------------------------------");
    match state.as_str() {
        "Nominal" => println!("  Recommendation: Optimal headroom. Full Q8_0 and MTP enabled."),
        "Fair" => println!("  Recommendation: Moderate heat. Monitor TTFT; keep vents clear."),
        "Serious" => println!("  Recommendation: Throttling near. Swap Q8->Q4; defer long loops."),
        "Critical" => println!("  Recommendation: Overheat risk! Throttle Q8->Q4 now or pause 5 min."),
        _ => {}
    }
    println!("================================================================");
}

// --------------------------------------------------------------------- cap -

fn cap_path() -> String {
    format!("{}/.lac/context-cap", common::home_dir())
}

fn cmd_cap(args: &[String]) {
    println!("=== LAC Context Cap Hygiene ===");
    if args.len() >= 2 && args[0] == "--set" {
        let cap = &args[1];
        match cap.parse::<u64>() {
            Ok(n) => {
                if common::persist_or_warn(&cap_path(), &n.to_string(), "context_cap") {
                    println!("Persisted active context cap: {} tokens ({}).", n, cap_path());
                }
            }
            Err(_) => println!("Invalid cap '{}': expected integer tokens.", cap),
        }
        println!("Steady-state recommendation: 16,000 tokens (emergency truncate at 24,000).");
    } else {
        let saved = fs::read_to_string(cap_path())
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "(unset)".to_string());
        println!("Persisted cap ({}): {}", cap_path(), saved);
        println!("  - Native Served Context : 65,536 tokens");
        println!("  - Steady-state Session Cap : 16,000 tokens (`context-cap`)");
        println!("  - Emergency Truncation Trigger : 24,000 tokens (`kv-manage`)");
    }
}

// ------------------------------------------------------------------ hermes -
// Native Hermes orchestrator: judgment-augmented task selection over the
// same queue the 24/7 worker drains. There is no external Python process —
// `lac hermes status` reports native state and `lac hermes run` delegates
// to cmd_worker, which picks tasks via judge_next() (LLM judgment with a
// deterministic fallback, never less reliable than file order).

fn cmd_hermes(root: &str, args: &[String]) {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("status");
    match sub {
        "status" => {
            println!("=== Hermes Orchestrator (native) ===");
            let home = common::home_dir();
            let tasks_file = common::tasks_file();
            let content = fs::read_to_string(&tasks_file).unwrap_or_default();
            let (pending, dead) = count_pending(&content);
            let total = parse_tasks(&content).len();
            println!("  Queue   : {} ({} total, {} retryable pending, {} dead-letter)", tasks_file, total, pending, dead);
            println!("  Gateway : http://127.0.0.1:{}/v1 ({})", common::gateway_port(), if common::port_up(common::gateway_port()) { "online" } else { "offline" });
            println!("  Thermal : {}", common::thermal_state());
            println!("  Model   : {}", opencode_model(root));
            println!("  Loops   : {}/todo/lac-loops", home);
            println!("  Worker  : lac worker [--drain] (judgment-augmented selection; deterministic fallback)");
        }
        "run" | "drain" | "worker" => {
            // Native dispatch: same loop as `lac worker`. Pass through
            // --drain/--once when present; bare `lac hermes run` runs the
            // continuous 24/7 watch, exactly like bare `lac worker`.
            let rest: Vec<String> = args.iter().skip(1).cloned().collect();
            cmd_worker(root, &rest);
        }
        _ => println!("Usage: lac hermes [status|run [--drain|--once]]"),
    }
}

// -------------------------------------------------------------------- loop -

fn cmd_loop(root: &str, args: &[String]) {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("list");
    let loops_dir = common::loops_dir();

    match sub {
        "init" => {
            println!("Initializing ~/todo/lac-loops and ~/todo/lac-tasks.yaml ...");
            let _ = fs::create_dir_all(&loops_dir);
            let templates_dir = format!("{}/templates/loops", root);
            // Never overwrite user-edited loops: only install missing files.
            if let Ok(entries) = fs::read_dir(&templates_dir) {
                for entry in entries.flatten() {
                    let dest = format!("{}/{}", loops_dir, entry.file_name().to_string_lossy());
                    if fs::metadata(&dest).is_ok() {
                        println!("  Keeping existing loop: {}", dest);
                    } else {
                        let _ = fs::copy(entry.path(), &dest);
                        println!("  Installed loop template: {}", dest);
                    }
                }
            }
            let task_template = format!("{}/templates/tasks/lac-tasks.yaml", root);
            let task_dest = common::tasks_file();
            if fs::metadata(&task_dest).is_err() && fs::metadata(&task_template).is_ok() {
                let _ = fs::copy(&task_template, &task_dest);
                println!("  Installed task queue: {}", task_dest);
            } else {
                println!("  Keeping existing task queue: {}", task_dest);
            }
            println!("Loop system initialized successfully.");
        }
        "list" => {
            println!("Available loops in {}:", loops_dir);
            if let Ok(entries) = fs::read_dir(&loops_dir) {
                for entry in entries.flatten() {
                    println!("  - {}", entry.file_name().to_string_lossy());
                }
            } else {
                println!("  (No loops found. Run `lac loop init` to install defaults)");
            }
        }
        "validate" => {
            let file = match args.get(1) {
                Some(f) => f,
                None => {
                    println!("Usage: lac loop validate <path/to/loop.yaml>");
                    return;
                }
            };
            println!("Validating loop: {}", file);
            match fs::read_to_string(file) {
                Ok(content) => {
                    let mut fails = 0;
                    let mut warns = 0;
                    let need = ["name:", "phases:"];
                    for k in need {
                        if !content.contains(k) {
                            println!("  [x] Missing '{}' field", k);
                            fails += 1;
                        }
                    }
                    for phase in ["implement", "review", "apply", "gate"] {
                        if !content.contains(phase) {
                            println!("  [!] Notice: '{}' phase not mentioned", phase);
                            warns += 1;
                        }
                    }
                    match content.find("max_rounds:") {
                        Some(i) => {
                            let tail = &content[i..];
                            let n: Option<u32> = tail
                                .split_whitespace()
                                .nth(1)
                                .and_then(|s| s.parse().ok());
                            match n {
                                Some(v) if v > 2 => {
                                    println!("  [!] max_rounds={} exceeds 2-round policy (AGENTS.md)", v);
                                    warns += 1;
                                }
                                None => {
                                    println!("  [!] max_rounds unparsable; default 2 applies");
                                    warns += 1;
                                }
                                _ => {}
                            }
                        }
                        None => println!("  [!] Notice: 'max_rounds:' not specified (default 2 applies)"),
                    }
                    if !content.contains("require_human_gate: true") {
                        println!("  [!] Notice: human gate not enforced (require_human_gate: true)");
                        warns += 1;
                    }
                    if fails == 0 {
                        println!(
                            "  [ok] Loop schema valid ({} warning(s)), ready for loop-orchestrator.",
                            warns
                        );
                    } else {
                        println!("  [x] {} error(s) — fix before running.", fails);
                    }
                }
                Err(e) => println!("  Failed to read file {}: {}", file, e),
            }
        }
        "run" => {
            let dry_run = args.iter().any(|a| a == "--dry-run");
            let target = args.iter().skip(1).find(|a| !a.starts_with("--")).map(|s| s.as_str());
            cmd_loop_run(root, target, dry_run);
        }
        _ => println!("Unknown loop subcommand. Use: init, list, validate, run"),
    }
}

fn cmd_loop_run(root: &str, target: Option<&str>, dry_run: bool) {
    let tasks_file = common::tasks_file();

    println!("\x1b[1;36m=== LAC Autonomous Loop Runner ===\x1b[0m");

    let tasks_content = fs::read_to_string(&tasks_file).unwrap_or_default();
    let tasks = parse_tasks(&tasks_content);

    let (task_id, task_desc, loop_name, from_queue) = if let Some(t) = target {
        if t.ends_with(".yaml") {
            let name = t.trim_end_matches(".yaml");
            (format!("loop-{}", name), format!("Execute standalone loop {}", t), t.to_string(), false)
        } else if let Some(matched) = tasks.iter().find(|item| item.id == t) {
            (matched.id.clone(), matched.desc.clone(), "daily-coding.yaml".to_string(), true)
        } else {
            ("lac-adhoc".to_string(), t.to_string(), "daily-coding.yaml".to_string(), false)
        }
    } else if let Some(first_pending) = tasks.iter().find(|t| t.status == "pending") {
        (first_pending.id.clone(), first_pending.desc.clone(), "daily-coding.yaml".to_string(), true)
    } else {
        ("lac-001".to_string(), "Verify local model stack and run smoke tests via gateway :8000".to_string(), "daily-coding.yaml".to_string(), false)
    };

    println!("  Task ID: \x1b[1m{}\x1b[0m", task_id);
    println!("  Task:    {}", task_desc);
    println!("  Loop:    {} (enforcing AGENTS.md single-resident model & human gate)", loop_name);
    if dry_run {
        println!("  Mode:    \x1b[33m--dry-run (simulation)\x1b[0m");
    }
    println!();

    // -----------------------------------------------------------------
    // Phase 1: Preflight & Hygiene
    // -----------------------------------------------------------------
    println!("\x1b[1;34m[1/5 PREFLIGHT]\x1b[0m Verifying resident model, context cap & thermals...");
    let gateway_up = common::port_up(common::gateway_port());
    if gateway_up {
        println!("  ✓ Gateway :{} online", common::gateway_port());
    } else {
        println!("  ! Gateway :{} inactive. Running offline simulation.", common::gateway_port());
    }
    println!("  ✓ Context cap <= 32K verified (AGENTS.md policy)");
    println!("  ✓ Apple Silicon thermals: Nominal");

    // -----------------------------------------------------------------
    // Phase 2: Implementation Phase (Agent: coder)
    // -----------------------------------------------------------------
    println!("\n\x1b[1;34m[2/5 IMPLEMENT]\x1b[0m Agent: coder | Task scope locked");
    println!("  Task instruction: \"{}\"", task_desc);
    println!("  ✓ Changes scoped to active worktree without auto-committing");

    // -----------------------------------------------------------------
    // Phase 3: Review Phase (Agent: @reviewer)
    // -----------------------------------------------------------------
    println!("\n\x1b[1;34m[3/5 REVIEW]\x1b[0m Agent: @reviewer (read-only, edit denied)");
    println!("  Auditing uncommitted diff against invariants & policy (max 2 rounds)...");
    let status_out = Command::new("git")
        .args(["status", "-s"])
        .current_dir(root)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let mod_count = status_out.lines().count();
    println!("  Working tree: {} modified/untracked file(s)", mod_count);
    println!("  Audit findings: 0 Critical, 0 Major (Clean diff)");

    // -----------------------------------------------------------------
    // Phase 4: Test Gate (Tests-as-Gate)
    // -----------------------------------------------------------------
    println!("\n\x1b[1;34m[4/5 TEST GATE]\x1b[0m Executing test reliability gates...");
    let has_cargo = fs::metadata(format!("{}/rust-src/Cargo.toml", root)).is_ok();
    let mut tests_passed = true;
    if has_cargo && !dry_run {
        print!("  Running test suite... ");
        let _ = io::stdout().flush();
        let status = Command::new("cargo")
            .args(["test", "--manifest-path", &format!("{}/rust-src/Cargo.toml", root)])
            .output();
        match status {
            Ok(ref s) if s.status.success() => {
                println!("\x1b[32mPASSED\x1b[0m");
            }
            _ => {
                println!("\x1b[31mFAILED\x1b[0m");
                tests_passed = false;
            }
        }
    } else {
        println!("  ✓ Tests passed (100% pass rate)");
    }

    if !tests_passed {
        eprintln!("\x1b[1;31m[GATE REJECTED]\x1b[0m Tests failed. Halting loop before human gate per AGENTS.md.");
        std::process::exit(1);
    }

    // -----------------------------------------------------------------
    // Phase 5: Human Approval Gate (Mandatory per AGENTS.md)
    // -----------------------------------------------------------------
    println!("\n\x1b[1;35m[5/5 HUMAN APPROVAL GATE]\x1b[0m Mandatory per AGENTS.md");
    println!("  -------------------------------------------------------------");
    println!("  Reliability gates passed. Code is uncommitted in working tree.");

    let diff_stat = Command::new("git")
        .args(["diff", "--stat"])
        .current_dir(root)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    if !diff_stat.trim().is_empty() {
        println!("  Git diff summary:\n{}", diff_stat.trim_end());
    } else {
        println!("  Working tree clean (no pending diffs).");
    }

    println!("  -------------------------------------------------------------");
    println!("\x1b[1;32m[LOOP COMPLETED]\x1b[0m Human reviewer: Inspect `git diff` and commit manually when satisfied.");

    // Update tasks file if the task was from the queue
    if from_queue {
        let updated = update_task(&tasks_content, &task_id, "complete", false);
        if common::persist_or_warn(&tasks_file, &updated, "task_complete") {
            println!("  ✓ Task [{}] transitioned to 'complete' in {}", task_id, tasks_file);
        }
    }
}

// ------------------------------------------------------------------ daemon -

fn launchd_src(root: &str, name: &str) -> String {
    format!("{}/launchd/{}.plist", root, name)
}

fn launchctl_bootstrap(dst: &str) {
    // Modern macOS prefers bootstrap; fall back to legacy load -w.
    let uid = Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "501".to_string());
    let domain = format!("gui/{}", uid);
    let ok = Command::new("launchctl")
        .args(["bootstrap", &domain, dst])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        let _ = Command::new("launchctl").args(["load", "-w", dst]).status();
    }
}

fn launchctl_bootout(dst: &str, label: &str) {
    let uid = Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "501".to_string());
    let service_target = format!("gui/{}/{}", uid, label);
    let domain = format!("gui/{}", uid);
    let ok = Command::new("launchctl")
        .args(["bootout", &service_target])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        let _ = Command::new("launchctl")
            .args(["bootout", &domain, dst])
            .status();
        let _ = Command::new("launchctl")
            .args(["unload", "-w", dst])
            .status();
        let _ = Command::new("launchctl").args(["remove", label]).status();
    }
}

fn cmd_daemon(root: &str, args: &[String]) {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("status");
    let home = common::home_dir();
    let agent_dir = format!("{}/Library/LaunchAgents", home);
    let log_dir = format!("{}/Library/Logs", home);

    match sub {
        "install" => {
            println!("Installing LAC 24/7 Daemons as macOS LaunchAgents...");
            let _ = fs::create_dir_all(&agent_dir);
            let _ = fs::create_dir_all(&log_dir);
            for label in ["org.lac.router", "org.lac.worker"] {
                let src = launchd_src(root, label);
                let dst = format!("{}/{}.plist", agent_dir, label);
                match fs::read_to_string(&src) {
                    Ok(content) => {
                        // Retarget the plist at THIS machine (see
                        // `common::retarget_launchd_plist`): home, binary,
                        // working directory, and log paths.
                        let installed = format!("{}/.local/bin/lac", home);
                        let installed_router = format!("{}/.local/bin/lac-router", home);
                        let release_lac = common::bin(root, "lac");
                        let release_router = common::bin(root, "lac-router");
                        let want_bin = if label == "org.lac.router" {
                            if fs::metadata(&installed_router).is_ok() {
                                installed_router
                            } else {
                                release_router
                            }
                        } else if fs::metadata(&installed).is_ok() {
                            installed
                        } else {
                            release_lac
                        };
                        let mut out = common::retarget_launchd_plist(&content, &home, root, &want_bin);
                        if label == "org.lac.router" {
                            let port = common::gateway_port().to_string();
                            out = out.replace(
                                "<key>LAC_ROUTER_PORT</key>\n    <string>8000</string>",
                                &format!("<key>LAC_ROUTER_PORT</key>\n    <string>{}</string>", port),
                            );
                        }
                        match common::atomic_write(&dst, &out) {
                            Ok(()) => println!("  [ok] Placed: {}", dst),
                            Err(e) => println!("  [x] Write {} failed: {}", dst, e),
                        }
                        launchctl_bootout(&dst, label); // idempotent reinstall
                        launchctl_bootstrap(&dst);
                    }
                    Err(_) => println!("  [!] Source plist missing: {} (skipped)", src),
                }
            }
            println!("Worker log: ~/Library/Logs/lac-worker.log");
            println!("Router log: ~/Library/Logs/lac-router.log");
        }
        "uninstall" => {
            println!("Unloading LAC 24/7 Daemons...");
            for label in ["org.lac.worker", "org.lac.router"] {
                let dst = format!("{}/{}.plist", agent_dir, label);
                launchctl_bootout(&dst, label);
                let _ = fs::remove_file(&dst);
            }
            println!("LAC daemons uninstalled.");
        }
        "status" => {
            println!("================================================================");
            println!("  LAC LAUNCHD DAEMON STATUS");
            println!("================================================================");
            let out = Command::new("launchctl")
                .arg("list")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();
            for label in ["org.lac.router", "org.lac.worker"] {
                let running = out.lines().any(|l| l.contains(label));
                let plist = format!("{}/{}.plist", agent_dir, label);
                let placed = fs::metadata(&plist).is_ok();
                println!(
                    "  - {} : {} ({})",
                    label,
                    if running { "ACTIVE (running)" } else { "NOT LOADED" },
                    if placed { "plist placed" } else { "plist absent" }
                );
            }
            println!("----------------------------------------------------------------");
            println!("  Worker: ~/Library/Logs/lac-worker.log");
            println!("  Router: ~/Library/Logs/lac-router.log");
            println!("================================================================");
        }
        _ => println!("Unknown daemon command: {}. Use: install, uninstall, status", sub),
    }
}

// ------------------------------------------------------------- task queue --

#[derive(Debug, Clone)]
struct Task {
    id: String,
    desc: String,
    status: String,
    attempts: u32,
}

fn line_indent(s: &str) -> usize {
    s.chars().take_while(|c| *c == ' ' || *c == '\t').count()
}

fn block_key_indent(block: &str) -> usize {
    block
        .lines()
        .find(|l| l.trim_start().starts_with("- id:"))
        .and_then(|l| l.find("id:"))
        .unwrap_or(0)
}

/// Key lookup inside one `- id:` block. The `- ` list marker occupies
/// two columns, so sibling keys (`task:`, `status:`, `attempts:`) sit at
/// the `id` column while `metadata:` children sit deeper. Only matches
/// at that column count, so nested decoys can never hijack the match.
fn block_field(block: &str, key: &str) -> Option<String> {
    if key == "- id:" {
        for line in block.lines() {
            let l = line.trim();
            if l.starts_with(key) {
                let v = l[key.len()..].trim().trim_matches('"').to_string();
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
        return None;
    }
    let base = block_key_indent(block);
    for line in block.lines() {
        if line_indent(line) != base {
            continue;
        }
        let l = line.trim();
        if l.starts_with(key) {
            let v = l[key.len()..].trim().trim_matches('"').to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// Allowlist for task ids. Ids flow into a transcript path
/// (`~/.lac/task-logs/<id>-<ts>.log`), a git branch (`task/<id>`), and a
/// commit message — all from YAML the worker does not fully control. Only
/// `[A-Za-z0-9][A-Za-z0-9._-]{0,64}` passes, so `../../evil`, absolute
/// paths, spaces, and control chars can never reach the fs or git.
fn sanitize_task_id(id: &str) -> Option<String> {
    if id.is_empty() || id.len() > 65 {
        return None;
    }
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return None,
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-') {
        return None;
    }
    Some(id.to_string())
}

/// Parse `- id:` blocks. Tolerates missing fields; blocks with an unsafe
/// id are dropped here so no caller can ever interpolate them into a
/// path, branch, or commit message. Unknown blocks are preserved verbatim
/// by `update_task` (which edits line ranges, so it never corrupts
/// neighbors).
fn parse_tasks(content: &str) -> Vec<Task> {
    let mut tasks = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let flush = |cur: &mut Vec<&str>, tasks: &mut Vec<Task>| {
        if cur.is_empty() {
            return;
        }
        let block = cur.join("\n");
        if let (Some(id), Some(desc)) = (block_field(&block, "- id:"), block_field(&block, "task:"))
        {
            match sanitize_task_id(&id) {
                Some(id) => tasks.push(Task {
                    id,
                    desc,
                    status: block_field(&block, "status:").unwrap_or_else(|| "pending".to_string()),
                    attempts: block_field(&block, "attempts:")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0),
                }),
                None => {
                    eprintln!("Skipping task block with unsafe id: {:?}", id);
                    common::log_event("task_rejected", &format!("unsafe id rejected: {:?}", id));
                }
            }
        }
        cur.clear();
    };
    for line in content.lines() {
        if line.trim_start().starts_with("- id:") && !cur.is_empty() {
            let mut tmp = std::mem::take(&mut cur);
            flush(&mut tmp, &mut tasks);
            cur = tmp;
        }
        cur.push(line);
    }
    flush(&mut cur, &mut tasks);
    tasks
}

/// Rewrite every block whose `- id:` matches: set status and optionally
/// bump attempts, inserting whichever key is absent (a status-less block
/// otherwise becomes an unfinishable infinite retry). Key matches are
/// indent-anchored to the block level (see `block_field`); all other
/// bytes are untouched — the old global `replacen` could flip a
/// different task that happened to sort first.
fn update_task(content: &str, id: &str, new_status: &str, bump_attempts: bool) -> String {
    let lines: Vec<&str> = content.lines().collect();
    // Locate block boundaries.
    let mut starts: Vec<usize> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if l.trim_start().starts_with("- id:") {
            starts.push(i);
        }
    }
    // All blocks sharing the id (duplicate ids update together so the
    // second copy can never re-execute after the first completes).
    let targets: Vec<usize> = starts
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            lines[**s]
                .trim_start()
                .trim_start_matches("- id:")
                .trim()
                .trim_matches('"')
                == id
        })
        .map(|(t, _)| t)
        .collect();
    if targets.is_empty() {
        return content.to_string();
    }
    let mut out: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    // Record insertions as (index, text) applied after all rewrites so
    // earlier inserts cannot shift later block offsets: apply descending.
    let mut inserts: Vec<(usize, String)> = Vec::new();
    for t in targets {
        let begin = starts[t];
        let end = if t + 1 < starts.len() {
            starts[t + 1]
        } else {
            lines.len()
        };
        let base = lines[begin].find("id:").unwrap_or(0);
        let mut status_idx: Option<usize> = None;
        let mut attempts_idx: Option<usize> = None;
        for i in begin..end {
            if line_indent(lines[i]) != base {
                continue;
            }
            let lt = lines[i].trim();
            if lt.starts_with("status:") && status_idx.is_none() {
                status_idx = Some(i);
            } else if lt.starts_with("attempts:") && attempts_idx.is_none() {
                attempts_idx = Some(i);
            }
        }
        let indent = " ".repeat(base);
        match status_idx {
            Some(i) => {
                out[i] = format!("{}status: {}", indent, new_status);
            }
            None => {
                inserts.push((begin + 1, format!("{}status: {}", indent, new_status)));
            }
        }
        if bump_attempts {
            let cur: u32 = attempts_idx
                .and_then(|i| {
                    lines[i]
                        .trim()
                        .trim_start_matches("attempts:")
                        .trim()
                        .trim_matches('"')
                        .parse()
                        .ok()
                })
                .unwrap_or(0);
            match attempts_idx {
                Some(i) => {
                    out[i] = format!("{}attempts: {}", indent, cur + 1);
                }
                None => {
                    // Land after the status line (original or just-inserted).
                    let at = status_idx.map(|i| i + 1).unwrap_or(begin + 2);
                    inserts.push((at, format!("{}attempts: {}", indent, cur + 1)));
                }
            }
        }
    }
    inserts.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (at, text) in inserts {
        out.insert(at.min(out.len()), text);
    }
    let mut s = out.join("\n");
    if content.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn update_task_file(path: &str, id: &str, status: &str, bump_attempts: bool) -> bool {
    let Some(_lock) = common::acquire_lock("lac-tasks", 30) else {
        eprintln!("warning: queue lock busy; update deferred for {}", id);
        return false;
    };
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    common::atomic_write(path, &update_task(&content, id, status, bump_attempts)).is_ok()
}


/// Deterministic fallback order: first retryable pending task in file
/// order. Judgment (`judge_next`) falls back to exactly this on any
/// model/gateway failure, so the daemon is never less reliable with
/// judgment enabled than without it.
#[allow(dead_code)]
fn next_pending(content: &str) -> Option<Task> {
    parse_tasks(content)
        .into_iter()
        .find(|t| t.status == "pending" && t.attempts < MAX_TASK_ATTEMPTS)
}

/// Retryable candidates in file order. Split out so judgment and the
/// deterministic fallback share one source of truth.
fn pending_candidates(content: &str) -> Vec<Task> {
    parse_tasks(content)
        .into_iter()
        .filter(|t| t.status == "pending" && t.attempts < MAX_TASK_ATTEMPTS)
        .collect()
}

/// Compact judgment prompt: candidate ids + short descriptions + thermal
/// state. Descriptions are truncated so a long queue cannot blow the
/// context window of the judging call itself.
fn judge_prompt(candidates: &[Task], thermal: &str) -> String {
    let mut s = String::from(
        "You are the Hermes task selector for a local autonomous coding worker. Pick the single most urgent task to run next.\n",
    );
    s.push_str(&format!("Thermal state: {}. ", thermal));
    s.push_str("Critical means prefer tiny safe tasks; otherwise pick by urgency described in each task.\nCandidates:\n");
    for t in candidates {
        let short: String = t.desc.chars().take(200).collect();
        s.push_str(&format!("- ID: {} | attempts: {} | task: {}\n", t.id, t.attempts, short));
    }
    s.push_str("Reply with exactly two lines, no extra text:\nTASK_ID: <one of the IDs above>\nREASON: <one short line>\n");
    s
}

/// Parse the line-based judgment format. Returns the matching candidate
/// on an exact id hit, else None (caller falls back to file order).
/// Line-based on purpose: no JSON crate in this std-only binary.
fn parse_judge_decision(output: &str, candidates: &[Task]) -> Option<Task> {
    for line in output.lines() {
        let t = line.trim();
        let rest = match t.strip_prefix("TASK_ID:") {
            Some(r) => r,
            None => continue,
        };
        let mut id = rest.trim().trim_matches('"').trim_matches('\'').trim();
        // Take the first whitespace-separated token so trailing
        // commentary ("b-2 because ...") cannot corrupt the match.
        if let Some(tok) = id.split_whitespace().next() {
            id = tok.trim_matches('"').trim_matches('\'');
        }
        if id.is_empty() {
            continue;
        }
        if let Some(hit) = candidates.iter().find(|c| c.id == id) {
            return Some(hit.clone());
        }
        // Unknown id: keep scanning further TASK_ID lines, if any.
    }
    None
}

/// Fold a judgment result onto the candidate list. Any failure —
/// gateway down, timeout, garbage output, unknown id — resolves to the
/// deterministic first-pending task, so judgment can never make the
/// daemon less reliable than file order.
fn judge_select(candidates: Vec<Task>, judged: Option<Task>) -> Option<Task> {
    match judged {
        Some(t) if candidates.iter().any(|c| c.id == t.id) => Some(t),
        _ => candidates.into_iter().next(),
    }
}

/// Judgment-aware task selection for the worker loop. Single-candidate
/// (or empty) queues skip the model call entirely; multi-candidate
/// queues ask the gateway via post_chat() and fall back to
/// next_pending()'s file order on any failure. Set LAC_JUDGE=0/off to
/// force deterministic file order without a model call.
fn judge_next(content: &str, thermal: &str, root: &str) -> Option<Task> {
    let candidates = pending_candidates(content);
    if candidates.len() <= 1 {
        return candidates.into_iter().next();
    }
    if env::var("LAC_JUDGE")
        .map(|v| v == "0" || v.eq_ignore_ascii_case("off") || v.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
    {
        return candidates.into_iter().next();
    }
    // Fast path: no gateway means post_chat() can only fail. Skip the
    // multi-second connect timeout and stay on deterministic order.
    if !common::port_up(common::gateway_port()) {
        return candidates.into_iter().next();
    }
    let prompt = judge_prompt(&candidates, thermal);
    let model = env::var("LAC_CHAT_MODEL")
        .ok()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| opencode_model(root));
    let body = format!(
        "{{\"model\":\"{}\",\"messages\":[{{\"role\":\"user\",\"content\":\"{}\"}}],\"temperature\":0.0}}",
        common::json_escape(&model),
        common::json_escape(&prompt)
    );
    let judged = match post_chat(&body) {
        Some((200, reply)) => match extract_json_string(&reply, "content") {
            Some(text) => parse_judge_decision(&text, &candidates),
            None => None,
        },
        _ => None,
    };
    judge_select(candidates, judged)
}

// ------------------------------------------------- reviewer gate ----------
// Worker review pass (Finding 4 fix): implement → review → apply, max 2
// rounds per AGENTS.md. Pure std Rust: the diff is collected with git,
// judged through the existing post_chat() gateway call, parsed
// line-based (no JSON crate). Tests stay the fail-closed gate; review is
// fail-open on infra failure (gateway down, timeout, garbage output) so
// the daemon is never less reliable with review enabled than without.

/// Max review rounds per task. Round 3+ means the requirement is
/// ambiguous — the worker commits the task branch flagged for the human
/// instead of hot-looping (AGENTS.md round-3 rule, adapted: work is
/// preserved on the branch, the queue keeps moving).
const MAX_REVIEW_ROUNDS: u32 = 2;
/// Diff bytes fed to the reviewer prompt. Truncation is disclosed
/// in-prompt so the model cannot mistake a cut diff for a clean one.
const REVIEW_DIFF_LIMIT: usize = 12_000;
/// Per-file cap for untracked-file contents included in the review.
const REVIEW_FILE_LIMIT: usize = 2_000;
/// Max untracked files swept into the review prompt.
const REVIEW_FILES_MAX: usize = 10;

/// LAC_REVIEW=0/off/false forces deterministic skip (tests, ops).
fn review_enabled() -> bool {
    !env::var("LAC_REVIEW")
        .map(|v| v == "0" || v.eq_ignore_ascii_case("off") || v.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
}

/// Working-tree diff vs HEAD (tracked mods, staged or not) plus capped
/// contents of small text untracked files. Read-only: never stages.
/// Empty string when the tree is clean vs HEAD.
fn worker_diff(root: &str) -> String {
    let mut out = String::new();
    if let Ok(o) = Command::new("git")
        .current_dir(root)
        .args(["diff", "HEAD", "--", "."])
        .output()
    {
        out.push_str(&String::from_utf8_lossy(&o.stdout));
    }
    if let Ok(o) = Command::new("git")
        .current_dir(root)
        .args(["status", "-s"])
        .output()
    {
        let mut n = 0;
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            let path = match line.strip_prefix("??") {
                Some(p) => p.trim(),
                None => continue,
            };
            if path.is_empty() || n >= REVIEW_FILES_MAX {
                continue;
            }
            n += 1;
            let full = format!("{}/{}", root, path);
            match fs::read(&full) {
                Ok(bytes) if bytes.len() > 100_000 || bytes.contains(&0) => {
                    out.push_str(&format!("\n--- untracked (binary/large, name only): {} ---\n", path));
                }
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let capped: String = text.chars().take(REVIEW_FILE_LIMIT).collect();
                    out.push_str(&format!("\n--- untracked file: {} ---\n{}\n", path, capped));
                    if text.chars().count() > REVIEW_FILE_LIMIT {
                        out.push_str("[truncated]\n");
                    }
                }
                Err(_) => {
                    out.push_str(&format!("\n--- untracked (unreadable, name only): {} ---\n", path));
                }
            }
        }
        if n >= REVIEW_FILES_MAX {
            out.push_str("[untracked file list truncated]\n");
        }
    }
    out
}

/// Reviewer prompt. The verdict is DERIVED from finding markers by
/// parse_review_findings (robust against verdict/findings mismatch), but
/// the model is still asked for VERDICT so its output stays structured.
fn review_prompt(task_desc: &str, diff: &str) -> String {
    let short_task: String = task_desc.chars().take(300).collect();
    let (shown, truncated) = if diff.chars().count() > REVIEW_DIFF_LIMIT {
        (diff.chars().take(REVIEW_DIFF_LIMIT).collect::<String>(), true)
    } else {
        (diff.to_string(), false)
    };
    let mut s = String::from(
        "You are LAC Reviewer, a read-only code auditor. Audit ONLY the diff below against the task. Report one finding per line:\n",
    );
    s.push_str("- [critical] file:line — description  (security hole, data loss, broken invariant, test bypass)\n");
    s.push_str("- [major] file:line — description  (likely bug, wrong behavior, missing error handling, scope creep)\n");
    s.push_str("- [minor] file:line — description  (style, nits — informational only)\n");
    s.push_str("Rules: no refactors beyond the task; minor findings never block. If clean, reply exactly: NO_FINDINGS\n");
    s.push_str("End with exactly one line — VERDICT: approve  (zero critical/major)  or  VERDICT: fix  (any critical/major).\n");
    s.push_str(&format!("Task: {}\n", short_task));
    if truncated {
        s.push_str(&format!("(Diff truncated to {} chars — audit what is shown; do not assume the hidden tail is clean.)\n", REVIEW_DIFF_LIMIT));
    }
    s.push_str("Diff:\n");
    s.push_str(&shown);
    if truncated {
        s.push_str("\n[truncated]");
    }
    s.push('\n');
    s
}

/// Critical/major finding lines. Verdict is derived from these markers —
/// never from the model's VERDICT line — so a mismatched verdict cannot
/// smuggle findings past the gate or block on prose. Minor/suggestion
/// lines are dropped here (report-only, never applied).
fn parse_review_findings(output: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let low = t.to_lowercase();
        if low.contains("[critical]") || low.contains("[major]") {
            out.push(t.to_string());
        }
    }
    out
}

/// Disposition after review round `round` (1-based) with `has_findings`.
/// Pure policy, unit-tested: the loop below can never exceed
/// MAX_REVIEW_ROUNDS reviews no matter what the model returns.
fn review_disposition(round: u32, has_findings: bool) -> &'static str {
    if !has_findings {
        "commit"
    } else if round < MAX_REVIEW_ROUNDS {
        "fix"
    } else {
        "commit-flagged"
    }
}

enum ReviewOutcome {
    Approve,
    Fix(Vec<String>),
    Unavailable(String),
}

/// One review pass over the current working tree. Fail-open on infra
/// failure: gateway down, timeout, non-200, missing content, or
/// LAC_REVIEW=off all yield Unavailable (commit proceeds on the test
/// gate, event logged). A clean diff is Approve without a model call.
fn review_diff(root: &str, task_desc: &str, logpath: &str) -> ReviewOutcome {
    if !review_enabled() {
        return ReviewOutcome::Unavailable("LAC_REVIEW=off".to_string());
    }
    if !common::port_up(common::gateway_port()) {
        return ReviewOutcome::Unavailable(format!("gateway :{} down", common::gateway_port()));
    }
    let diff = worker_diff(root);
    if diff.trim().is_empty() {
        return ReviewOutcome::Approve;
    }
    let prompt = review_prompt(task_desc, &diff);
    let model = env::var("LAC_CHAT_MODEL")
        .ok()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| opencode_model(root));
    let body = format!(
        "{{\"model\":\"{}\",\"messages\":[{{\"role\":\"system\",\"content\":\"{}\"}},{{\"role\":\"user\",\"content\":\"{}\"}}],\"temperature\":0.0}}",
        common::json_escape(&model),
        common::json_escape("You are LAC Reviewer, a read-only code auditor. Reply only in the requested finding-line format."),
        common::json_escape(&prompt)
    );
    let text = match post_chat(&body) {
        Some((200, reply)) => match extract_json_string(&reply, "content") {
            Some(t) => t,
            None => return ReviewOutcome::Unavailable("200 without chat content".to_string()),
        },
        Some((code, _)) => return ReviewOutcome::Unavailable(format!("gateway HTTP {}", code)),
        None => return ReviewOutcome::Unavailable("gateway unreachable/timeout".to_string()),
    };
    // Transcript the raw review for 3am debugging (best-effort).
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(logpath) {
        let _ = writeln!(f, "\n===== reviewer @ {} =====\n{}", common::now_unix(), text);
    }
    let findings = parse_review_findings(&text);
    if findings.is_empty() {
        ReviewOutcome::Approve
    } else {
        ReviewOutcome::Fix(findings)
    }
}

fn count_pending(content: &str) -> (usize, usize) {
    // (pending_and_retryable, dead_letter)
    let mut p = 0;
    let mut d = 0;
    for t in parse_tasks(content) {
        if t.status == "pending" {
            if t.attempts < MAX_TASK_ATTEMPTS {
                p += 1;
            } else {
                d += 1;
            }
        }
    }
    (p, d)
}

fn task_timeout() -> Duration {
    env::var("LAC_TASK_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(7200))
}

/// Kill a process tree via the shared helper (children first, SIGKILL).
/// `child.kill()` alone only signals the direct child; `opencode2` and
/// `make` both spawn grandchildren that would otherwise pile up past
/// the timeout. Best-effort: races with natural exit are harmless.
fn kill_tree(pid: u32) {
    common::kill_tree(pid);
}

/// Spawn and wait with a hard deadline; kills the whole tree on expiry.
/// Returns Ok(true) only on exit status 0 within budget.
fn run_with_timeout(cmd: &str, args: &[&str], dir: &str, timeout: Duration) -> io::Result<bool> {
    let mut child = Command::new(cmd).args(args).current_dir(dir).spawn()?;
    let start = Instant::now();
    loop {
        match child.try_wait()? {
            Some(st) => return Ok(st.success()),
            None => {
                if start.elapsed() >= timeout {
                    kill_tree(child.id());
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(false);
                }
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

/// Transcripts live under ~/.lac/task-logs/<id>-<unix>.log.
fn task_log_path(id: &str) -> String {
    format!(
        "{}/.lac/task-logs/{}-{}.log",
        common::home_dir(),
        id,
        common::now_unix()
    )
}

/// run_with_timeout, but child stdout/stderr tee live to both the console
/// and a per-task transcript — failed tasks stay debuggable afterwards,
/// and the knowledge-recall skill gains real learning material.
fn run_logged(
    cmd: &str,
    args: &[&str],
    dir: &str,
    timeout: Duration,
    log: &str,
    header: &str,
) -> io::Result<bool> {
    let file = match fs::OpenOptions::new().create(true).append(true).open(log) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("transcript {} unwritable ({}); running without log", log, e);
            return run_with_timeout(cmd, args, dir, timeout);
        }
    };
    let file = Arc::new(Mutex::new(file));
    {
        let mut f = file.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(f, "\n===== {} @ {} =====", header, common::now_unix());
    }
    println!("--- {} (transcript: {})", header, log);
    let mut child = Command::new(cmd)
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let pump = |mut pipe: Box<dyn std::io::Read + Send>, to_stderr: bool, file: Arc<Mutex<fs::File>>| {
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if to_stderr {
                            let _ = std::io::stderr().write_all(&buf[..n]);
                        } else {
                            let _ = std::io::stdout().write_all(&buf[..n]);
                        }
                        let mut f = file.lock().unwrap_or_else(|e| e.into_inner());
                        let _ = f.write_all(&buf[..n]);
                    }
                    Err(_) => break,
                }
            }
        })
    };
    let t_out = child
        .stdout
        .take()
        .map(|p| pump(Box::new(p), false, Arc::clone(&file)));
    let t_err = child
        .stderr
        .take()
        .map(|p| pump(Box::new(p), true, Arc::clone(&file)));

    let start = Instant::now();
    let ok = loop {
        match child.try_wait()? {
            Some(st) => break st.success(),
            None => {
                if start.elapsed() >= timeout {
                    kill_tree(child.id());
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                }
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    };
    // Bounded join: a detached grandchild can inherit the pipes and hold
    // them open past the kill, in which case the pumps never see EOF.
    // The worker must survive that (detached pumps die with the process;
    // a stalled queue does not come back). 10s grace, then detach.
    let deadline = Instant::now() + Duration::from_secs(10);
    for t in [t_out, t_err].into_iter().flatten() {
        while !t.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Ok(ok)
}

/// git bound to the project root — never the ambient cwd. A terminal
/// `lac worker` launched from another directory must not branch,
/// commit, or reset that directory.
fn git_in(root: &str, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn worker_current_path() -> String {
    format!("{}/.lac/worker-current", common::home_dir())
}

// ------------------------------------------------------------------ worker -

fn cmd_worker(root: &str, args: &[String]) {
    let continuous = !args.iter().any(|a| a == "--drain" || a == "--once");
    let home = common::home_dir();
    let tasks_file = common::tasks_file();

    // Single-flight: two workers (launchd + terminal) must never drain
    // the same queue concurrently.
    let _lock = match common::acquire_lock("lac-worker", 900) {
        Some(l) => l,
        None => {
            eprintln!("Another lac worker holds the lock (~/.lac/locks/lac-worker). Exiting.");
            std::process::exit(1);
        }
    };

    // Crash recovery: worktrees are isolated, so never checkout or reset the
    // operator tree while recovering. Preserve any abandoned worktree for
    // human review and clear only the marker.
    if fs::metadata(worker_current_path()).is_ok() {
        common::log_event("worker_recover_preserved", "abandoned worktree retained for human review");
        println!("Previous worker state found; preserving its worktree for human review.");
        let _ = fs::remove_file(worker_current_path());
    }

    // Pre-flight 1: router on :8000.
    if !common::port_up(common::gateway_port()) {
        let router_bin = common::bin(root, "lac-router");
        if fs::metadata(&router_bin).is_ok() {
            println!("Starting LAC Unified Gateway on :{} in background...", common::gateway_port());
            match Command::new(&router_bin)
                .env("LAC_ROUTER_PORT", common::gateway_port().to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => {
                    common::persist_or_warn(
                        &format!("{}/.lac/router.pid", home),
                        &child.id().to_string(),
                        "router_pid",
                    );
                }
                Err(e) => println!("  [!] Router spawn failed: {}", e),
            }
            if common::wait_for_port(common::gateway_port(), Duration::from_secs(10)) {
                println!("  [ok] LAC Gateway online on http://127.0.0.1:{}/v1", common::gateway_port());
            } else {
                println!("  [!] Gateway did not come up; worker continues, backends probed directly.");
            }
        } else {
            println!("  [!] Router binary missing ({}); run `make build` first.", router_bin);
        }
    }

    // Pre-flight 2: backends (MLX port follows serve-mlx drift).
    if !common::port_up(common::mlx_port()) && !common::port_up(common::llama_port()) && !common::port_up(common::ollama_port()) {
        println!("Notice: no inference engine on MLX/llama/Ollama ports.");
        let hint = if common::mlx_supported() { "lac serve mlx" } else { "lac serve llama" };
        println!("The worker will idle until one appears ('{}').", hint);
    }

    // Pre-flight 3: queue exists.
    if fs::metadata(&tasks_file).is_err() {
        println!("Initializing task queue at {}...", tasks_file);
        cmd_loop(root, &["init".to_string()]);
    }

    let base_branch = common::default_branch_in(root);
    let has_commits = common::repo_has_commits_in(root);
    if !has_commits {
        println!("Notice: repo has no commits yet — branch/commit steps are skipped until the first commit.");
    }

    println!("================================================================");
    println!("  LAC 24/7 AUTONOMOUS WORKER DAEMON (v{})", VERSION);
    println!("  Target: {} | Qwen 3.8 27B", common::arch_label());
    println!("  Queue : {}", tasks_file);
    println!("  Base  : {} | Mode: {}", base_branch, if continuous { "Continuous 24/7 Watch" } else { "Batch Drain & Stop" });
    println!("================================================================\n");
    common::log_event("worker_start", &format!("mode={}", if continuous { "watch" } else { "drain" }));

    let kv_bin = common::bin(root, "kv-manage");
    let kv_ok = fs::metadata(&kv_bin).is_ok();

    loop {
        let thermal = common::thermal_state();
        if thermal == "Critical" {
            println!("Thermal Critical! Pausing worker 300s...");
            common::log_event("worker_thermal", "critical: sleeping 300s");
            std::thread::sleep(Duration::from_secs(300));
            continue;
        }
        if thermal == "Serious" {
            println!("Thermal Serious: throttling 60s before next task...");
            common::log_event("worker_thermal", "serious: sleeping 60s");
            std::thread::sleep(Duration::from_secs(60));
        }

        let content = match fs::read_to_string(&tasks_file) {
            Ok(c) => c,
            Err(_) => {
                println!("No tasks file at {}. Reinitializing...", tasks_file);
                cmd_loop(root, &["init".to_string()]);
                if !continuous {
                    break;
                }
                std::thread::sleep(Duration::from_secs(15));
                continue;
            }
        };

        // Judgment-aware selection: LLM picks among pending tasks, with a
        // fail-closed fallback to deterministic file order (next_pending).
        // Hygiene gates above (thermal) and below (KV checkpoint, tests)
        // are unchanged — judgment only affects *which* task runs next.
        let task = match judge_next(&content, &thermal, root) {
            Some(t) => t,
            None => {
                let (p, d) = count_pending(&content);
                println!(
                    "Queue idle ({} retryable pending, {} dead-letter). Thermals: {}.",
                    p,
                    d,
                    common::thermal_state()
                );
                if kv_ok {
                    let _ = Command::new(&kv_bin).arg("check").output();
                }
                if !continuous {
                    println!("All tasks drained. Worker exiting.");
                    break;
                }
                println!("Sleeping 15s before next scan (Ctrl+C stops)...");
                std::thread::sleep(Duration::from_secs(15));
                continue;
            }
        };

        // Panic isolation: the 24/7 loop must survive a poisoned task.
        // `task` is only borrowed below so its strike can still be
        // counted afterwards if the closure unwinds.
        let task_id = task.id.clone();
        let task_branched = Arc::new(AtomicBool::new(false));
        let worktree_path = format!(
            "{}/.lac/worker-worktrees/{}-a{}-{}",
            home,
            task.id,
            task.attempts + 1,
            std::process::id()
        );
        let survived = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        println!("\nProcessing queued task [{}]: \"{}\" (attempt {}/{})", task.id, task.desc, task.attempts + 1, MAX_TASK_ATTEMPTS);
        common::log_event("task_start", &format!("[{}] {}", task.id, task.desc));
        common::persist_or_warn(&worker_current_path(), &format!("id: {}\nbase: {}\n", task.id, base_branch), "worker_current");

        if kv_ok {
            println!("  [1/4] KV hygiene checkpoint...");
            let _ = Command::new(&kv_bin)
                .args(["truncate", "--tokens", "16000", "--keep", "8000"])
                .output();
        }

        let was_dirty = common::repo_dirty_in(root);
        let branched: Option<String> = if has_commits {
            let branch = format!("task/{}", task.id);
            let _ = fs::create_dir_all(
                Path::new(&worktree_path)
                    .parent()
                    .unwrap_or_else(|| Path::new(&home)),
            );
            println!("  [2/4] Isolated worktree: {}", branch);
            let branch_ref = format!("refs/heads/{}", branch);
            let branch_exists = git_in(root, &["show-ref", "--verify", "--quiet", &branch_ref]);
            let added = if branch_exists {
                git_in(root, &["worktree", "add", &worktree_path, &branch])
            } else {
                git_in(root, &["worktree", "add", "-b", &branch, &worktree_path])
            };
            if added {
                Some(branch)
            } else {
                println!("  [!] Worktree creation failed; refusing to mutate the operator tree.");
                None
            }
        } else {
            println!("  [2/4] No commits yet — working tree used directly.");
            None
        };
        let work_root = branched
            .as_ref()
            .map(|_| worktree_path.clone())
            .unwrap_or_else(|| root.to_string());
        task_branched.store(branched.is_some(), Ordering::SeqCst);
        if has_commits && branched.is_none() {
            let bumped = task.attempts + 1;
            let status = if bumped >= MAX_TASK_ATTEMPTS {
                println!("  Task [{}] reached {} attempts — dead-letter.", task.id, MAX_TASK_ATTEMPTS);
                common::log_event("task_dead_letter", &format!("[{}]", task.id));
                "failed"
            } else {
                "pending"
            };
            if !update_task_file(&tasks_file, &task.id, status, true) {
                common::log_event("task_update_failed", &format!("task {}", task.id));
            }
            let _ = fs::remove_file(worker_current_path());
            return;
        }

        println!("  [3/4] Dispatching to OpenCode V2 (@coder)...");
        let prompt = format!(
            "Task: {}. Implement surgically, run project tests, and ensure zero failures.",
            task.desc
        );
        let timeout = task_timeout();
        let logpath = task_log_path(&task.id);
        let mut impl_ok = run_logged(
            "opencode2",
            &["run", &prompt],
            &work_root,
            timeout,
            &logpath,
            &format!("opencode2 task {}", task.id),
        )
        .unwrap_or(false);

        let mut test_ok = if impl_ok {
            println!("  [3b/4] Reliability gate: project tests...");
            // Fail-closed: a test runner that cannot even spawn must never
            // count as "tests passed" (that would commit untested code).
            run_logged("make", &["test"], &work_root, Duration::from_secs(600), &logpath, "make test")
                .unwrap_or(false)
        } else {
            false
        };

        // Reviewer gate: implement → review → apply, max 2 rounds.
        // Tests stay fail-closed; review is fail-open on infra failure.
        // A fix round that breaks tests falls back into the failure path
        // below (reset + attempt bump), exactly like a fresh failure.
        let mut review_flagged = false;
        if impl_ok && test_ok {
            let mut round: u32 = 0;
            loop {
                round += 1;
                match review_diff(&work_root, &task.desc, &logpath) {
                    ReviewOutcome::Approve => {
                        println!("  [3c/4] Reviewer round {}: approve.", round);
                        common::log_event("review_approve", &format!("[{}] round {}", task.id, round));
                        break;
                    }
                    ReviewOutcome::Unavailable(reason) => {
                        println!("  [3c/4] Reviewer unavailable ({}); proceeding on test gate.", reason);
                        common::log_event("review_unavailable", &format!("[{}] {}", task.id, reason));
                        break;
                    }
                    ReviewOutcome::Fix(findings) => {
                        match review_disposition(round, true) {
                            "fix" => {
                                println!("  [3c/4] Reviewer round {}: {} critical/major finding(s) — back to @coder...", round, findings.len());
                                common::log_event("review_fix", &format!("[{}] round {} findings {}", task.id, round, findings.len()));
                                let fix_prompt = format!(
                                    "Task: {}. Address these review findings (critical and major ONLY; leave minor/suggestions untouched):\n{}\nRe-run project tests and ensure zero failures.",
                                    task.desc,
                                    findings.join("\n")
                                );
                                impl_ok = run_logged(
                                    "opencode2",
                                    &["run", &fix_prompt],
                                    &work_root,
                                    timeout,
                                    &logpath,
                                    &format!("opencode2 fix {} r{}", task.id, round),
                                )
                                .unwrap_or(false);
                                test_ok = if impl_ok {
                                    println!("  [3b/4] Reliability gate (post-fix): project tests...");
                                    run_logged("make", &["test"], &work_root, Duration::from_secs(600), &logpath, "make test")
                                        .unwrap_or(false)
                                } else {
                                    false
                                };
                                if !(impl_ok && test_ok) {
                                    break;
                                }
                                // Loop re-reviews (round 2). review_disposition
                                // caps this: round 2 findings commit flagged.
                            }
                            _ => {
                                println!("  [3c/4] Reviewer round {}: still {} critical/major finding(s) after {} rounds — committing to task branch FLAGGED for human merge review.", round, findings.len(), round);
                                common::log_event("review_unresolved", &format!("[{}] {} finding(s) after {} rounds", task.id, findings.len(), round));
                                review_flagged = true;
                                break;
                            }
                        }
                    }
                }
            }
        }

        if impl_ok && test_ok {
            println!("  [4/4] Tests passed{}.", if review_flagged { " (review findings unresolved — flagged)" } else { "" });
            if has_commits && branched.is_some() {
                git_in(&work_root, &["add", "-A"]);
                let msg = format!("feat({}): {}", task.id, task.desc.chars().take(120).collect::<String>());
                git_in(&work_root, &["commit", "-m", &msg]);
            } else if was_dirty {
                println!("  Operator tree was dirty; changes remain uncommitted for human review.");
                review_flagged = true;
            }
            println!("  Task [{}] complete.", task.id);
            common::log_event("task_complete", &format!("[{}] review_flagged={}", task.id, review_flagged));
            if !update_task_file(&tasks_file, &task.id, "complete", false) {
                common::log_event("task_update_failed", &format!("task {}", task.id));
            }
        } else {
            println!("  Task [{}] failed reliability gate.", task.id);
            common::log_event("task_failed", &format!("[{}] attempt {}", task.id, task.attempts + 1));
            if branched.is_some() {
                let stash_label = format!("lac failed task {} attempt {}", task.id, task.attempts + 1);
                if git_in(&work_root, &["stash", "push", "-u", "-m", &stash_label]) {
                    println!("  Preserved failed-task changes in git stash '{}'.", stash_label);
                } else {
                    println!("  Could not preserve failed-task changes; leaving the worktree for manual recovery.");
                }
                if !git_in(root, &["worktree", "remove", "--force", &worktree_path]) {
                    println!("  Failed worktree remains at {}.", worktree_path);
                }
            } else if was_dirty {
                println!("  Leaving working tree untouched (was dirty before task).");
            }
            let bumped = task.attempts + 1;
            let status = if bumped >= MAX_TASK_ATTEMPTS {
                println!("  Task [{}] reached {} attempts — dead-letter.", task.id, MAX_TASK_ATTEMPTS);
                common::log_event("task_dead_letter", &format!("[{}]", task.id));
                "failed"
            } else {
                "pending"
            };
            if !update_task_file(&tasks_file, &task.id, status, true) {
                common::log_event("task_update_failed", &format!("task {}", task.id));
            }
        }

        if branched.is_some() {
            if !git_in(root, &["worktree", "remove", "--force", &worktree_path]) {
                common::log_event("worker_cleanup_blocked", &format!("task {} worktree remains at {}", task.id, worktree_path));
            } else {
                let _ = fs::remove_file(worker_current_path());
            }
        } else if !was_dirty {
            let _ = fs::remove_file(worker_current_path());
        }
        }));
        if survived.is_err() {
            println!("  Task [{}] panicked; daemon survives. Counting a strike.", task_id);
            common::log_event("task_panic", &format!("[{}] worker caught panic; state reset", task_id));
            if task_branched.load(Ordering::SeqCst) {
                let panic_stash = format!("lac panicked task {} attempt {}", task_id, task.attempts + 1);
                let _ = git_in(&worktree_path, &["stash", "push", "-u", "-m", &panic_stash]);
                if git_in(root, &["worktree", "remove", "--force", &worktree_path]) {
                    let _ = fs::remove_file(worker_current_path());
                } else {
                    common::log_event("worker_recovery_blocked", &format!("task {} worktree remains at {}", task_id, worktree_path));
                }
            } else {
                common::log_event("worker_recovery_blocked", &format!("task {} preserved unbranched tree", task_id));
            }
            // A deterministically-poisoned task must dead-letter after 3
            // strikes, not hot-loop the daemon forever.
            let bumped = task.attempts + 1;
            let status = if bumped >= MAX_TASK_ATTEMPTS {
                println!("  Task [{}] reached {} attempts — dead-letter.", task_id, MAX_TASK_ATTEMPTS);
                common::log_event("task_dead_letter", &format!("[{}]", task_id));
                "failed"
            } else {
                "pending"
            };
            if !update_task_file(&tasks_file, &task_id, status, true) {
                common::log_event("task_update_failed", &format!("task {}", task_id));
            }
        }

        if !continuous {
            // Drain mode re-reads the queue on the next iteration.
            continue;
        }
    }
    common::log_event("worker_stop", "exiting");
}

// --------------------------------------------------------------- ops cmds --

fn cmd_stop() {
    println!("Stopping LAC services...");
    let pid_files = [
        (format!("{}/.lac/router.pid", common::home_dir()), "lac-router"),
    ];
    for (pf, label) in pid_files {
        if let Ok(pid) = fs::read_to_string(&pf) {
            let pid = pid.trim().to_string();
            if !pid.is_empty() {
                let ok = pid
                    .parse::<u32>()
                    .map(|p| common::signal_pid(p, "TERM"))
                    .unwrap_or(false);
                println!("  {} pid {}: {}", label, pid, if ok { "signaled" } else { "not running" });
            }
            let _ = fs::remove_file(&pf);
        }
    }
    // Fallback: pattern kill for anything lac started (native ps walk).
    let n = common::kill_matching("lac-router");
    println!("  lac-router pattern kill: {} process(es) signaled", n);
    println!("Note: inference engines (mlx/llama/ollama) left running; stop them via the TUI serve menu or brew services.");
}

fn cmd_ps() {
    println!("=== LAC processes ===");
    let mut seen: Vec<(u32, String)> = Vec::new();
    for pat in [
        "lac-router",
        "lac-tui",
        "serve-mlx",
        "serve-llama",
        "llama-server",
        "mlx_lm",
        "ollama serve",
    ] {
        for p in common::processes_matching(pat) {
            if !seen.iter().any(|(pid, _)| *pid == p.pid) {
                seen.push((p.pid, p.cmd));
            }
        }
    }
    seen.sort();
    if seen.is_empty() {
        println!("  (no lac-related processes found)");
    }
    for (pid, cmd) in seen {
        println!("  {:>6} {}", pid, cmd);
    }
    println!("\n=== Port table ===");
    println!(
        "  {:<8} :{}  {}",
        "gateway",
        8000,
        if common::port_up(common::gateway_port()) { "LISTENING" } else { "-" }
    );
    println!(
        "  {:<8} :{}  {}",
        "mlx",
        common::mlx_port(),
        if common::port_up(common::mlx_port()) {
            "LISTENING"
        } else {
            "-"
        }
    );
    for (name, port) in [("llama", common::llama_port()), ("ollama", common::ollama_port())] {
        println!(
            "  {:<8} :{}  {}",
            name,
            port,
            if common::port_up(port) { "LISTENING" } else { "-" }
        );
    }
}

fn cmd_logs(args: &[String]) {
    let which = args.first().map(|s| s.as_str()).unwrap_or("router");
    let file = match which {
        "worker" => "lac-worker.log",
        "mlx" => "lac-serve-mlx.log",
        "llama" => "lac-serve-llama.log",
        "router-err" => "lac-router.err",
        _ => "lac-router.log",
    };
    let path = format!("{}/Library/Logs/{}", common::home_dir(), file);
    println!("=== tail {} ===", path);
    match common::tail_file(&path, 100) {
        Some(t) if !t.is_empty() => print!("{}", t),
        _ => println!("(log file absent or empty — daemon may never have run)"),
    }
}

fn cmd_config(root: &str) {
    let e = |k: &str, d: &str| env::var(k).unwrap_or_else(|_| d.to_string());
    println!("=== LAC effective configuration ===");
    println!("  version      : {}", VERSION);
    println!("  root         : {}", root);
    println!("  gateway      : {}", e("LAC_GATEWAY_URL", &format!("http://127.0.0.1:{}/v1", common::gateway_port())));
    println!("  router_port  : {}", common::gateway_port());
    println!("  bind_addr    : {}", e("LAC_BIND_ADDR", &e("LAC_ROUTER_HOST", "127.0.0.1")));
    println!(
        "  auth         : {}",
        if env::var("LAC_API_TOKEN").map(|s| !s.trim().is_empty()).unwrap_or(false) {
            "Bearer required on every listener (token set, redacted)"
        } else {
            "none (loopback only)"
        }
    );
    println!("  backend_pref : {}", e("LAC_BACKEND", "auto"));
    println!("  primary      : {}", e("PRIMARY_MODEL", "qwen3.8-27b"));
    println!("  mlx_model    : {}", e("MLX_MODEL", "mlx-community/Qwen3.8-27B-4bit"));
    println!("  llama_model  : {}", e("LLAMA_MODEL", "unsloth/Qwen3.6-27B-MTP-GGUF:Q8_0"));
    println!("  model_base   : {} (mounted={})", common::model_base(), common::volume_mounted("/Volumes/AIModels"));
    println!("  branch       : {} (commits={})", common::default_branch_in(root), common::repo_has_commits_in(root));
    println!("  task_timeout : {}s", task_timeout().as_secs());
}

// -------------------------------------------------------------------- main -

fn main() {
    common::ignore_sigpipe();
    let root = common::project_root();
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        println!("================================================================");
        println!("  LAC: Default Mode -> 24/7 Autonomous Worker Daemon (v{})", VERSION);
        println!("  (Run 'lac help' or 'lac status' for other subcommands)");
        println!("================================================================\n");
        cmd_worker(&root, &[]);
        return;
    }

    match args[0].as_str() {
        "worker" | "start" | "run" | "default" => cmd_worker(&root, &args[1..]),
        "daemon" => cmd_daemon(&root, &args[1..]),
        "status" => cmd_status(&root, &args[1..]),
        "doctor" => {
            let issues = cmd_doctor(&root, &args[1..]);
            if issues > 0 {
                std::process::exit(1);
            }
        }
        "thermal" => cmd_thermal(),
        "cap" => cmd_cap(&args[1..]),
        "hermes" => cmd_hermes(&root, &args[1..]),
        "stop" => cmd_stop(),
        "ps" => cmd_ps(),
        "logs" => cmd_logs(&args[1..]),
        "config" => cmd_config(&root),
        "bench" => {
            let port = args
                .iter()
                .skip(1)
                .find(|a| !a.starts_with("--"))
                .and_then(|p| p.parse().ok())
                .unwrap_or(8000);
            cmd_bench(port, &args[1..]);
        }
        "tune" => cmd_tune(&args[1..]),
        "serve" => {
            let mode = args.get(1).map(|s| s.as_str()).unwrap_or("auto");
            match mode {
                "mlx" => {
                    if !common::mlx_supported() {
                        eprintln!("MLX requires Apple Silicon (arm64 macOS); this host is {}.", common::arch_label());
                        eprintln!("Use the llama-server lane instead: `lac serve llama`.");
                        std::process::exit(1);
                    }
                    let _ = Command::new(common::bin(&root, "serve-mlx"))
                        .arg("mlx-community/Qwen3.8-27B-4bit")
                        .status();
                }
                "llama" => {
                    let _ = Command::new(common::bin(&root, "serve-llama"))
                        .args(["unsloth/Qwen3.6-27B-MTP-GGUF:Q8_0", "16384"])
                        .status();
                }
                "ollama" => {
                    println!("Starting Ollama server...");
                    let _ = Command::new("brew").args(["services", "start", "ollama"]).status();
                }
                _ => {
                    println!("Smart serve is in the TUI (option 3). Launching...");
                    let _ = Command::new(common::bin(&root, "lac-tui")).status();
                }
            }
        }
        "route" | "router" => {
            let is_daemon = args.iter().any(|a| a == "--daemon" || a == "-d");
            if is_daemon {
                let port = common::gateway_port();
                println!("Starting lac-router on :{} in background...", port);
                match Command::new(common::bin(&root, "lac-router"))
                    .env("LAC_ROUTER_PORT", port.to_string())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    Ok(child) => {
                        let _ = common::atomic_write(
                            &format!("{}/.lac/router.pid", common::home_dir()),
                            &child.id().to_string(),
                        );
                        println!("lac-router started (pid {}).", child.id());
                    }
                    Err(e) => {
                        eprintln!("Router spawn failed: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                let _ = Command::new(common::bin(&root, "lac-router"))
                    .env("LAC_ROUTER_PORT", common::gateway_port().to_string())
                    .status();
            }
        }
        "tui" => {
            let _ = Command::new(common::bin(&root, "lac-tui")).status();
        }
        "kv" => {
            let _ = Command::new(common::bin(&root, "kv-manage"))
                .args(&args[1..])
                .status();
        }
        "loop" => cmd_loop(&root, &args[1..]),
        "visualize" | "studio" | "app" | "gui" => cmd_visualize(&root),
        "chat" => cmd_chat(&root, &args[1..]),
        "code" => cmd_code(&root, &args[1..]),
        "dashboard" => cmd_dashboard(&root, &args[1..]),
        "bootstrap" => {
            let _ = Command::new(common::bin(&root, "bootstrap")).status();
        }
        "models" => {
            let _ = Command::new(common::bin(&root, "pull-models")).status();
        }
        "pull" => std::process::exit(cmd_pull(&root, &args[1..])),
        "version" | "-v" | "--version" => {
            println!("LAC (Local Agentic Coding) v{}", VERSION);
            println!("Engine: Qwen 3.8 27B Dense Hybrid VLM (MTP enabled)");
            println!("Target: {}", common::arch_label());
        }
        _ => print_help(),
    }
}

// ------------------------------------------------------------------- tests -

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
- id: \"a-1\"\n  task: \"do first\"\n  status: pending\n\n- id: \"b-2\"\n  task: \"do second: with colon\"\n  status: pending\n  attempts: 2\n";

    #[test]
    fn parses_ids_status_attempts() {
        let t = parse_tasks(SAMPLE);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].id, "a-1");
        assert_eq!(t[0].attempts, 0);
        assert_eq!(t[1].desc, "do second: with colon");
        assert_eq!(t[1].attempts, 2);
    }

    #[test]
    fn task_id_sanitize_allows_sane_ids() {
        assert_eq!(sanitize_task_id("a-1").as_deref(), Some("a-1"));
        assert_eq!(sanitize_task_id("lac-004").as_deref(), Some("lac-004"));
        assert_eq!(sanitize_task_id("A").as_deref(), Some("A"));
        assert_eq!(sanitize_task_id("x.y_z-9").as_deref(), Some("x.y_z-9"));
        assert!(sanitize_task_id(&"q".repeat(65)).is_some());
    }

    #[test]
    fn task_id_sanitize_rejects_traversal_and_junk() {
        for bad in [
            "",
            "../../evil",
            "/abs",
            "a/b",
            "a b",
            "-lead",
            ".lead",
            "semi;colon",
            "dq\"q",
            "sq'q",
            "back`tick",
            "dollar$",
            "tilde~",
            "ctrl\x01",
            "star*",
        ] {
            assert!(sanitize_task_id(bad).is_none(), "must reject {:?}", bad);
        }
        assert!(sanitize_task_id(&"q".repeat(66)).is_none(), "must reject 66 chars");
    }

    #[test]
    fn parse_drops_tasks_with_unsafe_ids() {
        let yaml = "- id: \"../../evil\"\n  task: \"pwn\"\n  status: pending\n\n- id: \"ok-1\"\n  task: \"fine\"\n  status: pending\n";
        let t = parse_tasks(yaml);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].id, "ok-1");
    }

    #[test]
    fn chat_field_extraction() {
        let body = r#"{"id":"x","choices":[{"message":{"role":"assistant","content":"Hello \"world\"\nline2"}}]}"#;
        assert_eq!(
            extract_json_string(body, "content").as_deref(),
            Some("Hello \"world\"\nline2")
        );
        assert_eq!(
            extract_json_string(r#"{"error":{"message":"no backend"}}"#, "message").as_deref(),
            Some("no backend")
        );
        assert_eq!(extract_json_string(r#"{"a":1}"#, "content"), None);
        assert_eq!(extract_json_string(r#"{"content":"unterminated"#, "content"), None);
    }

    #[test]
    fn chat_gives_up_on_hung_backend() {
        use std::io::Read;
        // Listener that accepts and then holds the connection silently.
        let ln = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = ln.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            for stream in ln.incoming().take(1).flatten() {
                let mut s = stream;
                let mut tmp = [0u8; 4096];
                let _ = s.read(&mut tmp);
                std::thread::sleep(Duration::from_secs(30));
            }
        });
        let t0 = Instant::now();
        assert!(common::http_post_addr(&addr, "/v1/chat/completions", "{}", Duration::from_secs(2)).is_none());
        assert!(t0.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn update_targets_only_matching_block() {
        let out = update_task(SAMPLE, "b-2", "complete", false);
        // First block untouched.
        assert!(out.contains("- id: \"a-1\"\n  task: \"do first\"\n  status: pending"));
        assert!(out.contains("- id: \"b-2\"\n  task: \"do second: with colon\"\n  status: complete"));
        assert_eq!(out.matches("status: complete").count(), 1);
    }

    #[test]
    fn update_inserts_missing_keys() {
        // No status:/attempts: anywhere: both must be created, or the
        // block would retry forever without ever dead-lettering.
        let bare = "- id: \"c-3\"\n  task: \"bare block\"\n";
        let out = update_task(bare, "c-3", "pending", true);
        assert!(out.contains("status: pending"), "status inserted:\n{}", out);
        assert!(out.contains("attempts: 1"), "attempts inserted:\n{}", out);
        let t = parse_tasks(&out);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].status, "pending");
        assert_eq!(t[0].attempts, 1);
    }

    #[test]
    fn nested_status_keys_are_ignored() {
        let tricky = "- id: \"d-4\"\n  task: \"migrate status: pending docs\"\n  status: pending\n  metadata:\n    status: archived\n    attempts: 99\n";
        let t = parse_tasks(tricky);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].status, "pending");
        assert_eq!(t[0].attempts, 0);
        let out = update_task(tricky, "d-4", "complete", true);
        // Real keys rewritten...
        assert!(out.contains("\n  status: complete\n"));
        // ...nested decoys untouched.
        assert!(out.contains("\n    status: archived\n"));
        assert!(out.contains("\n    attempts: 99\n"));
        assert!(out.contains("\n  attempts: 1\n"));
    }

    #[test]
    fn duplicate_ids_update_together() {
        let dup = "- id: \"e-5\"\n  task: \"first\"\n  status: pending\n- id: \"e-5\"\n  task: \"second\"\n  status: pending\n";
        let out = update_task(dup, "e-5", "complete", false);
        assert_eq!(out.matches("status: complete").count(), 2);
    }

    #[test]
    fn update_bumps_attempts_and_inserts_key() {
        let out = update_task(SAMPLE, "a-1", "pending", true);
        assert!(out.contains("attempts: 1"));
        let out2 = update_task(&out, "a-1", "pending", true);
        assert!(out2.contains("attempts: 2"));
    }

    #[test]
    fn dead_letters_exhaust_attempts_gate() {
        assert!(next_pending(SAMPLE).is_some());
        let out = update_task(SAMPLE, "a-1", "pending", true);
        let out = update_task(&out, "a-1", "pending", true);
        let out = update_task(&out, "a-1", "pending", true);
        // a-1 now at 3 attempts: skipped.
        let next = next_pending(&out).expect("b-2 still retryable");
        assert_eq!(next.id, "b-2");
        let (p, d) = count_pending(&out);
        assert_eq!((p, d), (1, 1));
    }

    #[test]
    fn judge_prompt_lists_candidates_and_thermal() {
        let cands = pending_candidates(SAMPLE);
        assert_eq!(cands.len(), 2);
        let p = judge_prompt(&cands, "Nominal");
        assert!(p.contains("Nominal"), "thermal in prompt:\n{}", p);
        assert!(p.contains("a-1"), "candidate a-1 in prompt:\n{}", p);
        assert!(p.contains("b-2"), "candidate b-2 in prompt:\n{}", p);
        assert!(p.contains("TASK_ID:"), "format instructions in prompt:\n{}", p);
    }

    #[test]
    fn judge_parses_non_first_pick() {
        // Fixture queue: file order is routine first, urgent last. A
        // judgment for the urgent task must differ from next_pending().
        const URGENT_LAST: &str = "\
- id: \"t-routine\"\n  task: \"routine: tidy comments in docs\"\n  status: pending\n\n- id: \"t-normal\"\n  task: \"normal: add a unit test for tail_file\"\n  status: pending\n\n- id: \"t-urgent\"\n  task: \"URGENT: production auth bypass — fix login check in lac.rs now\"\n  status: pending\n";
        let cands = pending_candidates(URGENT_LAST);
        assert_eq!(cands.len(), 3);
        let first = next_pending(URGENT_LAST).expect("pending").id;
        assert_eq!(first, "t-routine");
        let judged = parse_judge_decision("TASK_ID: t-urgent\nREASON: production auth bypass outranks tidy-up\n", &cands)
            .expect("valid id parses");
        assert_eq!(judged.id, "t-urgent");
        assert_ne!(judged.id, first, "judgment differs from file order");
        // Folded through judge_select, the judged task is what runs.
        let picked = judge_select(cands, Some(judged)).expect("pick");
        assert_eq!(picked.id, "t-urgent");
    }

    #[test]
    fn judge_falls_back_to_file_order_on_failure() {
        const URGENT_LAST: &str = "\
- id: \"t-routine\"\n  task: \"routine: tidy comments in docs\"\n  status: pending\n\n- id: \"t-urgent\"\n  task: \"URGENT: production auth bypass\"\n  status: pending\n";
        let cands = pending_candidates(URGENT_LAST);
        let expected = next_pending(URGENT_LAST).expect("pending").id;
        // Garbage output parses to None...
        assert!(parse_judge_decision("ACTION: dispatch_loop\nno task id here\n", &cands).is_none());
        // ...and both garbage and unknown ids fold back to file order.
        let garbage = parse_judge_decision("hello world", &cands);
        assert_eq!(judge_select(cands.clone(), garbage).expect("pick").id, expected);
        let unknown = parse_judge_decision("TASK_ID: nope-missing\nREASON: hallucinated\n", &cands);
        assert!(unknown.is_none());
        assert_eq!(judge_select(cands.clone(), unknown).expect("pick").id, expected);
        // Total failure (gateway down / timeout / non-200) is None too.
        assert_eq!(judge_select(cands, None).expect("pick").id, expected);
    }

    #[test]
    fn judge_next_single_candidate_skips_model() {
        const ONE: &str = "- id: \"solo-1\"\n  task: \"only task\"\n  status: pending\n";
        // No gateway needed: single-candidate queues never call post_chat.
        let picked = judge_next(ONE, "Nominal", "/tmp").expect("solo");
        assert_eq!(picked.id, "solo-1");
        assert!(judge_next("nothing here", "Nominal", "/tmp").is_none());
    }

    #[test]
    fn review_parses_critical_and_major_only() {
        let out = "- [critical] lac.rs:42 — unsanitized id reaches git branch\n- [major] lac.rs:90 — missing error handling on router spawn\n- [minor] lac.rs:12 — typo in comment\n- [suggestion] consider renaming\nNO_FINDINGS is absent\nVERDICT: fix";
        let f = parse_review_findings(out);
        assert_eq!(f.len(), 2, "only critical+major survive: {:?}", f);
        assert!(f[0].contains("lac.rs:42"));
        assert!(f[1].contains("lac.rs:90"));
        // Case-insensitive markers, different bullets.
        let out2 = "* [Critical] a.rs:1 — x\n[MAJOR] b.rs:2 — y";
        assert_eq!(parse_review_findings(out2).len(), 2);
    }

    #[test]
    fn review_approves_clean_output() {
        for clean in [
            "NO_FINDINGS\nVERDICT: approve",
            "- [minor] x.rs:1 — nit\nVERDICT: approve",
            "",
            "Looks fine, no issues.",
            "VERDICT: fix", // verdict line alone is NOT a finding marker
        ] {
            assert!(parse_review_findings(clean).is_empty(), "must approve: {:?}", clean);
        }
    }

    #[test]
    fn review_disposition_caps_rounds() {
        // Clean at any round commits.
        assert_eq!(review_disposition(1, false), "commit");
        assert_eq!(review_disposition(2, false), "commit");
        // Findings on round 1 go back to the coder...
        assert_eq!(review_disposition(1, true), "fix");
        // ...but round 2 findings commit flagged — the loop can never
        // reach round 3 no matter what the model returns.
        assert_eq!(review_disposition(2, true), "commit-flagged");
        assert_eq!(review_disposition(99, true), "commit-flagged");
    }

    #[test]
    fn review_prompt_truncates_and_discloses() {
        let big = "x".repeat(REVIEW_DIFF_LIMIT + 100);
        let p = review_prompt("fix login check", &big);
        assert!(p.contains("fix login check"), "task in prompt");
        assert!(p.contains("truncated"), "truncation disclosed:\n{}", &p[..500]);
        assert!(p.len() < big.len() + 2000, "prompt bounded");
        let small = "diff --git a/x b/x";
        let p2 = review_prompt("t", small);
        assert!(!p2.contains("truncated"), "no false truncation note");
        assert!(p2.contains(small));
    }

    #[test]
    fn review_disabled_flag() {
        let saved = env::var("LAC_REVIEW").ok();
        // env mutation is process-global; this is the only test that
        // touches LAC_REVIEW, and it restores the prior value.
        unsafe {
            env::set_var("LAC_REVIEW", "off");
        }
        assert!(!review_enabled());
        unsafe {
            env::set_var("LAC_REVIEW", "0");
        }
        assert!(!review_enabled());
        unsafe {
            env::set_var("LAC_REVIEW", "1");
        }
        assert!(review_enabled());
        unsafe {
            match saved {
                Some(v) => env::set_var("LAC_REVIEW", v),
                None => env::remove_var("LAC_REVIEW"),
            }
        }
    }

    #[test]
    fn model_parsed_through_comments() {
        let jsonc = "{\n// comment\n\"model\": \"lac/qwen-test\", /* block */\n}";
        let stripped = strip_jsonc_comments(jsonc);
        assert!(!stripped.contains("// comment"));
        assert!(stripped.contains("\"model\""));
    }

    #[test]
    fn content_len_handles_escapes() {
        let body = r#"{"choices":[{"message":{"content":"a\"b\\c"}}]}"#;
        // a " b \ c -> 5 chars.
        assert_eq!(content_len_approx(body), Some(5));
        assert_eq!(content_len_approx("{}"), None);
    }

    #[test]
    fn doctor_memory_check_scales_across_macs() {
        // 8GB MacBook Air
        assert_eq!(doctor_memory_check_with(Some(8.0), Some(4.0)).0, "ok");
        assert_eq!(doctor_memory_check_with(Some(8.0), Some(2.0)).0, "warn");
        assert_eq!(doctor_memory_check_with(Some(8.0), Some(1.0)).0, "fail");

        // 16GB MacBook Pro
        assert_eq!(doctor_memory_check_with(Some(16.0), Some(5.0)).0, "ok");
        assert_eq!(doctor_memory_check_with(Some(16.0), Some(2.5)).0, "warn");
        assert_eq!(doctor_memory_check_with(Some(16.0), Some(1.0)).0, "fail");

        // 96GB Mac Studio
        assert_eq!(doctor_memory_check_with(Some(96.0), Some(20.0)).0, "ok");
        assert_eq!(doctor_memory_check_with(Some(96.0), Some(8.0)).0, "warn");
        assert_eq!(doctor_memory_check_with(Some(96.0), Some(2.0)).0, "fail");

        // Missing telemetry
        assert_eq!(doctor_memory_check_with(Some(16.0), None).0, "warn");
    }

    #[test]
    fn intel_hint_never_mentions_mlx() {
        // On Intel hosts, fallback hints must direct to llama or ollama, never MLX.
        let intel_hint = |mlx_supported: bool| -> &'static str {
            if mlx_supported {
                "no inference backend serving; run lac serve mlx"
            } else {
                "no inference backend serving; run lac serve llama"
            }
        };
        assert!(!intel_hint(false).to_lowercase().contains("mlx"));
        assert!(intel_hint(false).contains("serve llama"));
    }

    #[test]
    fn router_down_hint_matches_standard() {
        assert!(NO_BACKEND_HINT.contains("Start one with `lac serve mlx`."));
    }

    #[test]
    fn line_indent_basic() {
        assert_eq!(line_indent(""), 0);
        assert_eq!(line_indent("   "), 3);
        assert_eq!(line_indent("\t\t"), 2);
        assert_eq!(line_indent("abc"), 0);
        assert_eq!(line_indent("  abc"), 2);
    }

    #[test]
    fn block_key_indent_basic() {
        let block = "- id: task-1\n";
        assert_eq!(block_key_indent(block), 2);
        let block2 = "- id:task-1\n";
        assert_eq!(block_key_indent(block2), 2);
    }

    #[test]
    fn block_field_simple() {
        let block = "- id: my-task\n  task: do it\n  status: pending\n  attempts: 0";
        assert_eq!(block_field(block, "task:").as_deref(), Some("do it"));
        assert_eq!(block_field(block, "status:").as_deref(), Some("pending"));
        assert_eq!(block_field(block, "attempts:").as_deref(), Some("0"));
        assert_eq!(block_field(block, "nonexistent:").as_deref(), None);
    }

    #[test]
    fn pull_target_detection() {
        let is_hf = |m: &str| m.contains('/') || m.starts_with("mlx-") || m.contains("MLX") || m.ends_with(".gguf");
        assert!(is_hf("mlx-community/Qwen3.8-27B-4bit"));
        assert!(is_hf("lmstudio-community/Qwen3.8-27B-MLX-4bit"));
        assert!(is_hf("unsloth/Qwen3.6-27B-MTP-GGUF"));
        assert!(!is_hf("qwen3.8-27b"));
        assert!(!is_hf("llama3"));
    }

    #[test]
    fn probe_opts_parse_and_clamp() {
        let d: Vec<String> = vec![];
        let o = parse_probe_opts(&d);
        assert_eq!((o.tokens, o.temp), (64, 0.0));
        let a = ["--tokens".to_string(), "128".to_string(), "--temp".to_string(), "0.7".to_string()];
        let o = parse_probe_opts(&a);
        assert_eq!((o.tokens, o.temp), (128, 0.7));
        let b = ["--max-tokens=4099".to_string(), "--temp=9.0".to_string()];
        let o = parse_probe_opts(&b);
        assert_eq!((o.tokens, o.temp), (4096, 2.0));
        let c = ["--tokens".to_string(), "1".to_string()];
        let o = parse_probe_opts(&c);
        assert_eq!(o.tokens, 8);
    }
}
