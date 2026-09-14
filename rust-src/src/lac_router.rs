//! lac-router v2.7 — adaptive unified gateway on :8000.
//!
//! v2.7 learning layer over v2.6:
//! - Per-backend EWMA first-byte latency + outcome/error counters, exposed
//!   in /lac/status (`stats`) — the gateway measures instead of guessing.
//! - `fastest` preference: opt-in lowest-measured-latency routing (tiers
//!   stay policy-ordered by default; speed never silently overrides the
//!   operator's quality choice).
//! - Circuit breaker: 3 consecutive failures cool a backend down for 30s.
//! - Per-request usage log (~/.lac/router-usage.jsonl, rotated at 8MB)
//!   with byte counts and token estimates — the honest token source that
//!   lets kv-manage stop scraping for session numbers.
//! - Zero-cost model sniffing: when the request already names a model and
//!   the merged catalog mapped it to a specific live backend, that backend
//!   goes first (no extra reads, no added latency, pure reorder).

mod common;

use std::collections::{HashMap, HashSet};
use std::env;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

// Backend IDs
const BACKEND_AUTO: usize = 0;
const BACKEND_MLX: usize = 1;
const BACKEND_LLAMA: usize = 2;
const BACKEND_OLLAMA: usize = 3;
const BACKEND_FASTEST: usize = 4;

/// MLX lane port with drift awareness (serve-mlx moves to 8082+ when
/// 8080 is taken and records it in ~/.lac/mlx.port).
fn port_mlx() -> u16 {
    common::mlx_port()
}

/// llama/ollama ports are env-overridable (testability + non-standard
/// layouts); the router still readiness-gates whatever is configured.
fn port_llama() -> u16 {
    env::var("LAC_LLAMA_PORT")
        .ok()
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or(8081)
}

fn port_ollama() -> u16 {
    env::var("LAC_OLLAMA_PORT")
        .ok()
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or(11434)
}

/// Tier order with live ports: [(id, port)] MLX -> llama -> Ollama.
fn tier_backends() -> Vec<(usize, u16)> {
    vec![
        (BACKEND_MLX, port_mlx()),
        (BACKEND_LLAMA, port_llama()),
        (BACKEND_OLLAMA, port_ollama()),
    ]
}

#[allow(dead_code)]
fn backend_id_for_port(port: u16) -> usize {
    for (id, p) in tier_backends() {
        if p == port {
            return id;
        }
    }
    BACKEND_AUTO
}

const MAX_HEADERS: usize = 65536;
const MAX_INFLIGHT: usize = 128;
const STREAM_BUDGET: Duration = Duration::from_secs(600);

fn backend_name(id: usize) -> &'static str {
    match id {
        BACKEND_MLX => "mlx",
        BACKEND_LLAMA => "llama",
        BACKEND_OLLAMA => "ollama",
        BACKEND_FASTEST => "fastest",
        _ => "auto",
    }
}

fn backend_id(name: &str) -> usize {
    match name.to_lowercase().as_str() {
        "mlx" => BACKEND_MLX,
        "llama" => BACKEND_LLAMA,
        "ollama" => BACKEND_OLLAMA,
        "fastest" => BACKEND_FASTEST,
        _ => BACKEND_AUTO,
    }
}

fn backend_port(id: usize) -> u16 {
    match id {
        BACKEND_MLX => port_mlx(),
        BACKEND_LLAMA => port_llama(),
        BACKEND_OLLAMA => port_ollama(),
        _ => port_mlx(),
    }
}

// ------------------------------------------------------------ health cache --

struct HealthCache {
    inner: Mutex<HashMap<u16, (bool, Instant)>>,
    ttl: Duration,
}

