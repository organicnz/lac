mod common;

use std::fs;
use std::io::{self, Write};
use std::process::Command;

fn clear() {
    print!("\x1B[2J\x1B[1;1H");
    let _ = io::stdout().flush();
}

fn pause() {
    println!("\nPress Enter to continue...");
    let mut buf = String::new();
    let _ = io::stdin().read_line(&mut buf);
}

fn run(cmd: &str, args: &[&str]) -> String {
    match Command::new(cmd).args(args).output() {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).to_string();
            let e = String::from_utf8_lossy(&o.stderr).to_string();
            if !e.is_empty() {
                s.push_str("\n[stderr]\n");
                s.push_str(&e);
            }
            if s.trim().is_empty() {
                format!("(exit: {})", o.status)
            } else {
                s
            }
        }
        Err(e) => format!("failed to run {}: {}", cmd, e),
    }
}

fn port_up(port: u16) -> bool {
    common::port_up(port)
}


fn free_ram_gib() -> Option<f64> {
    common::free_ram_gib()
}

// Project root + sibling binaries resolve through the shared module.
fn project_root() -> String {
    common::project_root()
}

fn bin(root: &str, name: &str) -> String {
    common::bin(root, name)
}

fn status_header(root: &str) -> String {
    let gw = if port_up(8000) { "UP  " } else { "DOWN" };
    let o = if port_up(11434) { "UP  " } else { "DOWN" };
    let mlx = if port_up(common::mlx_port()) { "UP  " } else { "DOWN" };
    let llama = if port_up(8081) { "UP  " } else { "DOWN" };
    let ram = free_ram_gib()
        .map(|f| format!("{:.1}GiB", f))
        .unwrap_or_else(|| "?".to_string());
    let model = fs::read_to_string(format!("{}/opencode.jsonc", root))
        .ok()
        .and_then(|t| {
            t.find("\"model\"").and_then(|i| {
                let rest = &t[i..];
                let q1 = rest.find('"').unwrap_or(0);
                let rest2 = &rest[q1 + 1..];
                let q2 = rest2.find('"').unwrap_or(0);
                let rest3 = &rest2[q2 + 1..];
                let q3 = rest3.find('"').unwrap_or(0);
                let rest4 = &rest3[q3 + 1..];
                rest4.find('"').map(|q4| rest4[..q4].to_string())
            })
        })
        .unwrap_or_else(|| "?".to_string());
    format!(
        "LAC  model:{}  gw:{}  ollama:{}  mlx:{}  llama:{}  free:{}",
        model, gw, o, mlx, llama, ram
    )
}

/// Native model-catalog probe (no curl subprocess): truncated for display.
fn http_models(port: u16) -> String {
    match common::http_get(port, "/v1/models", 3000) {
        Some((200, b)) => {
            let mut s = b;
            if s.len() > 1500 {
                // Byte cuts can split UTF-8: walk back to a char boundary
                // (String::truncate panics otherwise).
                let mut cut = 1500.min(s.len());
                while !s.is_char_boundary(cut) {
                    cut -= 1;
                }
                s.truncate(cut);
                s.push_str("…[truncated]");
            }
            s
        }
        Some((c, _)) => format!("HTTP {}", c),
        None => "OFFLINE".to_string(),
    }
}

fn health_check(root: &str) {
    clear();
    println!("=== LAC Stack Health ===\n");
    println!("-- Ollama :11434 --");
    println!("{}", http_models(11434));
    println!("\n-- MLX lane (port follows serve-mlx drift) --");
    println!("{}", http_models(common::mlx_port()));
    println!("\n-- llama :8081 --");
    println!("{}", http_models(8081));
    println!("\n-- gateway :8000 --");
    match common::http_get(8000, "/lac/status", 3000) {
        Some((200, b)) => println!("{}", b),
        Some((c, _)) => println!("HTTP {}", c),
        None => println!("OFFLINE"),
    }
    println!("\n-- ollama ps --");
    println!("{}", run("ollama", &["ps"]));
    println!("\n-- kv-manage check --");
    println!("{}", run(&bin(root, "kv-manage"), &["check"]));
    println!("\n-- disk /Volumes/AIModels --");
    println!("{}", run("df", &["-h", "/Volumes/AIModels"]));
    println!("\n-- thermals --");
    println!("{}", common::head_lines(&common::thermal_detail(), 20));
    pause();
}

