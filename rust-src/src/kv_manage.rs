//! kv-manage v2.7 — context cache risk analyzer & truncation engine.
//!
//! v2.7: the router's usage log (~/.lac/router-usage.jsonl) is now the
//! fallback token source, replacing the fragile opencode2-endpoint
//! scrape. Estimates can only escalate GREEN→YELLOW: only measurements
//! (explicit --tokens) or real RAM pressure may call RED.

mod common;

use std::env;
use std::fs;
use std::process::Command;

// qwen3.8-27B full-attention KV footprint, FP16 bytes per token.
// 16 full-attn layers x 4 KV heads x 256 dim x 2 (K+V) x 2 bytes = 32 KiB.
// Gated DeltaNet layers hold recurrent state, not full KV — excluded.
const FP16_BYTES_PER_TOKEN: f64 = 32768.0;

// RAM pressure coupling, scaled by total RAM via
// `common::mem_thresholds_gib()` (sane from 8GB Air to 96GB Studio).

fn usage() -> ! {
    eprintln!("usage: kv-manage check [--tokens N] [--threshold N] [--kv-q4|--kv-q8|--kv-fp16] [--json]");
    eprintln!("       kv-manage truncate --tokens N [--keep N]");
    eprintln!("exit codes: check 0 GREEN / 1 YELLOW / 2 RED; usage errors 3; truncate failures 1");
    std::process::exit(3);
}

// Best-effort session token count: explicit flag > LAC_TOKENS env >
// router usage log > opencode2 probe. Only integers attached to a
// token-like key count; bare-number scraping produced false positives.
fn session_tokens() -> Option<u64> {
    if let Ok(v) = env::var("LAC_TOKENS") {
        if let Ok(n) = v.trim().parse::<u64>() {
            return Some(n);
        }
    }
    if let Some(n) = router_est_tokens(3600) {
        return Some(n);
    }
    opencode_tokens()
}

