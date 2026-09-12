//! LAC shared helpers — std-only, zero external crates.
//!
//! Every binary in this suite includes this file with a top-level
//! `mod common;` declaration (same directory, no Cargo.toml change).
//! Centralizes: memory probing, thermals, port/health checks,
//! atomic file writes, inter-process locks, and repo introspection.

use std::env;
use std::fs;
use std::io;
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------- paths ---

/// Home directory, falling back to "." when $HOME is unset.
pub fn home_dir() -> String {
    env::var("HOME").unwrap_or_else(|_| ".".to_string())
}

/// Project root: current dir when it holds opencode.jsonc, else derived
/// from the executable path (target/release/<bin> -> up 3 = rust-src's
/// parent), else current dir.
pub fn project_root() -> String {
    let cwd = env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());
    if fs::metadata(format!("{}/opencode.jsonc", cwd)).is_ok() {
        return cwd;
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(root) = exe
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
        {
            let r = root.to_string_lossy().to_string();
            if fs::metadata(format!("{}/opencode.jsonc", r)).is_ok() {
                return r;
            }
        }
    }
    cwd
}

/// Absolute path to a sibling release binary.
pub fn bin(root: &str, name: &str) -> String {
    format!("{}/rust-src/target/release/{}", root, name)
}

// ---------------------------------------------------------------- memory --

/// OS page size via `sysctl hw.pagesize`; Apple Silicon fallback 16384.
/// Never hardcode: Rosetta/Intel pages are 4096 and silently skew RAM math.
pub fn page_size() -> u64 {
    if let Ok(out) = Command::new("sysctl")
        .args(["-n", "hw.pagesize"])
        .output()
    {
        if let Ok(n) = String::from_utf8_lossy(&out.stdout).trim().parse::<u64>() {
            if n > 0 {
                return n;
            }
        }
    }
    16384
}

/// Free unified memory in GiB.
///
/// Counts free + inactive + speculative pages. Speculative pages are
/// reclaimable on memory pressure; ignoring them (as before) understated
/// free RAM by several GiB and caused false OOM alarms.
pub fn free_ram_gib() -> Option<f64> {
    let out = Command::new("vm_stat").output().ok()?;
    let txt = String::from_utf8_lossy(&out.stdout);
    let ps = page_size() as f64;
    let mut pages: f64 = 0.0;
    for line in txt.lines() {
        let l = line.trim();
        if l.starts_with("Pages free:")
            || l.starts_with("Pages inactive:")
            || l.starts_with("Pages speculative:")
        {
            let n: String = l.chars().filter(|c| c.is_ascii_digit()).collect();
            pages += n.parse::<f64>().unwrap_or(0.0);
        }
    }
    if pages <= 0.0 {
        return None;
    }
    Some(pages * ps / 1073741824.0)
}

/// Total physical RAM in GiB via `sysctl hw.memsize`.
pub fn total_ram_gib() -> Option<f64> {
    let out = Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    let n: f64 = String::from_utf8(out.stdout).ok()?.trim().parse().ok()?;
    if n <= 0.0 {
        return None;
    }
    Some(n / 1073741824.0)
}

// ---------------------------------------------------------------- thermals -