fn serve_menu(root: &str) {
    loop {
        clear();
        println!("=== Serve Menu (qwen3.8-27B) ===\n");
        println!("  ollama :11434  {}", if port_up(11434) { "UP" } else { "DOWN" });
        println!("  mlx :{}  {}", common::mlx_port(), if port_up(common::mlx_port()) { "UP" } else { "DOWN" });
        println!("  llama :8081 {}", if port_up(8081) { "UP" } else { "DOWN" });
        println!();
        println!("  1) Start mlx-lm Q4 (fast, 16.1GB) — blocks, Ctrl+C stops");
        println!("  2) Start llama-server Q8_0 (quality, ~20GB) — blocks");
        println!("  3) Start ollama serve (background)");
        println!("  4) Stop mlx/llama servers (native kill)");
        println!("  5) Install launchd KeepAlive (non-stop survival)");
        println!("  6) Smart Serve         (auto-Q4/Q8 based on thermal/RAM)");
        println!("  7) Switch router backend (mlx/llama/ollama/auto/fastest)");
        println!("  0) Back\n");
        print!("choice> ");
        let _ = io::stdout().flush();
        let mut c = String::new();
        if io::stdin().read_line(&mut c).is_err() {
            return;
        }
        match c.trim() {
            "1" => {
                println!("\nExec: {} mlx-community/Qwen3.8-27B-4bit", bin(root, "serve-mlx"));
                let _ = Command::new(bin(root, "serve-mlx"))
                    .arg("mlx-community/Qwen3.8-27B-4bit")
                    .status();
                pause();
            }
            "2" => {
                println!("\nExec: serve-llama Q8_0 ctx 16384 (Ctrl+C stops)");
                let _ = Command::new(bin(root, "serve-llama"))
                    .args(["unsloth/Qwen3.6-27B-MTP-GGUF:Q8_0", "16384"])
                    .status();
                pause();
            }
            "3" => {
                if port_up(11434) {
                    println!("ollama :11434 already up");
                    pause();
                    continue;
                }
                // Prefer the brew service; fall back to a detached server.
                let started = Command::new("brew")
                    .args(["services", "start", "ollama"])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                if started {
                    println!("brew services: ollama started");
                } else if port_up(11434) {
                    println!("ollama :11434 already up");
                } else {
                    match Command::new("ollama")
                        .arg("serve")
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                    {
                        Ok(c) => println!("ollama serve spawned in background (pid {})", c.id()),
                        Err(e) => println!("could not start ollama: {}", e),
                    }
                }
                pause();
            }
            "4" => {
                let a = common::kill_matching("mlx_lm.server");
                let b = common::kill_matching("llama-server");
                println!("signaled {} mlx + {} llama processes", a, b);
                pause();
            }
            "5" => {
                // Native launchd install (no shell pipeline): place the
                // plist retargeted at this user's home, then bootstrap it
                // into the GUI domain.
                let src = format!("{}/launchd/org.lac.serve-mlx.plist", root);
                let home = common::home_dir();
                let dst = format!("{}/Library/LaunchAgents/org.lac.serve-mlx.plist", home);
                match std::fs::create_dir_all(format!("{}/Library/LaunchAgents", home))
                    .and_then(|_| std::fs::read_to_string(&src))
                    .map(|c| c.replace("/Users/organic", &home))
                    .and_then(|c| std::fs::write(&dst, c))
                {
                    Ok(()) => {
                        let uid = Command::new("id")
                            .arg("-u")
                            .output()
                            .ok()
                            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                            .unwrap_or_else(|| "501".to_string());
                        let ok = Command::new("launchctl")
                            .args(["bootstrap", &format!("gui/{}", uid), &dst])
                            .status()
                            .map(|s| s.success())
                            .unwrap_or(false);
                        println!("{}", if ok { "installed" } else { "plist placed; bootstrap it manually" });
                    }
                    Err(e) => println!("install failed: {}", e),
                }
                pause();
            }
            "6" => smart_serve(root),
            "7" => switch_backend(),
            "0" | "q" => return,
            _ => {}
        }
    }
}