impl HealthCache {
    fn new(ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    /// Cached HTTP readiness. One `/v1/models` probe per backend per TTL
    /// window instead of 3× TCP connects on every request.
    fn ready(&self, port: u16) -> bool {
        if let Ok(guard) = self.inner.lock() {
            if let Some((v, t)) = guard.get(&port) {
                if t.elapsed() < self.ttl {
                    return *v;
                }
            }
        }
        let v = common::http_ready(port, 1500);
        if let Ok(mut guard) = self.inner.lock() {
            guard.insert(port, (v, Instant::now()));
        }
        v
    }
}

// ------------------------------------------------------- measured routing --

/// Per-backend learned state. EWMA weights recent first-byte latency so
/// the router adapts within a handful of requests; error streaks feed
/// the circuit breaker; byte/token counters feed the usage log.
#[derive(Clone, Default)]
struct BackendStats {
    ewma_ms: Option<f64>,
    ok: u64,
    err: u64,
    consec_err: u32,
    last_err: Option<Instant>,
    req_bytes: u64,
    resp_bytes: u64,
    est_tokens: u64,
}

type StatsMap = Arc<Mutex<HashMap<u16, BackendStats>>>;

/// model id -> (backend port, mapped_at). Populated from merged catalogs;
/// entries older than 5 minutes are ignored (backends come and go).
type RouteMap = Arc<Mutex<HashMap<String, (u16, Instant)>>>;

const BREAKER_ERRORS: u32 = 3;
const BREAKER_COOLDOWN: Duration = Duration::from_secs(30);
const ROUTE_TTL: Duration = Duration::from_secs(300);
const USAGE_ROTATE_BYTES: u64 = 8_000_000;
/// Unmeasured backends sort as slow: measured evidence always wins.
const EWMA_UNKNOWN_MS: f64 = 30_000.0;

fn note_success(stats: &StatsMap, port: u16, ttfb_ms: f64, req_body: u64, resp_body: u64) {
    if let Ok(mut m) = stats.lock() {
        let s = m.entry(port).or_default();
        s.ewma_ms = Some(match s.ewma_ms {
            Some(e) => 0.7 * e + 0.3 * ttfb_ms,
            None => ttfb_ms,
        });
        s.ok += 1;
        s.consec_err = 0;
        s.req_bytes += req_body;
        s.resp_bytes += resp_body;
        s.est_tokens += (req_body + resp_body) / 4;
    }
}

fn note_error(stats: &StatsMap, port: u16) {
    if let Ok(mut m) = stats.lock() {
        let s = m.entry(port).or_default();
        s.err += 1;
        s.consec_err += 1;
        s.last_err = Some(Instant::now());
    }
}

/// Pure breaker predicate: open when failures are repeated AND recent.
fn breaker_open(consec: u32, last_err: Option<Instant>, now: Instant) -> bool {
    consec >= BREAKER_ERRORS
        && last_err
            .map(|t| now.duration_since(t) < BREAKER_COOLDOWN)
            .unwrap_or(false)
}

fn breaker_open_for(stats: &StatsMap, port: u16) -> bool {
    match stats.lock() {
        Ok(m) => m
            .get(&port)
            .map(|s| breaker_open(s.consec_err, s.last_err, Instant::now()))
            .unwrap_or(false),
        Err(_) => false,
    }
}

fn ewma_of(stats: &StatsMap, port: u16) -> f64 {
    stats
        .lock()
        .ok()
        .and_then(|m| m.get(&port).and_then(|s| s.ewma_ms))
        .unwrap_or(EWMA_UNKNOWN_MS)
}

/// Best-effort model hint from already-buffered bytes (headers plus any
/// over-read body). Zero extra waiting: None simply means "route blind".
fn sniff_model(buf: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(buf);
    let key = "\"model\"";
    let p = s.find(key)?;
    let rest = &s[p + key.len()..];
    let c = rest.find(':')?;
    let after = rest[c + 1..].trim_start();
    let tail = after.strip_prefix('"')?;
    let mut out = String::new();
    let mut esc = false;
    for ch in tail.chars() {
        if esc {
            out.push(ch);
            esc = false;
        } else if ch == '\\' {
            esc = true;
        } else if ch == '"' {
            return Some(out);
        } else {
            out.push(ch);
        }
    }
    None
}

fn usage_log_path() -> String {
    format!("{}/.lac/router-usage.jsonl", common::home_dir())
}

/// One JSONL line per proxied request. Rotated past 8MB (previous window
/// kept as `.1`) so the log can never fill a disk by itself.
#[allow(clippy::too_many_arguments)]
fn log_usage(
    rid: usize,
    backend: &str,
    port: u16,
    model: &str,
    up_body: u64,
    down_body: u64,
    est_tokens: u64,
    ttfb_ms: Option<f64>,
    outcome: &str,
) {
    let path = usage_log_path();
    if std::fs::metadata(&path)
        .map(|m| m.len() > USAGE_ROTATE_BYTES)
        .unwrap_or(false)
    {
        let _ = std::fs::rename(&path, format!("{}.1", path));
    }
    let line = format!(
        "{{\"ts\":{},\"rid\":{},\"backend\":\"{}\",\"port\":{},\"model\":\"{}\",\"req_body\":{},\"resp_body\":{},\"est_tokens\":{},\"ttfb_ms\":{},\"outcome\":\"{}\"}}",
        common::now_unix(),
        rid,
        backend,
        port,
        common::json_escape(model),
        up_body,
        down_body,
        est_tokens,
        ttfb_ms.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "null".to_string()),
        outcome,
    );
    common::append_jsonl(&path, &line);
}

// ---------------------------------------------------------- request parsing -

fn find_headers_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
}

/// Read until end-of-headers (or limits). The old single 4 KiB read cut
/// large auth/tool-call headers in half and poisoned the forwarded stream.
fn read_headers(client: &TcpStream) -> io::Result<Vec<u8>> {
    // Owned handle: bytes consumed here are the client's already-sent
    // headers; the caller forwards them explicitly and streams the rest.
    let mut stream = client.try_clone()?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_HEADERS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request headers exceed 64 KiB",
            ));
        }
        if find_headers_end(&buf).is_some() {
            break;
        }
    }
    if buf.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "empty request",
        ));
    }
    Ok(buf)
}

fn request_line(buf: &[u8]) -> (String, String) {
    let head = String::from_utf8_lossy(buf);
    let first = head.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    (
        parts.next().unwrap_or("").to_string(),
        parts.next().unwrap_or("/").to_string(),
    )
}

// -------------------------------------------------------------- http blobs --