/// Raw `pmset -g thermals` output (empty string when unavailable, e.g.
/// virtualized macOS where pmset reports nothing useful).
pub fn thermal_detail() -> String {
    Command::new("pmset")
        .args(["-g", "thermals"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default()
}

/// One of Nominal | Fair | Serious | Critical.
/// Earlier code collapsed Serious into Nominal, hiding the throttle
/// warning that precedes Critical by minutes.
pub fn thermal_state() -> String {
    let out = thermal_detail();
    if out.contains("Critical") || out.contains("Overheat") {
        "Critical".to_string()
    } else if out.contains("Serious") {
        "Serious".to_string()
    } else if out.contains("Fair") {
        "Fair".to_string()
    } else {
        "Nominal".to_string()
    }
}

// ----------------------------------------------------------------- network -

/// TCP connect probe. Default budget 150ms keeps router hot-path fast.
pub fn port_up(port: u16) -> bool {
    port_up_ms(port, 150)
}

pub fn port_up_ms(port: u16, ms: u64) -> bool {
    let addr = format!("127.0.0.1:{}", port);
    match addr.parse() {
        Ok(sa) => TcpStream::connect_timeout(&sa, Duration::from_millis(ms)).is_ok(),
        Err(_) => false,
    }
}

/// Block until a TCP port accepts (or timeout). For supervising daemons.
pub fn wait_for_port(port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if port_up_ms(port, 200) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

/// HTTP readiness: `GET /v1/models` must return 200. A bound TCP port only
/// proves the socket exists; this proves the model is loaded and serving.
/// Used by the router health cache and server readiness threads.
pub fn http_ready(port: u16, timeout_ms: u64) -> bool {
    http_get(port, "/v1/models", timeout_ms)
        .map(|(code, _)| code == 200)
        .unwrap_or(false)
}

/// Minimal blocking HTTP/1.0-style GET over 127.0.0.1. Returns
/// (status_code, body). Cap 256 KiB — enough for model lists, bounded
/// against pathological responses.
pub fn http_get(port: u16, path: &str, timeout_ms: u64) -> Option<(u16, String)> {
    use std::io::{Read, Write};
    let addr = format!("127.0.0.1:{}", port);
    let sa: std::net::SocketAddr = addr.parse().ok()?;
    let mut s =
        TcpStream::connect_timeout(&sa, Duration::from_millis(timeout_ms.min(2000))).ok()?;
    let _ = s.set_read_timeout(Some(Duration::from_millis(timeout_ms.min(5000))));
    let _ = s.set_write_timeout(Some(Duration::from_millis(2000)));
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        path, addr
    );
    s.write_all(req.as_bytes()).ok()?;
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match s.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > 262_144 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&buf).to_string();
    let head_end = text.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0);
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .unwrap_or(0);
    Some((status, text[head_end.min(text.len())..].to_string()))
}