/// Smart Serve Orchestration — automatically selects Q4 vs Q8 based on
/// current thermal state, free RAM, and token usage. This is the "incredibly
/// intelligent" layer that builds on all the predictive/thermal/KV infrastructure.
fn smart_serve(root: &str) {
    clear();
    println!("=== Smart Serve Orchestration ===\n");
    println!("Assessing current conditions...\n");

    // 1. Check thermal state
    let thermal = thermal_state_from_pmset();
    println!("Thermal state: {}", thermal);

    // 2. Check free RAM
    let free = free_ram_gib().unwrap_or(0.0);
    let free_formatted = format!("{:.1} GiB", free);
    println!("Free RAM: {}", free_formatted);

    // 3. Check current token usage
    let current_tokens = session_tokens().unwrap_or(0);
    let tokens_str = if current_tokens > 0 {
        format!("{} tokens (estimated from session)", current_tokens)
    } else {
        "0 tokens (no session data)".to_string()
    };
    println!("Current tokens: {}", tokens_str);

    // 4. Apply decision rules
    let (model_choice, context, decision_reason) = decide_model_and_context(&thermal, free, current_tokens);

    println!();
    println!("Decision: {}", decision_reason);
    println!();
    println!("Selected: {}", model_choice);
    println!("Context window: {} tokens", context);

    // 5. Ask user to confirm before starting
    println!();
    print!("Start this configuration? [y/n]> ");
    let _ = io::stdout().flush();
    let mut confirm = String::new();
    if io::stdin().read_line(&mut confirm).is_err() {
        println!("Cancelled.");
        pause();
        return;
    }
    let confirm = confirm.trim().to_lowercase();
    if !confirm.starts_with('y') && !confirm.starts_with('1') {
        println!("Cancelled.");
        pause();
        return;
    }

    // 6. Start the selected server
    start_served_model(root, &model_choice, context);
    pause();
}

/// Decide which lane to serve: thresholds scaled for the 96GB Studio.
/// Q4 always rides MLX (native MTP, fastest); Q8 always rides
/// llama-server (quality). The old table mixed them up -- it labelled
/// choices "llama Q4" then booted an MLX model into llama-server.
fn decide_model_and_context(thermal: &str, free: f64, current_tokens: u64) -> (String, i32, String) {
    let threshold_tokens = 24000u64;
    let tokens_percent = if threshold_tokens > 0 {
        (current_tokens as f64 / threshold_tokens as f64) * 100.0
    } else {
        0.0
    };

    // Rule 1: heat always wins -- shed quality for survival.
    if thermal == "Critical" {
        return (
            "mlx Q4 (mlx-community/Qwen3.8-27B-4bit)".to_string(),
            4096,
            format!("Thermal Critical -> MLX Q4 ctx=4096 to survive. Free RAM: {:.1} GiB, tokens {}/24000 ({:.1}%)", free, current_tokens, tokens_percent),
        );
    }
    if thermal == "Serious" || thermal == "Fair" {
        return (
            "mlx Q4 (mlx-community/Qwen3.8-27B-4bit)".to_string(),
            8192,
            format!("Thermal {} -> MLX Q4 ctx=8192 for stability. Free RAM: {:.1} GiB.", thermal, free),
        );
    }

    // Rule 2: Nominal -- spend RAM on quality while headroom is vast.
    if free > 40.0 && tokens_percent < 50.0 {
        return (
            "llama-server Q8 (unsloth/Qwen3.6-27B-MTP-GGUF:Q8_0)".to_string(),
            16384,
            format!("Nominal + {:.1} GiB free + tokens {:.1}% -> llama Q8 ctx=16384 for maximum quality.", free, tokens_percent),
        );
    }
    if free >= 16.0 {
        return (
            "mlx Q4 (mlx-community/Qwen3.8-27B-4bit)".to_string(),
            16384,
            format!("Nominal + {:.1} GiB free -> MLX Q4 ctx=16384, balanced speed.", free),
        );
    }
    (
        "mlx Q4 (mlx-community/Qwen3.8-27B-4bit)".to_string(),
        8192,
        format!("Nominal but only {:.1} GiB free -> MLX Q4 ctx=8192 to conserve.", free),
    )
}

/// Start the selected lane. MLX <=> 4-bit MLX weights, llama <=> GGUF:
/// never cross them (llama-server cannot load MLX checkpoints).
fn start_served_model(root: &str, model_choice: &str, context: i32) {
    clear();
    println!("=== Starting Smart Serve ===\n");
    println!("Lane: {}", model_choice);
    println!("Context: {} tokens", context);
    println!();

    let ctx_str = context.to_string();
    if model_choice.starts_with("mlx") {
        println!("Starting mlx-lm (mlx-community/Qwen3.8-27B-4bit)...");
        let _ = Command::new(bin(root, "serve-mlx"))
            .arg("mlx-community/Qwen3.8-27B-4bit")
            .status();
    } else {
        println!("Starting llama-server (Q8_0, ctx {})...", ctx_str);
        let _ = Command::new(bin(root, "serve-llama"))
            .args(["unsloth/Qwen3.6-27B-MTP-GGUF:Q8_0", &ctx_str])
            .status();
    }

    println!();
    println!("Server start initiated. Check the TUI status header for live state.");
    pause();
}