fn http_response(status: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
        status,
        content_type,
        body.len(),
        body
    )
}

fn json_response(stream: &mut TcpStream, status: &str, body: &str) -> io::Result<()> {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    stream.write_all(http_response(status, "application/json", body).as_bytes())
}

// --------------------------------------------------------------- /lac/* -----

fn backend_pref_path() -> String {
    format!("{}/.lac/router-backend", common::home_dir())
}

fn load_persisted_backend() -> usize {
    std::fs::read_to_string(backend_pref_path())
        .ok()
        .map(|s| backend_id(s.trim()))
        .unwrap_or(BACKEND_AUTO)
}

fn persist_backend(id: usize) {
    let _ = common::atomic_write(&backend_pref_path(), backend_name(id));
}

fn handle_lac_status(
    mut stream: TcpStream,
    preferred: usize,
    hc: &HealthCache,
    stats: &StatsMap,
    routes: &RouteMap,
    started: Instant,
    inflight: usize,
) -> io::Result<()> {
    let mlx_port = port_mlx();
    let llama_port = port_llama();
    let ollama_port = port_ollama();
    let mlx_up = hc.ready(mlx_port);
    let llama_up = hc.ready(llama_port);
    let ollama_up = hc.ready(ollama_port);
    let order = pick_backends(preferred, hc, stats, None);
    let (active_id, active_port) = order
        .first()
        .copied()
        .unwrap_or((preferred, backend_port(preferred)));

    // Measured per-backend stats (zeros when nothing proxied yet).
    let mut stats_json = String::from("{");
    for (i, (bid, port)) in tier_backends().iter().enumerate() {
        let (ewma, ok, err, toks) = stats
            .lock()
            .ok()
            .and_then(|m| m.get(port).cloned())
            .map(|s| (s.ewma_ms, s.ok, s.err, s.est_tokens))
            .unwrap_or((None, 0, 0, 0));
        if i > 0 {
            stats_json.push(',');
        }
        stats_json.push_str(&format!(
            "\"{}\":{{\"port\":{},\"ewma_ms\":{},\"ok\":{},\"err\":{},\"est_tokens\":{}}}",
            backend_name(*bid),
            port,
            ewma.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "null".to_string()),
            ok,
            err,
            toks,
        ));
    }
    stats_json.push('}');
    let models_mapped = routes.lock().map(|r| r.len()).unwrap_or(0);

    let body = format!(
        concat!(
            "{{\n  \"status\": \"ok\",\n  \"router\": \"lac-router v2.7\",\n",
            "  \"preferred\": \"{}\",\n  \"active\": \"{}\",\n  \"target_port\": {},\n",
            "  \"uptime_secs\": {},\n  \"inflight\": {},\n  \"models_mapped\": {},\n",
            "  \"usage_log\": \"{}\",\n",
            "  \"backends\": {{\n",
            "    \"mlx\": {{ \"port\": {}, \"up\": {} }},\n",
            "    \"llama\": {{ \"port\": {}, \"up\": {} }},\n",
            "    \"ollama\": {{ \"port\": {}, \"up\": {} }}\n  }},\n",
            "  \"stats\": {}\n}}"
        ),
        backend_name(preferred),
        backend_name(active_id),
        active_port,
        started.elapsed().as_secs(),
        inflight,
        models_mapped,
        usage_log_path(),
        mlx_port,
        mlx_up,
        llama_port,
        llama_up,
        ollama_port,
        ollama_up,
        stats_json,
    );
    json_response(&mut stream, "200 OK", &body)
}

fn handle_lac_switch(
    mut stream: TcpStream,
    path: &str,
    preferred: &Arc<AtomicUsize>,
) -> io::Result<()> {
    let query = path.split('?').nth(1).unwrap_or("");
    let target_param = query
        .split('&')
        .filter_map(|kv| {
            let mut it = kv.splitn(2, '=');
            match (it.next(), it.next()) {
                (Some("target"), Some(v)) => Some(v),
                _ => None,
            }
        })
        .next()
        .unwrap_or("");
    // Back-compat: /lac/switch/mlx style paths still work.
    let new_target = if !target_param.is_empty() {
        backend_id(target_param)
    } else if path.contains("mlx") {
        BACKEND_MLX
    } else if path.contains("llama") {
        BACKEND_LLAMA
    } else if path.contains("ollama") {
        BACKEND_OLLAMA
    } else {
        BACKEND_AUTO
    };
    preferred.store(new_target, Ordering::SeqCst);
    persist_backend(new_target);
    let body = format!(
        "{{\n  \"status\": \"switched\",\n  \"target\": \"{}\",\n  \"port\": {},\n  \"persisted\": true\n}}",
        backend_name(new_target),
        backend_port(new_target)
    );
    json_response(&mut stream, "200 OK", &body)
}

fn handle_options(mut stream: TcpStream) -> io::Result<()> {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let resp = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    stream.write_all(resp.as_bytes())
}

// ---------------------------------------------------------- models merge ----