/// Block until `GET /v1/models` is 200 (or timeout). Servers call this
/// from a prober thread so `lac status` stops racing server boot.
pub fn wait_for_http(port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if http_ready(port, 1500) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

// ------------------------------------------------------------------ volumes -

/// Where the MLX lane lives: MLX_PORT env > serve-mlx's port file >
/// default 8080. serve-mlx drifts to 8082+ when 8080 is taken; without
/// this the router would probe a dead port while the server is up.
/// The readiness gate still applies, so a stale port file is harmless.
pub fn mlx_port() -> u16 {
    if let Ok(p) = env::var("MLX_PORT").or_else(|_| env::var("PORT")) {
        if let Ok(port) = p.trim().parse::<u16>() {
            if port > 0 {
                return port;
            }
        }
    }
    if let Ok(s) = fs::read_to_string(format!("{}/.lac/mlx.port", home_dir())) {
        if let Ok(port) = s.trim().parse::<u16>() {
            if port > 0 {
                return port;
            }
        }
    }
    8080
}

/// True only when `mount` shows the path as a real mount point.
/// NEVER `create_dir_all` under /Volumes on a false here: that shadows
/// the future mount with an internal-disk directory of the same name.
pub fn volume_mounted(mount_point: &str) -> bool {
    if let Ok(out) = Command::new("mount").output() {
        let txt = String::from_utf8_lossy(&out.stdout);
        let needle = format!(" on {} (", mount_point);
        if txt.lines().any(|l| l.contains(&needle)) {
            return true;
        }
    }
    false
}

/// Canonical model library: TB5 external when mounted, else internal
/// fallback. Callers must not create /Volumes paths without this gate.
pub fn model_base() -> String {
    if volume_mounted("/Volumes/AIModels") {
        "/Volumes/AIModels".to_string()
    } else {
        format!("{}/.lac/models", home_dir())
    }
}

// -------------------------------------------------------------------- files -

/// Write-temp-then-rename. A crash mid-write never leaves a half file,
/// which is what corrupted the Kanban queue under the old direct writes.
pub fn atomic_write(path: &str, contents: &str) -> io::Result<()> {
    let p = Path::new(path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let tmp = format!("{}.tmp.{}", path, std::process::id());
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn append_jsonl(path: &str, line: &str) {
    if let Some(parent) = Path::new(path).parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
        use std::io::Write as _;
        let _ = writeln!(f, "{}", line);
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Minimal JSON string escaper for hand-built payloads (std-only).
pub fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

// -------------------------------------------------------------------- locks -

/// Crash-safe inter-process lock via atomic `create_dir` + owner record.
/// Prevents two `lac worker` daemons (launchd + terminal) draining the
/// same queue twice. Stale locks (dead owner or age) are reclaimed.
pub struct DirLock {
    dir: String,
}

impl DirLock {
    pub fn dir(&self) -> &str {
        &self.dir
    }
}

impl Drop for DirLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Liveness probe: Some(true) alive, Some(false) dead,
/// None when the probe itself could not run (fail-open unknown).
fn process_alive(pid: i32) -> Option<bool> {
    let out = Command::new("kill").args(["-0", &pid.to_string()]).output().ok()?;
    Some(out.status.success())
}

/// Acquire `~/.lac/locks/<name>`. Returns None when a live owner holds it.
/// `stale_secs` is only a backstop for when the owner pid cannot be probed;
/// a live owner keeps the lock regardless of wall-clock age (a forward
/// clock jump must never hand one queue to two workers).
pub fn acquire_lock(name: &str, stale_secs: u64) -> Option<DirLock> {
    let parent = format!("{}/.lac/locks", home_dir());
    let _ = fs::create_dir_all(&parent);
    let dir = format!("{}/{}", parent, name);
    for _ in 0..2 {
        match fs::create_dir(&dir) {
            Ok(()) => {
                let owner = format!("{} {}", std::process::id(), now_unix());
                let _ = fs::write(format!("{}/owner", dir), owner);
                return Some(DirLock { dir });
            }
            Err(_) => {
                let owner_path = format!("{}/owner", dir);
                // Liveness first: a live owner keeps the lock even if the
                // wall clock jumped forward (NTP/VM resume). Wall-clock age
                // is only a backstop when the pid cannot be probed at all.
                let stale = match fs::read_to_string(&owner_path) {
                    Ok(c) => {
                        let mut it = c.split_whitespace();
                        let pid: i32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(-1);
                        let ts: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                        if pid <= 0 {
                            true
                        } else {
                            match process_alive(pid) {
                                Some(false) => true,
                                Some(true) => false,
                                None => now_unix().saturating_sub(ts) >= stale_secs,
                            }
                        }
                    }
                    Err(_) => true,
                };
                if stale {
                    let _ = fs::remove_dir_all(&dir);
                    continue;
                }
                return None;
            }
        }
    }
    None
}

// ---------------------------------------------------------------------- git -

/// Current branch in `dir`, working even on unborn repos (no commits yet).
/// Falls back to whichever of main/master exists, else "main".
/// Takes a directory so daemon flows never depend on process cwd.
pub fn default_branch_in(dir: &str) -> String {
    if let Ok(out) = Command::new("git")
        .current_dir(dir)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
    {
        if out.status.success() {
            let b = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !b.is_empty() {
                return b;
            }
        }
    }
    for cand in ["main", "master"] {
        let ok = Command::new("git")
            .current_dir(dir)
            .args(["show-ref", "--verify", &format!("refs/heads/{}", cand)])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            return cand.to_string();
        }
    }
    "main".to_string()
}

/// Current branch in the process working directory.
pub fn default_branch() -> String {
    default_branch_in(".")
}

/// Commit existence in `dir` (explicit cwd for daemon flows).
pub fn repo_has_commits_in(dir: &str) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// False on fresh `git init` repos (no HEAD) — callers must skip
/// branch/commit flows there instead of failing on `master`.
pub fn repo_has_commits() -> bool {
    repo_has_commits_in(".")
}

/// Dirtiness in `dir` (explicit cwd for daemon flows).
pub fn repo_dirty_in(dir: &str) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
}

/// True when `git status --porcelain` is non-empty (tracked mods or
/// untracked files). The worker treats a dirty tree as precious: it
/// never runs `checkout .` over user work.
pub fn repo_dirty() -> bool {
    Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
}

// -------------------------------------------------------------------- events -

/// Worker / daemon event log. JSONL so `grep`/jq keep working at 3am.
pub fn event_log_path() -> String {
    format!("{}/.lac/worker-events.jsonl", home_dir())
}

pub fn log_event(kind: &str, msg: &str) {
    let line = format!(
        "{{\"ts\":{},\"event\":\"{}\",\"msg\":\"{}\"}}",
        now_unix(),
        json_escape(kind),
        json_escape(msg)
    );
    append_jsonl(&event_log_path(), &line);
}

// ------------------------------------------------- native shell replacements -
// Every helper below exists to delete a shell-out: `which`, `tail`,
// `uname`, `pgrep`/`pkill`, and `pgrep -P` tree walks are now parsed or
// probed in Rust. Only one `ps` snapshot and one `kill` per signal remain
// as subprocesses (macOS offers no std API for either), and both are
// wrapped here so no binary shells out ad hoc anymore.

/// PATH lookup without spawning `which`. Returns the full path only when
/// the candidate exists AND has any execute bit (a shadow non-executable
/// file must not satisfy the probe).
pub fn which(cmd: &str) -> Option<String> {
    if cmd.contains('/') {
        let p = Path::new(cmd);
        if is_executable(p) {
            return Some(cmd.to_string());
        }
        return None;
    }
    let path = env::var("PATH").unwrap_or_default();
    for dir in path.split(':') {
        if dir.is_empty() {
            continue;
        }
        let cand = format!("{}/{}", dir, cmd);
        if is_executable(Path::new(&cand)) {
            return Some(cand);
        }
    }
    None
}

fn is_executable(p: &Path) -> bool {
    fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// OS/arch without spawning `uname`. (Kernel release omitted on purpose:
/// nothing in the stack consumes it.)
pub fn os_descr() -> String {
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Last `n` lines of a file (trailing newline normalized). Replaces
/// `tail -n` with zero subprocesses; fine for log-sized files.
pub fn tail_file(path: &str, n: usize) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Some(String::new());
    }
    let start = lines.len().saturating_sub(n.max(1));
    let mut out = lines[start..].join("\n");
    out.push('\n');
    Some(out)
}

/// First `n` lines of a string. Replaces `| head -N` in display paths.
pub fn head_lines(s: &str, n: usize) -> String {
    s.lines().take(n.max(1)).collect::<Vec<_>>().join("\n")
}

/// One row of `ps -eo pid=,ppid=,command=`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub cmd: String,
}

/// Parse one `ps` row: "<pid> <ppid> <command...>". Byte-index math lands
/// on ASCII whitespace/token boundaries only (pids are digits), so this
/// is panic-free on hostile input; kernel threads without commands yield None.
pub fn parse_ps_line(line: &str) -> Option<ProcInfo> {
    let t = line.trim_start();
    let p1e = t.find(|c: char| c.is_whitespace())?;
    let pid: u32 = t[..p1e].parse().ok()?;
    let rest = t[p1e..].trim_start();
    let p2e = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
    let ppid: u32 = rest[..p2e].parse().ok()?;
    let cmd = rest[p2e..].trim_start().to_string();
    if cmd.is_empty() {
        return None;
    }
    Some(ProcInfo { pid, ppid, cmd })
}

/// Single `ps` snapshot, parsed in Rust. The only process-listing
/// subprocess in the suite; every `pgrep`/`pkill` call site funnels here.
pub fn process_snapshot() -> Vec<ProcInfo> {
    Command::new("ps")
        .args(["-eo", "pid=,ppid=,command="])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .map(|t| t.lines().filter_map(parse_ps_line).collect())
        .unwrap_or_default()
}

/// Processes whose command line contains `substr`, excluding self.
/// (The old `pgrep -af` wrapper used to match its own probe command.)
pub fn processes_matching(substr: &str) -> Vec<ProcInfo> {
    let me = std::process::id();
    process_snapshot()
        .into_iter()
        .filter(|p| p.pid != me && p.cmd.contains(substr))
        .collect()
}

/// Direct children of `pid` (replaces `pgrep -P`).
pub fn children_of(pid: u32) -> Vec<u32> {
    process_snapshot()
        .iter()
        .filter(|p| p.ppid == pid)
        .map(|p| p.pid)
        .collect()
}

/// Signal a pid via `kill` (macOS has no std API for arbitrary pids).
/// Returns false when the signal could not be delivered.
pub fn signal_pid(pid: u32, sig: &str) -> bool {
    Command::new("kill")
        .args([format!("-{}", sig), pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// SIGTERM every process matching `substr`. Returns kill count.
/// Replaces `pkill -f` with an auditable Rust loop.
pub fn kill_matching(substr: &str) -> usize {
    processes_matching(substr)
        .iter()
        .filter(|p| signal_pid(p.pid, "TERM"))
        .count()
}

/// SIGKILL a process tree, children first (recursive). Best-effort:
/// races with natural exit are harmless. Replaces the old
/// `pgrep -P`-snapshot shell pipeline with the same semantics.
pub fn kill_tree(pid: u32) {
    for child in children_of(pid) {
        kill_tree(child);
    }
    signal_pid(pid, "9");
}

// --------------------------------------------------------------------- tests -

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_escape_roundtrip_safe() {
        assert_eq!(json_escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
        assert_eq!(json_escape("tab\there"), "tab\\there");
        assert_eq!(json_escape("plain"), "plain");
        assert_eq!(json_escape("\u{1}"), "\\u0001");
    }

    #[test]
    fn page_size_plausible() {
        let ps = page_size();
        assert!(ps == 4096 || ps == 16384 || ps == 65536, "ps={}", ps);
    }

    #[test]
    fn volume_mounted_rejects_fantasy_paths() {
        assert!(!volume_mounted("/Volumes/Definitely-Not-A-Volume-xyz"));
    }

    #[test]
    fn atomic_write_roundtrip() {
        let p = format!(
            "{}/lac-test-atomic-{}.txt",
            std::env::temp_dir().to_string_lossy(),
            std::process::id()
        );
        atomic_write(&p, "hello").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "hello");
        atomic_write(&p, "world").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "world");
        let _ = fs::remove_file(&p);
        // No stray tmp file left behind.
        assert!(!Path::new(&format!("{}.tmp.{}", p, std::process::id())).exists());
    }

    #[test]
    fn lock_excludes_second_owner_and_releases_on_drop() {
        let name = format!("test-{}", std::process::id());
        let a = acquire_lock(&name, 60);
        assert!(a.is_some());
        assert!(acquire_lock(&name, 60).is_none());
        drop(a);
        assert!(acquire_lock(&name, 60).is_some());
    }

    #[test]
    fn default_branch_never_empty() {
        assert!(!default_branch().is_empty());
    }

    #[test]
    fn which_finds_shell_and_rejects_ghosts() {
        let sh = which("sh");
        assert!(sh.is_some(), "sh must resolve via PATH");
        assert!(which("definitely-not-a-binary-xyz").is_none());
        // Explicit path without exec bit (or missing) is rejected.
        assert!(which("/nonexistent/path/xyz").is_none());
    }

    #[test]
    fn tail_file_roundtrip() {
        let p = format!(
            "{}/lac-test-tail-{}.log",
            std::env::temp_dir().to_string_lossy(),
            std::process::id()
        );
        fs::write(&p, "a\nb\nc\nd\n").unwrap();
        assert_eq!(tail_file(&p, 2).as_deref(), Some("c\nd\n"));
        assert_eq!(tail_file(&p, 99).as_deref(), Some("a\nb\nc\nd\n"));
        assert_eq!(tail_file(&p, 0).as_deref(), Some("d\n"));
        assert!(tail_file("/nonexistent/lac-tail-xyz", 10).is_none());
        let _ = fs::remove_file(&p);
        assert_eq!(head_lines("a\nb\nc", 2), "a\nb");
    }

    #[test]
    fn ps_line_parsing() {
        let p = parse_ps_line("  123   456 /usr/bin/foo --bar baz").unwrap();
        assert_eq!(p, ProcInfo { pid: 123, ppid: 456, cmd: "/usr/bin/foo --bar baz".to_string() });
        assert!(parse_ps_line("not a row").is_none());
        assert!(parse_ps_line("12").is_none());
        assert!(parse_ps_line("  7   1 ").is_none()); // kernel thread, no command
        assert!(parse_ps_line("").is_none());
    }

    #[test]
    fn snapshot_contains_self() {
        let me = std::process::id();
        let snap = process_snapshot();
        assert!(!snap.is_empty());
        assert!(snap.iter().any(|p| p.pid == me));
    }

    #[test]
    fn kill_tree_reaps_own_child() {
        let mut child = Command::new("sleep")
            .arg("120")
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("sleep must exist");
        let pid = child.id();
        kill_tree(pid);
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(st)) => {
                    assert!(!st.success(), "SIGKILLed child must not exit 0");
                    break;
                }
                Ok(None) => {
                    assert!(start.elapsed() < Duration::from_secs(10), "child survived kill_tree");
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => panic!("try_wait: {}", e),
            }
        }
    }
}