fn thermal_state_from_pmset() -> String {
    common::thermal_state()
}


/// Return the current token usage estimate.
fn session_tokens() -> Option<u64> {
    let out = Command::new("opencode2")
        .args(["api", "get", "/api/health"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let txt = String::from_utf8_lossy(&out.stdout).to_lowercase();
    let mut best: Option<u64> = None;
    let mut search = txt.as_str();
    loop {
        let pos = match (search.find("token"), search.find("context")) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        let p = match pos {
            Some(p) => p,
            None => break,
        };
        let after = &search[p..];
        let mut started: Option<usize> = None;
        let mut end = 0;
        for (i, b) in after.bytes().enumerate() {
            if b.is_ascii_digit() {
                if started.is_none() {
                    started = Some(i);
                }
                end = i + 1;
            } else if started.is_some() {
                break;
            }
        }
        if let Some(s) = started {
            if let Ok(n) = after[s..end].parse::<u64>() {
                if n > 100 && n < 10_000_000 {
                    best = Some(n);
                }
            }
            search = &after[end.min(after.len())..];
        } else if after.len() > 1 {
            search = &after[1..];
        } else {
            break;
        }
        if search.is_empty() {
            break;
        }
    }
    best
}

/// Hot-swap the router's preferred backend via /lac/switch (persisted
/// router-side in ~/.lac/router-backend, survives restarts).
fn switch_backend() {
    clear();
    println!("=== Router Backend Switch ===\n");
    println!("  Current preference is shown at http://127.0.0.1:8000/lac/status\n");
    println!("  1) auto    (MLX -> llama -> Ollama failover)");
    println!("  2) mlx     (Q4 speed lane)");
    println!("  3) llama   (Q8 quality lane)");
    println!("  4) ollama  (fallback)");
    println!("  5) fastest (lowest measured latency)");
    println!("  0) Back\n");
    print!("choice> ");
    let _ = io::stdout().flush();
    let mut c = String::new();
    if io::stdin().read_line(&mut c).is_err() {
        return;
    }
    let target = match c.trim() {
        "1" => "auto",
        "2" => "mlx",
        "3" => "llama",
        "4" => "ollama",
        "5" => "fastest",
        _ => return,
    };
    match common::http_get(8000, &format!("/lac/switch?target={}", target), 5000) {
        Some((200, b)) => println!("{}", b),
        Some((c, _)) => println!("router replied HTTP {}", c),
        None => println!("router :8000 unreachable — start it with lac route --daemon"),
    }
    pause();
}

fn skills_menu() {
    loop {
        clear();
        println!("=== Skills Menu ===\n");
        println!("  1) local-verify        (stack health)");
        println!("  2) model-swap           (Q4<->Q8 toggle)");
        println!("  3) kv-cache-manage      (check / truncate)");
        println!("  4) context-cap          (steady-state cap)");
        println!("  5) agent-resume         (save / resume)");
        println!("  6) thermal-monitor      (temp check)");
        println!("  7) auditor              (code audit)");
        println!("  8) task-runner status   (queue overview)");
        println!("  0) Back\n");
        print!("choice> ");
        let _ = io::stdout().flush();
        let mut c = String::new();
        if io::stdin().read_line(&mut c).is_err() {
            return;
        }
        match c.trim() {
            "1" => {
                println!("{}", run("opencode2", &["run", "Use the local-verify skill and report stack health"]));
                pause();
            }
            "2" => {
                println!("{}", run("opencode2", &["run", "Use the model-swap skill to toggle the model backend"]));
                pause();
            }
            "3" => {
                print!("check or truncate? [c/t]> ");
                let _ = io::stdout().flush();
                let mut m = String::new();
                let _ = io::stdin().read_line(&mut m);
                if m.trim() == "t" {
                    print!("tokens before?> ");
                    let _ = io::stdout().flush();
                    let mut t = String::new();
                    let _ = io::stdin().read_line(&mut t);
                    println!("{}", run("opencode2", &["run", &format!("Use the kv-cache-manage skill to truncate context, keeping the last {} tokens", t.trim())]));
                } else {
                    println!("{}", run("opencode2", &["run", "Use the kv-cache-manage skill to check context usage"]));
                }
                pause();
            }
            "4" => {
                println!("{}", run("opencode2", &["run", "Use the context-cap skill to check current context usage"]));
                pause();
            }
            "5" => {
                print!("save or resume? [s/r]> ");
                let _ = io::stdout().flush();
                let mut m = String::new();
                let _ = io::stdin().read_line(&mut m);
                let prompt = if m.trim() == "r" {
                    "Use the agent-resume skill to resume the saved session state"
                } else {
                    "Use the agent-resume skill to save the current session state"
                };
                println!("{}", run("opencode2", &["run", prompt]));
                pause();
            }
            "6" => {
                println!("{}", run("opencode2", &["run", "Use the thermal-monitor skill to check thermals"]));
                pause();
            }
            "7" => {
                println!("{}", run("opencode2", &["run", "Use the auditor skill to audit the codebase"]));
                pause();
            }
            "8" => {
                println!("{}", run("opencode2", &["run", "Use the task-runner skill to show queue status"]));
                pause();
            }
            "0" | "q" => return,
            _ => {}
        }
    }
}

fn preflight(root: &str) {
    clear();
    println!("=== Pre-flight (long runs) ===\n");
    println!("-- servers --");
    println!("ollama :11434  {}", if port_up(11434) { "UP" } else { "DOWN" });
    println!("mlx :{}  {}", common::mlx_port(), if port_up(common::mlx_port()) { "UP" } else { "DOWN" });
    println!("llama :8081 {}", if port_up(8081) { "UP" } else { "DOWN" });
    println!("\n-- kv-manage check --");
    println!("{}", run(&bin(root, "kv-manage"), &["check"]));
    println!("\n-- thermals --");
    println!("{}", common::head_lines(&common::thermal_detail(), 8));
    println!("\n-- agent state --");
    let home = common::home_dir();
    let mut states = Vec::new();
    for p in [
        "/Volumes/AIModels/hf/agent-state.json".to_string(),
        format!("{}/.lac/agent-state.json", home),
    ] {
        match std::fs::metadata(&p) {
            Ok(m) => states.push(format!("{} ({} bytes)", p, m.len())),
            Err(_) => states.push(format!("{} (absent)", p)),
        }
    }
    println!("{}", states.join("\n"));
    println!("\nChecklist: servers UP → kv GREEN/YELLOW → thermals Nominal/Fair → state saved.");
    println!("Then: context-cap --set → agent-resume --auto → loop-orchestrator --run");
    pause();
}

fn main() {
    let root = project_root();
    loop {
        clear();
        println!("==============================================");
        println!("  {}", status_header(&root));
        println!("  Mac Studio M5 Ultra 96GB | Q4<->Q8 | OpenCode V2");
        println!("==============================================\n");
        println!("  1) Health check (servers, kv, disk, thermals)");
        println!("  2) Serve menu (start/stop Q4/Q8, launchd install)");
        println!("  3) Smart Serve (auto-Q4/Q8 based on thermal/RAM)");
        println!("  4) Skills menu (verify, swap, kv, resume, audit)");
        println!("  5) Pull models (qwen3.8-27b)");
        println!("  6) Pre-flight check (8hr+ long runs)");
        println!("  7) Bootstrap (one-time setup)");
        println!("  0) Quit\n");
        print!("choice> ");
        let _ = io::stdout().flush();
        let mut c = String::new();
        if io::stdin().read_line(&mut c).is_err() {
            break;
        }
        match c.trim() {
            "1" => health_check(&root),
            "2" => serve_menu(&root),
            "3" => smart_serve(&root),
            "4" => skills_menu(),
            "5" => {
                clear();
                println!("Pulling qwen3.8-27b (may take a while)...\n");
                let _ = Command::new(bin(&root, "pull-models")).status();
                pause();
            }
            "6" => preflight(&root),
            "7" => {
                clear();
                println!("Running bootstrap...\n");
                let _ = Command::new(bin(&root, "bootstrap")).status();
                pause();
            }
            "0" | "q" | "quit" | "exit" => break,
            _ => {}
        }
    }
    clear();
    println!("LAC TUI exit.");
}