/// Locate the `"data": [ ... ]` array span (byte indices of brackets).
fn find_data_array(s: &str) -> Option<(usize, usize)> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' {
            // Possible string; check for the key "data".
            let mut j = i + 1;
            while j < b.len() && b[j] != b'"' {
                if b[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            if j < b.len() && &s[i + 1..j] == "data" {
                let mut k = j + 1;
                while k < b.len() && (b[k] as char).is_whitespace() {
                    k += 1;
                }
                // Skip ':' then whitespace, expect '['.
                if k < b.len() && b[k] == b':' {
                    k += 1;
                    while k < b.len() && (b[k] as char).is_whitespace() {
                        k += 1;
                    }
                    if k < b.len() && b[k] == b'[' {
                        let start = k;
                        let mut depth = 0i32;
                        let mut in_str = false;
                        let mut esc = false;
                        let mut m = k;
                        while m < b.len() {
                            let c = b[m];
                            if in_str {
                                if esc {
                                    esc = false;
                                } else if c == b'\\' {
                                    esc = true;
                                } else if c == b'"' {
                                    in_str = false;
                                }
                            } else if c == b'"' {
                                in_str = true;
                            } else if c == b'[' {
                                depth += 1;
                            } else if c == b']' {
                                depth -= 1;
                                if depth == 0 {
                                    return Some((start, m));
                                }
                            }
                            m += 1;
                        }
                        return None;
                    }
                }
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    None
}

/// Split top-level `{...}` items of an array body (without brackets).
fn split_items(inner: &str) -> Vec<String> {
    let b = inner.as_bytes();
    let mut items = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    let mut start: Option<usize> = None;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else if c == b'"' {
            in_str = true;
        } else if c == b'{' {
            if depth == 0 {
                start = Some(i);
            }
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
            if depth == 0 {
                if let Some(s) = start.take() {
                    items.push(inner[s..=i].to_string());
                }
            }
        }
        i += 1;
    }
    items
}

/// Extract the top-level `"id": "<v>"` from a model object.
/// Depth-aware and string-aware: an `"id"` nested in a sub-object or
/// embedded inside a string value can no longer shadow (and drop) the
/// real model id during merge dedup.
fn item_id(item: &str) -> Option<String> {
    let b = item.as_bytes();
    let mut i = 0;
    let mut depth = 0i32;
    while i < b.len() {
        match b[i] {
            b'"' => {
                let (token, next) = scan_string(b, i)?;
                // Lookahead: optional whitespace, then ':'.
                let mut k = next;
                while k < b.len() && (b[k] as char).is_whitespace() {
                    k += 1;
                }
                if depth == 1 && token == "id" && k < b.len() && b[k] == b':' {
                    k += 1;
                    while k < b.len() && (b[k] as char).is_whitespace() {
                        k += 1;
                    }
                    if k < b.len() && b[k] == b'"' {
                        let (val, _) = scan_string(b, k)?;
                        return Some(val);
                    }
                    return None;
                }
                i = next;
            }
            b'{' | b'[' => {
                depth += 1;
                i += 1;
            }
            b'}' | b']' => {
                depth -= 1;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    None
}

/// Scan a `"`-opened string starting at byte `i`; returns the unescaped
/// value and the index just past the closing quote.
fn scan_string(b: &[u8], i: usize) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut esc = false;
    let mut j = i + 1;
    while j < b.len() {
        let c = b[j];
        if esc {
            out.push(c as char);
            esc = false;
        } else if c == b'\\' {
            esc = true;
        } else if c == b'"' {
            return Some((out, j + 1));
        } else {
            out.push(c as char);
        }
        j += 1;
    }
    None
}

/// Aggregate `/v1/models` across live backends (MLX -> llama -> Ollama
/// priority, first id wins). Records id -> backend in the route map so
/// later requests naming a model can go straight there. Falls back to
/// plain forwarding on any parse failure so the endpoint never regresses.
fn merged_models(hc: &HealthCache, routes: &RouteMap) -> Option<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut items: Vec<String> = Vec::new();
    for (_, port) in tier_backends() {
        if !hc.ready(port) {
            continue;
        }
        let body = match common::http_get(port, "/v1/models", 3000) {
            Some((200, b)) => b,
            _ => continue,
        };
        let (s, e) = match find_data_array(&body) {
            Some(v) => v,
            None => continue,
        };
        for item in split_items(&body[s + 1..e]) {
            match item_id(&item) {
                Some(id) => {
                    if let Ok(mut r) = routes.lock() {
                        // Prune stale mappings while we hold the lock.
                        r.retain(|_, (_, t)| t.elapsed() < ROUTE_TTL);
                        r.insert(id.clone(), (port, Instant::now()));
                    }
                    if seen.insert(id) {
                        items.push(item);
                    }
                }
                None => items.push(item),
            }
        }
    }
    // No extractable models anywhere: fall through to single-backend
    // forwarding (which yields the real backend error/502) rather than
    // answering a misleading 200 with an empty catalog.
    if items.is_empty() {
        return None;
    }
    Some(format!(
        "{{\"object\":\"list\",\"data\":[{}]}}",
        items.join(",")
    ))
}

// -------------------------------------------------------------- forwarding --

enum ForwardError {
    /// TCP connect (or initial write) failed: safe to try the next backend.
    Connect,
    /// Stream broke mid-proxy: the response is already partial, do not retry.
    Stream,
}

/// Ordered candidate backends. Default policy is tier order
/// (MLX -> llama -> Ollama); `fastest` sorts ready backends by measured
/// EWMA instead. Backends that are down OR breaker-tripped are skipped.
/// A mapped model hint (AUTO/FASTEST only) goes first — it costs nothing
/// and avoids 404s on backends that never advertised that model.
/// Empty = nothing serving.
fn pick_backends(
    preferred: usize,
    hc: &HealthCache,
    stats: &StatsMap,
    model_hint: Option<u16>,
) -> Vec<(usize, u16)> {
    let usable = |port: u16| hc.ready(port) && !breaker_open_for(stats, port);
    let tier = tier_backends();

    if preferred == BACKEND_FASTEST {
        let mut c: Vec<(usize, u16)> = tier
            .iter()
            .copied()
            .filter(|(_, port)| usable(*port))
            .collect();
        c.sort_by(|a, b| {
            ewma_of(stats, a.1)
                .partial_cmp(&ewma_of(stats, b.1))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        // A mapped model still goes first: fastest-among-blind is no
        // excuse for 404ing on a backend that advertised the model
        // (forwarding treats 404 bodies as success — no failover saves it).
        if let Some(mp) = model_hint {
            if let Some(i) = c.iter().position(|(_, p)| *p == mp) {
                let h = c.remove(i);
                c.insert(0, h);
            }
        }
        return c;
    }

    let mut order: Vec<(usize, u16)> = Vec::new();
    if preferred != BACKEND_AUTO {
        let port = backend_port(preferred);
        if usable(port) {
            order.push((preferred, port));
        }
    } else if let Some(mp) = model_hint {
        // Mapped model first (AUTO only; an explicit pin always wins).
        if let Some((hid, _)) = tier.iter().find(|(_, p)| *p == mp) {
            let hid = *hid;
            if usable(mp) {
                order.push((hid, mp));
            }
        }
    }
    for (id, port) in &tier {
        if !order.iter().any(|(i, _)| *i == *id) && usable(*port) {
            order.push((*id, *port));
        }
    }
    order
}

/// Borrowed-client forward: on `Connect` failure the caller still owns
fn parse_content_length(headers: &[u8]) -> Option<usize> {
    let head = String::from_utf8_lossy(headers);
    for line in head.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        let mut parts = trimmed.splitn(2, ':');
        if let (Some(name), Some(val)) = (parts.next(), parts.next()) {
            if name.trim().eq_ignore_ascii_case("content-length") {
                if let Ok(len) = val.trim().parse::<usize>() {
                    return Some(len);
                }
            }
        }
    }
    None
}

/// Borrowed-client forward: on `Connect` failure the caller still owns
/// the client socket and may retry the next backend. Mid-stream
/// failures return `Stream` (response already partial — no retry).
/// Ok carries (request bytes, response bytes, first-byte latency).
fn try_forward(
    client: &TcpStream,
    target_port: u16,
    initial_bytes: &[u8],
) -> Result<(u64, u64, Option<f64>), ForwardError> {
    let t_start = Instant::now();
    let target_addr: SocketAddr = format!("127.0.0.1:{}", target_port)
        .parse()
        .map_err(|_| ForwardError::Connect)?;

    let mut server = match TcpStream::connect_timeout(&target_addr, Duration::from_secs(3)) {
        Ok(s) => s,
        Err(_) => return Err(ForwardError::Connect),
    };

    if server.write_all(initial_bytes).is_err() {
        return Err(ForwardError::Connect);
    }

    let _ = client.set_nodelay(true);
    let _ = server.set_nodelay(true);
    let _ = client.set_read_timeout(Some(STREAM_BUDGET));
    let _ = server.set_read_timeout(Some(Duration::from_secs(15)));
    let _ = client.set_write_timeout(Some(Duration::from_secs(60)));
    let _ = server.set_write_timeout(Some(Duration::from_secs(60)));

    let header_len = find_headers_end(initial_bytes).unwrap_or(initial_bytes.len());
    let body_in_initial = initial_bytes.len().saturating_sub(header_len);
    let content_len = parse_content_length(initial_bytes);

    let t1 = if let Some(cl) = content_len {
        if body_in_initial >= cl {
            // Whole body is already in initial_bytes and forwarded!
            // No client upload thread needed; request is already complete.
            None
        } else {
            let mut remaining = cl - body_in_initial;
            let mut client_read = client.try_clone().map_err(|_| ForwardError::Stream)?;
            let mut server_write = server.try_clone().map_err(|_| ForwardError::Stream)?;
            Some(thread::spawn(move || -> u64 {
                let mut buf = [0u8; 16384];
                let mut n_total = 0u64;
                while remaining > 0 {
                    let to_read = buf.len().min(remaining);
                    match client_read.read(&mut buf[..to_read]) {
                        Ok(0) => break,
                        Ok(n) => {
                            n_total += n as u64;
                            remaining = remaining.saturating_sub(n);
                            if server_write.write_all(&buf[..n]).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let _ = server_write.shutdown(std::net::Shutdown::Write);
                n_total
            }))
        }
    } else {
        let mut client_read = client.try_clone().map_err(|_| ForwardError::Stream)?;
        let mut server_write = server.try_clone().map_err(|_| ForwardError::Stream)?;
        Some(thread::spawn(move || -> u64 {
            let mut buf = [0u8; 16384];
            let mut n_total = 0u64;
            loop {
                match client_read.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        n_total += n as u64;
                        if server_write.write_all(&buf[..n]).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = server_write.shutdown(std::net::Shutdown::Write);
            n_total
        }))
    };

    let mut server_read = server;
    // Owned write handle; the borrowed `client` stays with the caller for
    // potential failover to the next backend.
    let mut client_write = client.try_clone().map_err(|_| ForwardError::Stream)?;
    let mut buf = [0u8; 16384];
    let mut down_total = 0u64;
    let mut ttfb: Option<f64> = None;
    loop {
        match server_read.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if ttfb.is_none() {
                    ttfb = Some(t_start.elapsed().as_secs_f64() * 1000.0);
                }
                down_total += n as u64;
                if client_write.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
            Err(e) if (e.kind() == io::ErrorKind::TimedOut || e.kind() == io::ErrorKind::WouldBlock) && t_start.elapsed() < STREAM_BUDGET => {
                // If response headers already delivered to client, emit SSE comment keep-alive to maintain transport
                if ttfb.is_some() {
                    let _ = client_write.write_all(b": keep-alive\n\n");
                }
                continue;
            }
            Err(_) => break,
        }
    }
    let _ = client_write.shutdown(std::net::Shutdown::Write);
    let _ = client.shutdown(std::net::Shutdown::Both);
    let up_extra = t1.map(|h| h.join().unwrap_or(0)).unwrap_or(0);
    Ok((
        initial_bytes.len() as u64 + up_extra,
        down_total,
        ttfb,
    ))
}

struct InflightGuard {
    count: Arc<AtomicUsize>,
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::SeqCst);
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_connection(
    client: TcpStream,
    preferred: Arc<AtomicUsize>,
    hc: Arc<HealthCache>,
    stats: StatsMap,
    routes: RouteMap,
    inflight: Arc<AtomicUsize>,
    rid: Arc<AtomicUsize>,
    started: Instant,
) {
    // Adopt the accept loop's reservation (see main); the guard releases
    // it on every return below — accounting stays exact under bursts.
    let _guard = InflightGuard {
        count: Arc::clone(&inflight),
    };
    let n = inflight.load(Ordering::SeqCst);
    let rid_n = rid.fetch_add(1, Ordering::SeqCst);

    let initial = match read_headers(&client) {
        Ok(b) => b,
        Err(_) => return,
    };
    let (method, path) = request_line(&initial);

    if method == "OPTIONS" {
        let _ = handle_options(client);
        return;
    }

    if path == "/lac/status" || path == "/lac/health" || path == "/v1/status" || path == "/v1/health" {
        let _ = handle_lac_status(
            client,
            preferred.load(Ordering::SeqCst),
            &hc,
            &stats,
            &routes,
            started,
            n,
        );
        return;
    }

    if path.starts_with("/lac/switch") {
        let _ = handle_lac_switch(client, &path, &preferred);
        return;
    }

    // Smart endpoint: aggregate model catalogs so OpenCode's picker sees
    // every live backend at once.
    if method == "GET" && (path == "/v1/models" || path.starts_with("/v1/models?")) {
        if let Some(body) = merged_models(&hc, &routes) {
            let mut c = client;
            let _ = json_response(&mut c, "200 OK", &body);
            eprintln!("[lac-router rid={}] GET /v1/models -> merged ({} bytes)", rid_n, body.len());
            return;
        }
        // else: fall through to single-backend forward.
    }

    let pref = preferred.load(Ordering::SeqCst);
    // Zero-cost model hint: only the bytes already buffered, only in
    // AUTO/FASTEST (an explicit pin always wins over a guess).
    let model = sniff_model(&initial).unwrap_or_default();
    let model_hint: Option<u16> = if pref == BACKEND_AUTO || pref == BACKEND_FASTEST {
        if model.is_empty() {
            None
        } else {
            routes.lock().ok().and_then(|r| {
                r.get(&model)
                    .filter(|(_, t)| t.elapsed() < ROUTE_TTL)
                    .map(|(p, _)| *p)
            })
        }
    } else {
        None
    };

    let candidates = pick_backends(pref, &hc, &stats, model_hint);
    if candidates.is_empty() {
        let err_body = format!(
            "{{\"error\":{{\"message\":\"No LAC inference backend is serving (MLX :{}, llama :{}, Ollama :{} all down or breaker-tripped). Start one with `lac serve mlx`.\",\"type\":\"lac_router_error\",\"code\":503}}}}",
            port_mlx(),
            port_llama(),
            port_ollama()
        );
        let mut c = client;
        let _ = json_response(&mut c, "503 Service Unavailable", &err_body);
        eprintln!("[lac-router rid={}] {} {} -> 503 no-backend", rid_n, method, path);
        return;
    }

    let header_len = find_headers_end(&initial).unwrap_or(initial.len());
    let t0 = Instant::now();
    let mut tried: Vec<u16> = Vec::new();
    for (bid, port) in candidates {
        match try_forward(&client, port, &initial) {
            Ok((up, down, ttfb)) => {
                let up_body = up.saturating_sub(header_len as u64);
                let down_body = down.saturating_sub(200);
                let est = (up_body + down_body) / 4;
                if let Some(ms) = ttfb {
                    note_success(&stats, port, ms, up_body, down_body);
                } else {
                    // Empty reply still counts as served, without latency data.
                    note_success(&stats, port, ewma_of(&stats, port).min(30_000.0), up_body, down_body);
                }
                log_usage(
                    rid_n,
                    backend_name(bid),
                    port,
                    &model,
                    up_body,
                    down_body,
                    est,
                    ttfb,
                    "proxied",
                );
                eprintln!(
                    "[lac-router rid={}] {} {} -> {} (:{} up={} down={} ttfb={} {:.1}s)",
                    rid_n,
                    method,
                    path,
                    backend_name(bid),
                    port,
                    up,
                    down,
                    ttfb.map(|v| format!("{:.0}ms", v)).unwrap_or_else(|| "-".to_string()),
                    t0.elapsed().as_secs_f64()
                );
                return;
            }
            Err(ForwardError::Connect) => {
                note_error(&stats, port);
                tried.push(port);
                continue;
            }
            Err(ForwardError::Stream) => {
                note_error(&stats, port);
                log_usage(rid_n, backend_name(bid), port, &model, 0, 0, 0, None, "stream-broke");
                eprintln!(
                    "[lac-router rid={}] {} {} -> :{} stream-broke ({:.1}s)",
                    rid_n,
                    method,
                    path,
                    port,
                    t0.elapsed().as_secs_f64()
                );
                return;
            }
        }
    }
    let err_body = format!(
        "{{\"error\":{{\"message\":\"All LAC backends refused the connection (preferred {}, tried {:?}). Start one with `lac serve mlx`.\",\"type\":\"lac_router_error\",\"code\":502}}}}",
        common::json_escape(backend_name(pref)),
        tried
    );
    let mut c = client;
    let _ = json_response(&mut c, "502 Bad Gateway", &err_body);
    eprintln!(
        "[lac-router rid={}] {} {} -> 502 all-refused ({:.1}s)",
        rid_n,
        method,
        path,
        t0.elapsed().as_secs_f64()
    );
}

fn main() {
    common::ignore_sigpipe();

    let port: u16 = env::var("LAC_ROUTER_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .or_else(|| env::args().nth(1).and_then(|p| p.parse().ok()))
        .unwrap_or(8000);

    let preferred = Arc::new(AtomicUsize::new(BACKEND_AUTO));
    // Precedence: explicit env > persisted file > auto.
    if let Ok(target) = env::var("LAC_BACKEND") {
        if !target.trim().is_empty() && target.to_lowercase() != "auto" {
            preferred.store(backend_id(&target), Ordering::SeqCst);
            persist_backend(backend_id(&target));
        }
    } else {
        preferred.store(load_persisted_backend(), Ordering::SeqCst);
    }

    let bind_addr = format!("127.0.0.1:{}", port);
    let listener = match TcpListener::bind(&bind_addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind lac-router to {}: {}", bind_addr, e);
            std::process::exit(1);
        }
    };

    let hc = Arc::new(HealthCache::new(Duration::from_secs(1)));
    let stats: StatsMap = Arc::new(Mutex::new(HashMap::new()));
    let routes: RouteMap = Arc::new(Mutex::new(HashMap::new()));
    let inflight = Arc::new(AtomicUsize::new(0));
    let rid = Arc::new(AtomicUsize::new(1));
    let started = Instant::now();

    eprintln!("========================================================");
    eprintln!("  LAC Unified Intelligent Router v2.7 (Rust native)");
    eprintln!("  Listening on http://{}", bind_addr);
    eprintln!(
        "  Proxying /v1/* -> MLX :{} | llama :{} | Ollama :{}",
        port_mlx(),
        port_llama(),
        port_ollama()
    );
    eprintln!(
        "  Preferred backend: {} (LAC_BACKEND env > ~/.lac/router-backend)",
        backend_name(preferred.load(Ordering::SeqCst))
    );
    eprintln!("  Health & metrics: http://{}/lac/status", bind_addr);
    eprintln!(
        "  Hot-swap backend: http://{}/lac/switch?target=[mlx|llama|ollama|auto|fastest]",
        bind_addr
    );
    eprintln!("  Usage log: {}", usage_log_path());
    eprintln!("========================================================");

    for stream in listener.incoming() {
        match stream {
            Ok(client) => {
                // Reserve synchronously in the single-threaded accept loop:
                // a burst of accepts can never slip past the cap (a load()
                // check here would race the spawned threads' increments).
                // The connection thread adopts this reservation and releases
                // it via InflightGuard on every exit path.
                let n = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                if n > MAX_INFLIGHT {
                    inflight.fetch_sub(1, Ordering::SeqCst);
                    let mut c = client;
                    let _ = json_response(
                        &mut c,
                        "503 Service Unavailable",
                        "{\"error\":{\"message\":\"router saturated (128 inflight)\",\"type\":\"lac_router_busy\",\"code\":503}}",
                    );
                    continue;
                }
                let pref = Arc::clone(&preferred);
                let hc = Arc::clone(&hc);
                let stats = Arc::clone(&stats);
                let routes = Arc::clone(&routes);
                let inflight = Arc::clone(&inflight);
                let rid = Arc::clone(&rid);
                thread::spawn(move || {
                    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        handle_connection(client, pref, hc, stats, routes, inflight, rid, started);
                    }));
                    if let Err(e) = res {
                        eprintln!("[lac-router] recovered safely from thread panic in connection handler: {:?}", e);
                    }
                });
            }
            Err(e) => {
                eprintln!("Incoming connection error: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_name_roundtrip() {
        assert_eq!(backend_name(BACKEND_MLX), "mlx");
        assert_eq!(backend_name(BACKEND_LLAMA), "llama");
        assert_eq!(backend_name(BACKEND_OLLAMA), "ollama");
        assert_eq!(backend_name(BACKEND_AUTO), "auto");
        assert_eq!(backend_id("mlx"), BACKEND_MLX);
        assert_eq!(backend_id("LLAMA"), BACKEND_LLAMA);
        assert_eq!(backend_id("bogus"), BACKEND_AUTO);
    }

    #[test]
    fn headers_end_detection() {
        let req = b"POST /v1/chat HTTP/1.1\r\nHost: x\r\n\r\n{}";
        assert_eq!(find_headers_end(req), Some(req.len() - 2));
        assert_eq!(find_headers_end(b"GET / HTTP/1.1\r\n"), None);
    }

    #[test]
    fn data_array_located_with_strings() {
        let body = r#"{"object":"list","note":"has [brackets] inside","data":[{"id":"a","x":"]"},{"id":"b"}]}"#;
        let (s, e) = find_data_array(body).expect("array found");
        assert!(body[s..=e].contains("\"id\":\"a\""));
        assert!(body[s..=e].contains("\"id\":\"b\""));
        assert!(!body[s..=e].contains("inside"));
    }

    #[test]
    fn items_split_and_id_extracted() {
        let inner = r#"{"id":"a","v":1},{"id":"b","nested":{"x":[1,2]}}"#;
        let items = split_items(inner);
        assert_eq!(items.len(), 2);
        assert_eq!(item_id(&items[0]).as_deref(), Some("a"));
        assert_eq!(item_id(&items[1]).as_deref(), Some("b"));
    }

    #[test]
    fn id_ignores_nested_and_string_values() {
        // "id" inside a string value and inside a nested object must not win.
        let item = r#"{"note":"the \"id\" is here","id":"real","meta":{"id":"inner"}}"#;
        assert_eq!(item_id(item).as_deref(), Some("real"));
        let no_top = r#"{"meta":{"id":"inner"}}"#;
        assert_eq!(item_id(no_top), None);
    }

    #[test]
    fn request_line_parsed() {
        let (m, p) = request_line(b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\n\r\n");
        assert_eq!(m, "POST");
        assert_eq!(p, "/v1/chat/completions");
    }

    #[test]
    fn breaker_matrix() {
        let now = Instant::now();
        assert!(!breaker_open(0, None, now));
        assert!(!breaker_open(2, Some(now), now));
        assert!(breaker_open(3, Some(now), now));
        assert!(!breaker_open(9, Some(now - Duration::from_secs(31)), now));
        assert!(!breaker_open(9, None, now));
    }

    #[test]
    fn sniff_extracts_model() {
        let req = b"POST /v1/chat HTTP/1.1\r\nHost: x\r\n\r\n{\"model\":\"qwen3.8-27b\",\"messages\":[]}";
        assert_eq!(sniff_model(req).as_deref(), Some("qwen3.8-27b"));
        assert_eq!(sniff_model(b"GET /v1/models HTTP/1.1\r\n\r\n"), None);
        assert_eq!(sniff_model(b"POST /x HTTP/1.1\r\n\r\n{\"model\": 42}"), None);
    }

    #[test]
    fn ewma_learns_and_breaker_trips() {
        let stats: StatsMap = Arc::new(Mutex::new(HashMap::new()));
        assert_eq!(ewma_of(&stats, 8080), EWMA_UNKNOWN_MS);
        note_success(&stats, 8080, 100.0, 400, 400);
        note_success(&stats, 8080, 200.0, 400, 400);
        assert!((ewma_of(&stats, 8080) - 130.0).abs() < 1e-6);
        note_error(&stats, 8080);
        note_error(&stats, 8080);
        assert!(!breaker_open_for(&stats, 8080));
        note_error(&stats, 8080);
        assert!(breaker_open_for(&stats, 8080));
        note_success(&stats, 8080, 50.0, 0, 0);
        assert!(!breaker_open_for(&stats, 8080));
    }

    #[test]
    fn parse_content_length_extracted() {
        let req = b"POST /v1/chat HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: 42\r\n\r\n{}";
        assert_eq!(parse_content_length(req), Some(42));
        let get = b"GET /v1/models HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert_eq!(parse_content_length(get), None);
    }
}