/// Legacy last-resort probe (opencode2 endpoint scrape).
fn opencode_tokens() -> Option<u64> {
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

fn kv_gib(tokens: u64, factor: f64) -> f64 {
    tokens as f64 * FP16_BYTES_PER_TOKEN * factor / 1073741824.0
}

/// Extract an unsigned integer JSON value for `"key":` (tolerant of
/// whitespace). Returns None when absent or malformed.
fn json_u64(line: &str, key: &str) -> Option<u64> {
    let p = line.find(key)?;
    let rest = line[p + key.len()..].trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    rest[..end].parse().ok()
}

/// Sum `est_tokens` from the router usage log (current + rotated window)
/// for entries within the last `window_secs`. None when the router never
/// logged (no gateway traffic yet) — distinct from a measured zero.
fn router_est_tokens(window_secs: u64) -> Option<u64> {
    let now = common::now_unix();
    let home = common::home_dir();
    let mut sum = 0u64;
    let mut lines = 0u64;
    for suffix in ["", ".1"] {
        let path = format!("{}/.lac/router-usage{}.jsonl", home, suffix);
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for line in content.lines() {
            let (Some(ts), Some(est)) =
                (json_u64(line, "\"ts\":"), json_u64(line, "\"est_tokens\":"))
            else {
                continue;
            };
            lines += 1;
            if ts.saturating_add(window_secs) >= now {
                sum = sum.saturating_add(est);
            }
        }
    }
    if lines == 0 {
        None
    } else {
        Some(sum)
    }
}

/// Pure risk matrix: token pressure OR memory pressure drives the level.
/// `measured` (explicit --tokens / LAC_TOKENS) may call RED; `estimate`
/// (router log, probes) may only escalate to YELLOW — estimates prompt
/// attention, only measurements prompt action. Returns (LEVEL, action).
fn risk_level_full(
    measured: Option<u64>,
    estimate: Option<u64>,
    threshold: u64,
    free_gib: Option<f64>,
) -> (&'static str, &'static str) {
    let (red_gib, yellow_gib) = common::mem_thresholds_gib();
    risk_level_with(measured, estimate, threshold, free_gib, red_gib, yellow_gib)
}

/// Pure matrix with explicit RAM gates (host-independent — use in tests).
fn risk_level_with(
    measured: Option<u64>,
    estimate: Option<u64>,
    threshold: u64,
    free_gib: Option<f64>,
    red_gib: f64,
    yellow_gib: f64,
) -> (&'static str, &'static str) {
    let pct = |t: u64| {
        if threshold > 0 {
            t.saturating_mul(100) / threshold.max(1)
        } else {
            0
        }
    };
    let m_red = measured.map(|t| t >= threshold).unwrap_or(false);
    let m_yel = measured.map(|t| pct(t) >= 66).unwrap_or(false);
    let e_yel = estimate.map(|t| pct(t) >= 66).unwrap_or(false);
    let mem_red = free_gib.map(|f| f < red_gib).unwrap_or(false);
    let mem_yellow = free_gib.map(|f| f < yellow_gib).unwrap_or(false);
    if m_red || mem_red {
        ("RED", "truncate now: kv-manage truncate --tokens N")
    } else if m_yel || e_yel || mem_yellow {
        ("YELLOW", "plan truncation within next few turns")
    } else {
        ("GREEN", "continue")
    }
}

#[allow(dead_code)]
fn risk_level(tokens: u64, threshold: u64, free_gib: Option<f64>) -> (&'static str, &'static str) {
    risk_level_full(Some(tokens), None, threshold, free_gib)
}

#[cfg(test)]
fn risk_level_fixed(tokens: u64, threshold: u64, free_gib: Option<f64>) -> (&'static str, &'static str) {
    // Fixed 2.0/8.0 gates: preserves the original 96GB-Studio expectations
    // independent of the test host's total RAM.
    risk_level_with(Some(tokens), None, threshold, free_gib, 2.0, 8.0)
}

fn exit_for(level: &str) -> i32 {
    match level {
        "RED" => 2,
        "YELLOW" => 1,
        _ => 0,
    }
}

fn cmd_check(args: &[String]) {
    let mut tokens: Option<u64> = None;
    let mut threshold: u64 = 24000;
    let mut factor = 0.5; // q8 default
    let mut kv_label = "q8";
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--tokens" => {
                i += 1;
                tokens = args.get(i).and_then(|s| s.parse().ok());
            }
            "--threshold" => {
                i += 1;
                threshold = args.get(i).and_then(|s| s.parse::<u64>().ok()).unwrap_or(24000);
            }
            "--kv-q4" => {
                factor = 0.25;
                kv_label = "q4";
            }
            "--kv-q8" => {
                factor = 0.5;
                kv_label = "q8";
            }
            "--kv-fp16" => {
                factor = 1.0;
                kv_label = "fp16";
            }
            "--json" => json = true,
            _ => usage(),
        }
        i += 1;
    }
    // Token provenance: explicit flag > LAC_TOKENS env (both measured) >
    // router 1h estimate / legacy probe (estimates, YELLOW-capped).
    let flag_tokens = tokens;
    let env_tokens = if flag_tokens.is_none() {
        env::var("LAC_TOKENS").ok().and_then(|v| v.trim().parse().ok())
    } else {
        None
    };
    let measured = flag_tokens.or(env_tokens);
    let (estimate, est_source) = if measured.is_none() {
        match router_est_tokens(3600) {
            Some(n) => (Some(n), "router-1h-est"),
            None => match opencode_tokens() {
                Some(n) => (Some(n), "opencode-probe"),
                None => (None, "unknown"),
            },
        }
    } else {
        (None, "")
    };
    let source = if flag_tokens.is_some() {
        "explicit"
    } else if env_tokens.is_some() {
        "env"
    } else {
        est_source
    };
    let tokens = measured.or(estimate).unwrap_or(0);
    let kv = kv_gib(tokens, factor);
    let free = common::free_ram_gib();
    let (level, action) = risk_level_full(measured, estimate, threshold, free);

    if json {
        println!(
            "{{\"tokens\":{},\"tokens_source\":\"{}\",\"tokens_measured\":{},\"kv_est_gib\":{:.2},\"kv_kind\":\"{}\",\"free_ram_gib\":{},\"threshold\":{},\"risk\":\"{}\",\"action\":\"{}\"}}",
            tokens,
            source,
            measured.is_some(),
            kv,
            kv_label,
            free.map(|f| format!("{:.1}", f)).unwrap_or_else(|| "null".to_string()),
            threshold,
            level,
            common::json_escape(action),
        );
    } else {
        println!("tokens: {} ({})", tokens, source);
        if measured.is_none() && estimate.is_some() {
            println!("note: estimate only — pass --tokens N for a measured reading");
        }
        println!("kv_est_gib ({}): {:.2}", kv_label, kv);
        match free {
            Some(f) => println!("free_ram_gib: {:.1}", f),
            None => println!("free_ram_gib: unknown (vm_stat failed)"),
        }
        println!("threshold: {}", threshold);
        println!("risk: {}", level);
        println!("action: {}", action);
    }
    std::process::exit(exit_for(level));
}

fn archive_dir() -> Option<String> {
    // model_base() already gates on the real mount; this can never
    // shadow /Volumes with an internal directory.
    let dir = format!("{}/hf/kv_cache_history", common::model_base());
    match fs::create_dir_all(&dir) {
        Ok(()) => Some(dir),
        Err(e) => {
            eprintln!("archive dir {} unavailable: {}", dir, e);
            None
        }
    }
}

fn cmd_truncate(args: &[String]) {
    let mut tokens: Option<u64> = None;
    let mut keep: u64 = 8000;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--tokens" => {
                i += 1;
                tokens = args.get(i).and_then(|s| s.parse().ok());
            }
            "--keep" => {
                i += 1;
                keep = args.get(i).and_then(|s| s.parse::<u64>().ok()).unwrap_or(8000);
            }
            _ => usage(),
        }
        i += 1;
    }
    if keep == 0 {
        eprintln!("--keep must be > 0");
        std::process::exit(3);
    }
    let tokens = tokens.or_else(session_tokens).unwrap_or(0);
    if keep > tokens && tokens > 0 {
        eprintln!("warning: --keep {} exceeds tokens_before {}; keeping everything", keep, tokens);
    }
    let dir = match archive_dir() {
        Some(d) => d,
        None => std::process::exit(1),
    };
    let ts = common::now_unix();
    let path = format!("{}/truncate-{}.json", dir, ts);
    let manifest = format!(
        "{{\"tokens_before\":{},\"keep_last\":{},\"preserve\":[\"system\",\"tools\"],\"ts\":{}}}",
        tokens, keep, ts
    );
    if let Err(e) = common::atomic_write(&path, &manifest) {
        eprintln!("archive write {} failed: {}", path, e);
        std::process::exit(1);
    }
    println!("manifest: {}", path);
    println!("tokens_before: {}", tokens);
    println!("keep_last: {}", keep);
    println!("next: in OpenCode, replace history with [system]+[tools]+[summary]+[last {} tokens], then verify with: kv-manage check", keep);
}

fn main() {
    common::ignore_sigpipe();
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        usage();
    }
    match args[0].as_str() {
        "check" => cmd_check(&args[1..]),
        "truncate" => cmd_truncate(&args[1..]),
        _ => usage(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn green_when_calm() {
        assert_eq!(risk_level_fixed(1000, 24000, Some(60.0)).0, "GREEN");
    }

    #[test]
    fn red_on_tokens() {
        assert_eq!(risk_level_fixed(24000, 24000, Some(60.0)).0, "RED");
    }

    #[test]
    fn yellow_on_token_pressure() {
        assert_eq!(risk_level_fixed(16000, 24000, Some(60.0)).0, "YELLOW");
    }

    #[test]
    fn red_on_ram_regardless_of_tokens() {
        assert_eq!(risk_level_fixed(500, 24000, Some(1.2)).0, "RED");
    }

    #[test]
    fn yellow_on_ram() {
        assert_eq!(risk_level_fixed(500, 24000, Some(6.0)).0, "YELLOW");
    }

    #[test]
    fn unknown_ram_falls_back_to_tokens() {
        assert_eq!(risk_level_fixed(500, 24000, None).0, "GREEN");
        assert_eq!(risk_level_fixed(30000, 24000, None).0, "RED");
    }

    #[test]
    fn kv_math_sane() {
        // 32K tokens fp16 = 1 GiB exactly (32768 * 32768 bytes).
        assert!((kv_gib(32768, 1.0) - 1.0).abs() < 1e-9);
        assert!((kv_gib(32768, 0.5) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn estimates_cap_at_yellow() {
        // A huge estimate alone must never call RED.
        assert_eq!(risk_level_with(None, Some(1_000_000), 24000, Some(60.0), 2.0, 8.0).0, "YELLOW");
        // ...but a measured value at threshold still goes RED.
        assert_eq!(risk_level_with(Some(24000), Some(1_000_000), 24000, Some(60.0), 2.0, 8.0).0, "RED");
        // Small estimate stays GREEN.
        assert_eq!(risk_level_with(None, Some(500), 24000, Some(60.0), 2.0, 8.0).0, "GREEN");
        // No data at all is GREEN (RAM calm).
        assert_eq!(risk_level_with(None, None, 24000, Some(60.0), 2.0, 8.0).0, "GREEN");
    }

    #[test]
    fn json_u64_parses_tolerantly() {
        assert_eq!(json_u64(r#"{"ts":123,"est_tokens":456}"#, "\"ts\":"), Some(123));
        assert_eq!(json_u64(r#"{"ts": 123 }"#, "\"ts\":"), Some(123));
        assert_eq!(json_u64(r#"{"nope":1}"#, "\"ts\":"), None);
        assert_eq!(json_u64(r#"{"ts":"abc"}"#, "\"ts\":"), None);
    }
}
