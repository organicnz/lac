//! lac-router v2.8 — adaptive unified gateway on :8000.
//!
//! v2.8 over v2.7:
//! - Response framing: Content-Length early-close (no hold-open stall),
//!   chunked terminal detection, SSE-only keep-alives (never JSON).
//! - Remote Bearer gate: LAC_BIND_ADDR + LAC_API_TOKEN, fail-closed on
//!   non-loopback bind without a token, 401 for remote without Bearer.
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
const MAX_BODY: usize = 16 * 1024 * 1024;
const MAX_INFLIGHT: usize = 128;
const MAX_INTERIM_RESPONSES: usize = 8;

/// Keep-alive comments the router injects into a stalled SSE stream. Counted
/// in the bytes we forward so the usage log reflects what the client got.
const KEEPALIVE_CHUNK: &[u8] = b"E\r\n: keep-alive\n\n\r\n";
const KEEPALIVE_PLAIN: &[u8] = b": keep-alive\n\n";

/// Wall-clock budgets are env-overridable so tests can shrink them
/// (`LAC_ROUTER_STREAM_BUDGET_SECS=1`); out-of-range values fall back to
/// the production default rather than disabling a budget.
fn budget_secs(name: &str, default_secs: u64) -> Duration {
    Duration::from_secs(
        env::var(name)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|&secs| (1..=86_400).contains(&secs))
            .unwrap_or(default_secs),
    )
}

/// Wall clock one relayed response may take from the moment its request
/// was written. Note this is not a whole-connection budget: headers, the
/// request body and the response each get their own window, so one stalled
/// client connection can hold an inflight slot for several of these.
fn stream_budget() -> Duration {
    budget_secs("LAC_ROUTER_STREAM_BUDGET_SECS", 600)
}

/// How long a client may take to finish sending request headers.
fn header_budget() -> Duration {
    budget_secs("LAC_ROUTER_HEADER_BUDGET_SECS", 10)
}

/// Idle gap tolerated between request-body reads (slow-upload backstop).
fn body_idle_timeout() -> Duration {
    budget_secs("LAC_ROUTER_BODY_IDLE_SECS", 30)
}

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
    negative_ttl: Duration,
}

impl HealthCache {
    fn new(ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            ttl,
            negative_ttl: Duration::from_millis(2500),
        }
    }

    /// Cached HTTP readiness. One `/v1/models` probe per backend per TTL
    /// window instead of 3× TCP connects on every request.
    fn ready(&self, port: u16) -> bool {
        if let Ok(guard) = self.inner.lock() {
            if let Some((value, timestamp)) = guard.get(&port) {
                let window = if *value { self.ttl } else { self.negative_ttl };
                if timestamp.elapsed() < window {
                    return *value;
                }
            }
        }
        let value = common::http_ready(port, 1500);
        if let Ok(mut guard) = self.inner.lock() {
            guard.insert(port, (value, Instant::now()));
        }
        value
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
    let deadline = Instant::now() + header_budget();
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "incomplete request headers",
            ));
        }
        stream.set_read_timeout(Some(remaining))?;
        let n = stream.read(&mut chunk).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("incomplete request headers: {error}"),
            )
        })?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = find_headers_end(&buf) {
            if end > MAX_HEADERS {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "request headers exceed 64 KiB",
                ));
            }
            break;
        }
        if buf.len() > MAX_HEADERS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request headers exceed 64 KiB",
            ));
        }
    }
    if find_headers_end(&buf).is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incomplete request headers",
        ));
    }
    Ok(buf)
}

/// Best-effort pipelining check: drain whatever the client has *already*
/// queued behind the request we just framed. A second request that arrives
/// after this drain is not detected — the forwarded bytes are still cut at
/// the first request's framing, so the trailing request is never relayed
/// upstream (no smuggling), it is simply dropped when the connection ends.
fn has_queued_bytes(mut client: &TcpStream) -> bool {
    if client.set_nonblocking(true).is_err() {
        return false;
    }
    let mut buffer = [0u8; 4096];
    let mut queued = false;
    let mut drained = 0usize;
    loop {
        match client.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                queued = true;
                drained += n;
                if drained >= MAX_BODY {
                    break;
                }
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(_) => break,
        }
    }
    let reset = client.set_nonblocking(false).is_ok();
    queued || !reset
}

fn valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-'
                        | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
                )
        })
}

fn strict_header_lines(headers: &[u8]) -> Result<Vec<String>, ()> {
    for (index, byte) in headers.iter().enumerate() {
        if *byte == b'\n' && (index == 0 || headers[index - 1] != b'\r') {
            return Err(());
        }
        if *byte == b'\r'
            && (index + 1 >= headers.len() || headers[index + 1] != b'\n')
        {
            return Err(());
        }
    }
    let text = String::from_utf8_lossy(headers);
    Ok(text.split("\r\n").map(str::to_string).collect())
}

fn parse_header_line(line: &str) -> Result<(&str, &str), ()> {
    if line.is_empty() {
        return Err(());
    }
    if line
        .as_bytes()
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        return Err(());
    }
    let (name, value) = line.split_once(':').ok_or(())?;
    if !valid_header_name(name)
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() && byte != b'\t')
    {
        return Err(());
    }
    Ok((name, value))
}

fn parse_response_header_line(line: &str) -> Result<(&str, &str), ()> {
    if line.is_empty()
        || line
            .as_bytes()
            .first()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        return Err(());
    }
    let (name, value) = line.split_once(':').ok_or(())?;
    let name = name.trim_matches(|character| matches!(character, ' ' | '\t'));
    if !valid_header_name(name)
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() && byte != b'\t')
    {
        return Err(());
    }
    Ok((name, value))
}

fn transfer_encoding_tokens(headers: &[u8], response: bool) -> Result<Vec<String>, ()> {
    let lines = strict_header_lines(headers).map_err(|_| ())?;
    let mut tokens = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() {
            break;
        }
        if index == 0 {
            continue;
        }
        let (name, value) = if response {
            let (name, value) = parse_response_header_line(line).map_err(|_| ())?;
            if matches!(
                name.to_ascii_lowercase().as_str(),
                "content-length"
                    | "transfer-encoding"
                    | "content-type"
                    | "content-encoding"
                    | "trailer"
            ) {
                parse_header_line(line).map_err(|_| ())?
            } else {
                (name, value)
            }
        } else {
            parse_header_line(line).map_err(|_| ())?
        };
        if name.eq_ignore_ascii_case("transfer-encoding") {
            tokens.extend(value.split(',').map(|token| {
                token
                    .trim_matches(|character| matches!(character, ' ' | '\t'))
                    .to_string()
            }));
        }
    }
    if tokens
        .iter()
        .any(|token| token.is_empty() || !token.bytes().all(is_token_byte))
    {
        return Err(());
    }
    Ok(tokens)
}

fn chunked_result(tokens: Vec<String>) -> Result<bool, ()> {
    if tokens.is_empty() {
        return Ok(false);
    }
    if tokens.last().is_some_and(|token| token.eq_ignore_ascii_case("chunked"))
        && tokens.iter().filter(|token| token.eq_ignore_ascii_case("chunked")).count() == 1
    {
        return Ok(true);
    }
    Err(())
}

fn expect_continue(headers: &[u8]) -> bool {
    let Ok(lines) = strict_header_lines(headers) else {
        return false;
    };
    lines.iter().skip(1).any(|line| {
        if line.is_empty() {
            return false;
        }
        let Ok((name, value)) = parse_header_line(line) else {
            return false;
        };
        name.eq_ignore_ascii_case("expect")
            && value.split(',').any(|token| {
                token
                    .trim_matches(|character| matches!(character, ' ' | '\t'))
                    .eq_ignore_ascii_case("100-continue")
            })
    })
}

fn has_chunked_transfer_encoding(headers: &[u8]) -> Result<bool, ()> {
    chunked_result(transfer_encoding_tokens(headers, false)?)
}

fn has_chunked_response_transfer_encoding(headers: &[u8]) -> Result<bool, ()> {
    chunked_result(transfer_encoding_tokens(headers, true)?)
}

fn plain_chunked_transfer_encoding(headers: &[u8]) -> Result<Option<bool>, ()> {
    let tokens = transfer_encoding_tokens(headers, true)?;
    if tokens.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        tokens.len() == 1 && tokens[0].eq_ignore_ascii_case("chunked"),
    ))
}

/// Strip hop-by-hop credentials and normalize framing before forwarding:
/// gateway `Authorization` never reaches an inference backend, the
/// `100-continue` expectation is dropped because we already answered it
/// ourselves, and `Content-Length` is rewritten once in canonical decimal
/// so a duplicate pair can never reach a strict backend as-is.
fn sanitize_upstream_request(request: &[u8]) -> Vec<u8> {
    let Some(header_len) = find_headers_end(request) else {
        return request.to_vec();
    };
    let mut sanitized = Vec::with_capacity(request.len());
    let mut content_length_seen = false;
    for line in request[..header_len].split_inclusive(|byte| *byte == b'\n') {
        let trimmed = line.strip_suffix(b"\n").unwrap_or(line);
        let trimmed = trimmed.strip_suffix(b"\r").unwrap_or(trimmed);
        if trimmed.is_empty() {
            sanitized.extend_from_slice(b"\r\n");
            continue;
        }
        if let Some(colon) = trimmed.iter().position(|byte| *byte == b':') {
            let name = String::from_utf8_lossy(&trimmed[..colon]);
            let name = name.trim();
            if name.eq_ignore_ascii_case("authorization")
                || name.eq_ignore_ascii_case("proxy-authorization")
            {
                continue;
            }
            let value = String::from_utf8_lossy(&trimmed[colon + 1..]);
            if name.eq_ignore_ascii_case("expect")
                && value
                    .trim_matches(|character| matches!(character, ' ' | '\t'))
                    .eq_ignore_ascii_case("100-continue")
            {
                continue;
            }
            if name.eq_ignore_ascii_case("content-length") {
                if let Ok(parsed) = value
                    .trim_matches(|character| matches!(character, ' ' | '\t'))
                    .parse::<usize>()
                {
                    if content_length_seen {
                        continue;
                    }
                    content_length_seen = true;
                    let canonical = format!("Content-Length: {parsed}\r\n");
                    sanitized.extend_from_slice(canonical.as_bytes());
                    continue;
                }
            }
        }
        sanitized.extend_from_slice(line);
    }
    sanitized.extend_from_slice(&request[header_len..]);
    sanitized
}

enum BodyError {
    TooLarge,
    Invalid,
}

struct RequestFrame {
    wire: Vec<u8>,
    has_following: bool,
}

/// Body-read budgets, resolved once per request so a 16 MiB upload does
/// not re-read the environment on every 16 KiB read.
#[derive(Clone, Copy)]
struct RequestBudgets {
    stream: Duration,
    idle: Duration,
}

fn read_body_bytes(
    mut client: &TcpStream,
    buffer: &mut [u8],
    started: Instant,
    budgets: RequestBudgets,
) -> Result<usize, BodyError> {
    let remaining = budgets
        .stream
        .saturating_sub(started.elapsed())
        .min(budgets.idle);
    if remaining.is_zero() {
        return Err(BodyError::Invalid);
    }
    client
        .set_read_timeout(Some(remaining))
        .map_err(|_| BodyError::Invalid)?;
    client.read(buffer).map_err(|_| BodyError::Invalid)
}

fn read_request_body(
    client: &TcpStream,
    initial: &[u8],
) -> Result<RequestFrame, BodyError> {
    let header_len = find_headers_end(initial).ok_or(BodyError::Invalid)?;
    let budgets = RequestBudgets {
        stream: stream_budget(),
        idle: body_idle_timeout(),
    };
    let mut request = initial.to_vec();
    let mut buffer = [0u8; 16384];
    let body_started = Instant::now();
    if let Some(content_len) =
        parse_content_length_strict(&initial[..header_len]).map_err(|_| BodyError::Invalid)?
    {
        let target = header_len.checked_add(content_len).ok_or(BodyError::Invalid)?;
        if target > header_len.saturating_add(MAX_BODY) {
            return Err(BodyError::TooLarge);
        }
        while request.len() < target {
            let remaining = target - request.len();
            let to_read = buffer.len().min(remaining);
            let n = read_body_bytes(client, &mut buffer[..to_read], body_started, budgets)?;
            if n == 0 {
                return Err(BodyError::Invalid);
            }
            request.extend_from_slice(&buffer[..n]);
        }
        let mut has_following = request.len() > target;
        if !has_following {
            has_following = has_queued_bytes(client);
        }
        request.truncate(target);
        return Ok(RequestFrame {
            wire: request,
            has_following,
        });
    }
    if has_chunked_transfer_encoding(&initial[..header_len]).map_err(|_| BodyError::Invalid)? {
        let max_request = header_len.saturating_add(MAX_BODY + MAX_HEADERS);
        if request.len() > max_request {
            return Err(BodyError::TooLarge);
        }
        let mut detector = ChunkedDetector::default();
        let mut fed = header_len;
        loop {
            match detector.feed(&request[fed..]).map_err(|_| BodyError::Invalid)? {
                ChunkProgress::Complete { consumed } => {
                    let end = fed + consumed;
                    let mut has_following = end < request.len();
                    if !has_following {
                        has_following = has_queued_bytes(client);
                    }
                    return Ok(RequestFrame {
                        wire: request[..end].to_vec(),
                        has_following,
                    });
                }
                ChunkProgress::NeedMore => {
                    let n = read_body_bytes(client, &mut buffer, body_started, budgets)?;
                    if n == 0 {
                        return Err(BodyError::Invalid);
                    }
                    fed = request.len();
                    request.extend_from_slice(&buffer[..n]);
                    if request.len() > max_request {
                        return Err(BodyError::TooLarge);
                    }
                }
            }
        }
    }
    let mut has_following = request.len() > header_len;
    if !has_following {
        has_following = has_queued_bytes(client);
    }
    Ok(RequestFrame {
        wire: request[..header_len].to_vec(),
        has_following,
    })
}

fn valid_request_line(line: &str) -> bool {
    let bytes = line.as_bytes();
    let Some(first_space) = bytes.iter().position(|byte| *byte == b' ') else {
        return false;
    };
    let Some(second_space) = bytes[first_space + 1..]
        .iter()
        .position(|byte| *byte == b' ')
        .map(|index| index + first_space + 1)
    else {
        return false;
    };
    let method = &line[..first_space];
    let target = &line[first_space + 1..second_space];
    let version = &line[second_space + 1..];
    valid_header_name(method)
        && !target.is_empty()
        && target
            .bytes()
            .all(|byte| (0x21..=0x7e).contains(&byte))
        && matches!(version, "HTTP/1.0" | "HTTP/1.1")
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
            "{{\n  \"status\": \"ok\",\n  \"router\": \"lac-router v2.8\",\n",
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
        common::json_escape(&usage_log_path()),
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
    /// Backend returned a retryable HTTP response before headers were forwarded.
    HttpStatus { status: u16, ttfb: Option<f64> },
    /// Stream broke mid-proxy: the response is already partial, do not retry.
    Stream { down: u64, ttfb: Option<f64> },
    /// An EOF-delimited response (no Content-Length, no chunking) was cut
    /// short by a connection reset. No promised frame was violated, so it
    /// is reported separately from a truncated frame — but the upstream is
    /// loopback, so the peer process went away and this is charged.
    StreamCut { down: u64, ttfb: Option<f64> },
    BadResponse { down: u64, ttfb: Option<f64> },
    ClientGone { down: u64, ttfb: Option<f64> },
    /// Backend requested a protocol upgrade that this HTTP relay does not support.
    UnsupportedUpgrade { up: u64, ttfb: Option<f64> },
}

/// How a response that was cut short is reported. Both are charged: the
/// upstream hop is always loopback, so a reset is the backend process
/// going away mid-request — not a network event — and it predicts the next
/// request exactly as a hang does. The split is kept for the usage log,
/// where "ended at a boundary the backend never delimited" and "truncated
/// inside a frame it did promise" are different bugs to chase.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StreamEnd {
    /// Connection reset mid-body: the peer process went away.
    Reset,
    /// Silent until the stream budget ran out: the backend is not talking.
    Hung,
}

/// Split the read errors that end a response. A timeout reaching this point
/// has already outlived the budget (the keepalive arm handles the rest).
fn classify_read_error(kind: io::ErrorKind) -> StreamEnd {
    match kind {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => StreamEnd::Hung,
        _ => StreamEnd::Reset,
    }
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
        // A mapped model still goes first: a backend that 404s on a model it
        // never advertised is a routing miss, and a miss we can retry
        // somewhere else rather than answer the caller with.
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

fn parse_content_length_strict(headers: &[u8]) -> Result<Option<usize>, ()> {
    let lines = strict_header_lines(headers)?;
    let mut value = None;
    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() {
            break;
        }
        if index == 0 {
            continue;
        }
        let (name, raw) = parse_header_line(line)?;
        if name.eq_ignore_ascii_case("content-length") {
            let digits = raw.trim_matches(|character| matches!(character, ' ' | '\t'));
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(());
            }
            let parsed = digits.parse::<usize>().map_err(|_| ())?;
            if value.is_some_and(|existing| existing != parsed) {
                return Err(());
            }
            value = Some(parsed);
        }
    }
    Ok(value)
}

#[cfg(test)]
fn parse_content_length(headers: &[u8]) -> Option<usize> {
    parse_content_length_strict(headers).ok().flatten()
}

/// Remote auth: tailnet-only Bearer gate (fail closed).
/// Loopback stays token-less for local dev; any non-loopback peer must
/// present `Authorization: Bearer <LAC_API_TOKEN>`, compared constant-time.
fn bind_host() -> String {
    env::var("LAC_BIND_ADDR")
        .or_else(|_| env::var("LAC_ROUTER_HOST"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string())
}

fn api_token() -> String {
    env::var("LAC_API_TOKEN")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && !s.chars().any(|c| c.is_control()))
        .unwrap_or_default()
}

fn bind_address(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{}]:{}", host, port)
    } else {
        format!("{}:{}", host, port)
    }
}

fn is_loopback_host(h: &str) -> bool {
    let t = h.trim().to_lowercase();
    t == "127.0.0.1" || t == "localhost" || t == "::1"
}

fn is_loopback_peer(addr: &SocketAddr) -> bool {
    addr.ip().is_loopback()
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    if ab.len() != bb.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for i in 0..ab.len() {
        diff |= ab[i] ^ bb[i];
    }
    diff == 0
}

/// Extract `Bearer <token>` from raw request headers (case-insensitive).
fn bearer_from(headers: &[u8]) -> Option<String> {
    let head = String::from_utf8_lossy(headers);
    for line in head.lines() {
        let t = line.trim();
        if t.is_empty() {
            break;
        }
        if let Some((name, val)) = t.split_once(':') {
            if name.trim().eq_ignore_ascii_case("authorization") {
                let v = val.trim();
                let (scheme, token) = v.split_once(char::is_whitespace)?;
                if !scheme.eq_ignore_ascii_case("bearer") {
                    return None;
                }
                return Some(token.trim().to_string()).filter(|s| !s.is_empty());
            }
        }
    }
    None
}

fn forwarded_request(headers: &[u8]) -> bool {
    let head = String::from_utf8_lossy(headers);
    head.lines().take_while(|line| !line.trim().is_empty()).any(|line| {
        line.split_once(':').is_some_and(|(name, _)| {
            matches!(
                name.trim().to_ascii_lowercase().as_str(),
                "forwarded" | "x-forwarded-for" | "x-forwarded-proto" | "x-real-ip"
            )
        })
    })
}

fn unauthorized(stream: &mut TcpStream) -> io::Result<()> {
    json_response(
        stream,
        "401 Unauthorized",
        "{\"error\":{\"message\":\"missing or invalid Bearer token (set LAC_API_TOKEN)\",\"type\":\"lac_router_auth\",\"code\":401}}",
    )
}
/// Keep-alive tick for SSE streams. Env-overridable for tests
/// (`LAC_ROUTER_KEEPALIVE_SECS=1`); defaults to 15s in prod.
fn keepalive_secs() -> u64 {
    env::var("LAC_ROUTER_KEEPALIVE_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&n| (1..=60).contains(&n))
        .unwrap_or(15)
}

fn response_status(headers: &[u8]) -> Option<u16> {
    let line_end = headers
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(headers.len());
    let line = headers[..line_end].strip_suffix(b"\r").unwrap_or(&headers[..line_end]);
    let prefix_len = if line.starts_with(b"HTTP/1.1 ") || line.starts_with(b"HTTP/1.0 ") {
        9
    } else {
        return None;
    };
    if line.len() < prefix_len + 3 {
        return None;
    }
    let code = &line[prefix_len..prefix_len + 3];
    if !code.iter().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if line.len() > prefix_len + 3 {
        if line[prefix_len + 3] != b' '
            || line[prefix_len + 4..]
                .iter()
                .any(|byte| (*byte < 0x20 && *byte != b'\t') || *byte == 0x7f)
        {
            return None;
        }
    }
    std::str::from_utf8(code).ok()?.parse().ok()
}

/// Parse response framing: (content-length, is_chunked, is_sse, sse_keepalive).
fn parse_resp_headers(head: &[u8]) -> Result<(Option<usize>, bool, bool, bool), ()> {
    let lines = strict_header_lines(head).map_err(|_| ())?;
    let mut cl: Option<usize> = None;
    let mut sse = false;
    let mut content_type: Option<String> = None;
    let mut content_encoding_identity = true;
    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() {
            break;
        }
        if index == 0 {
            continue;
        }
        let (name, val) = parse_response_header_line(line).map_err(|_| ())?;
        let (name, val) = if matches!(
            name.to_ascii_lowercase().as_str(),
            "content-length"
                | "transfer-encoding"
                | "content-type"
                | "content-encoding"
                | "trailer"
        ) {
            parse_header_line(line).map_err(|_| ())?
        } else {
            (name, val)
        };
        match name.to_ascii_lowercase().as_str() {
            "content-length" => {
                let digits = val.trim_matches(|character| matches!(character, ' ' | '\t'));
                if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(());
                }
                let n = digits.parse::<usize>().map_err(|_| ())?;
                if cl.is_some_and(|existing| existing != n) {
                    return Err(());
                }
                cl = Some(n);
            }
            "content-type" => {
                let normalized = val.trim_matches(|character| matches!(character, ' ' | '\t'));
                if content_type
                    .as_ref()
                    .is_some_and(|existing| existing != normalized)
                {
                    return Err(());
                }
                content_type = Some(normalized.to_string());
                if normalized
                    .split(';')
                    .next()
                    .map(str::trim)
                    .is_some_and(|media| media.eq_ignore_ascii_case("text/event-stream"))
                {
                    sse = true;
                }
            }
            "content-encoding" => {
                let encoding = val.trim_matches(|character| matches!(character, ' ' | '\t'));
                if !encoding.is_empty() && !encoding.eq_ignore_ascii_case("identity") {
                    content_encoding_identity = false;
                }
            }
            _ => {}
        }
    }
    let plain_chunked = plain_chunked_transfer_encoding(head).map_err(|_| ())?;
    let sse_keepalive = sse && content_encoding_identity && plain_chunked != Some(false);
    let chunked = has_chunked_response_transfer_encoding(head).map_err(|_| ())?;
    if chunked && cl.is_some() {
        return Err(());
    }
    Ok((cl, chunked, sse, sse_keepalive))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChunkState {
    Size,
    Data,
    DataEnd,
    Trailers,
    Done,
    Invalid,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ChunkProgress {
    NeedMore,
    Complete { consumed: usize },
}

fn update_payload_tail(tail: &mut [u8; 4], len: &mut usize, data: &[u8]) {
    if data.is_empty() {
        return;
    }
    if data.len() >= tail.len() {
        let tail_len = tail.len();
        tail.copy_from_slice(&data[data.len() - tail_len..]);
        *len = tail_len;
        return;
    }
    let overflow = (*len + data.len()).saturating_sub(tail.len());
    tail.copy_within(overflow..*len, 0);
    *len -= overflow;
    tail[*len..*len + data.len()].copy_from_slice(data);
    *len += data.len();
}

fn payload_at_boundary(tail: &[u8; 4], len: usize) -> bool {
    debug_assert!(len <= tail.len());
    if len == 0 {
        return true;
    }
    if len < 2 {
        return false;
    }
    let last = tail[len - 1];
    let previous = tail[len - 2];
    if last == b'\n' && previous == b'\n' {
        return true;
    }
    if last == b'\r' && previous == b'\r' {
        return true;
    }
    len >= 3 && last == b'\n' && previous == b'\r' && tail[len - 3] == b'\n'
}

struct ChunkedDetector {
    state: ChunkState,
    line: Vec<u8>,
    remaining: u64,
    control_bytes: usize,
    payload_tail: [u8; 4],
    payload_len: usize,
    track_payload: bool,
}

impl Default for ChunkedDetector {
    fn default() -> Self {
        Self {
            state: ChunkState::Size,
            line: Vec::with_capacity(32),
            remaining: 0,
            control_bytes: 0,
            payload_tail: [0; 4],
            payload_len: 0,
            track_payload: false,
        }
    }
}

impl ChunkedDetector {
    fn can_inject_keepalive(&self) -> bool {
        self.state == ChunkState::Size && self.line.is_empty()
    }

    fn payload_at_sse_boundary(&self) -> bool {
        self.track_payload && payload_at_boundary(&self.payload_tail, self.payload_len)
    }

    fn note_payload(&mut self, data: &[u8]) {
        if self.track_payload {
            update_payload_tail(&mut self.payload_tail, &mut self.payload_len, data);
        }
    }

    fn feed(&mut self, data: &[u8]) -> Result<ChunkProgress, ()> {
        let mut i = 0;
        while i < data.len() {
            match self.state {
                ChunkState::Size | ChunkState::DataEnd | ChunkState::Trailers => {
                    if self.line.last() == Some(&b'\r') {
                        if data[i] != b'\n' {
                            self.state = ChunkState::Invalid;
                            return Err(());
                        }
                        let line = std::mem::replace(&mut self.line, Vec::with_capacity(32));
                        let state = self.state;
                        i += 1;
                        match state {
                            ChunkState::Size => match parse_size_line(&line) {
                                Some(size) if size > 0 => {
                                    self.remaining = size;
                                    self.state = ChunkState::Data;
                                }
                                Some(_) => {
                                    self.control_bytes = 0;
                                    self.state = ChunkState::Trailers;
                                }
                                None => {
                                    self.state = ChunkState::Invalid;
                                    return Err(());
                                }
                            },
                            ChunkState::DataEnd if line == b"\r" => self.state = ChunkState::Size,
                            ChunkState::DataEnd => {
                                self.state = ChunkState::Invalid;
                                return Err(());
                            }
                            ChunkState::Trailers if line == b"\r" => {
                                self.state = ChunkState::Done;
                                return Ok(ChunkProgress::Complete { consumed: i });
                            }
                            ChunkState::Trailers => {
                                let line = line.strip_suffix(b"\r").unwrap_or(&line);
                                if !valid_trailer_line(line) {
                                    self.state = ChunkState::Invalid;
                                    return Err(());
                                }
                            }
                            _ => unreachable!(),
                        }
                    } else if data[i] == b'\n' {
                        self.state = ChunkState::Invalid;
                        return Err(());
                    } else {
                        if self.state == ChunkState::Trailers {
                            if self.control_bytes >= MAX_HEADERS {
                                self.state = ChunkState::Invalid;
                                return Err(());
                            }
                            self.control_bytes += 1;
                        } else if self.line.len() >= MAX_HEADERS {
                            self.state = ChunkState::Invalid;
                            return Err(());
                        }
                        self.line.push(data[i]);
                        i += 1;
                    }
                }
                ChunkState::Data => {
                    let take = self.remaining.min((data.len() - i) as u64) as usize;
                    self.note_payload(&data[i..i + take]);
                    i += take;
                    self.remaining -= take as u64;
                    if self.remaining == 0 {
                        self.state = ChunkState::DataEnd;
                    }
                }
                ChunkState::Done => return Ok(ChunkProgress::Complete { consumed: i }),
                ChunkState::Invalid => return Err(()),
            }
        }
        if self.state == ChunkState::Done {
            Ok(ChunkProgress::Complete { consumed: i })
        } else {
            Ok(ChunkProgress::NeedMore)
        }
    }
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.'
                | b'^' | b'_' | b'`' | b'|' | b'~'
        )
}

fn skip_bws(mut data: &[u8]) -> &[u8] {
    while data.first().is_some_and(|byte| matches!(byte, b' ' | b'\t')) {
        data = &data[1..];
    }
    data
}

fn trim_bws(mut data: &[u8]) -> &[u8] {
    while data.last().is_some_and(|byte| matches!(byte, b' ' | b'\t')) {
        data = &data[..data.len() - 1];
    }
    data
}

fn consume_quoted_value(mut data: &[u8]) -> Option<&[u8]> {
    if data.first() != Some(&b'"') {
        return None;
    }
    data = &data[1..];
    let mut index = 0;
    while index < data.len() {
        match data[index] {
            b'"' => return Some(&data[index + 1..]),
            b'\\' => {
                if index + 1 >= data.len()
                    || !matches!(data[index + 1], b'\t' | b' ' | 0x21..=0x7e | 0x80..=0xff)
                {
                    return None;
                }
                index += 2;
            }
            b'\t' | b' ' | 0x21..=0x7e | 0x80..=0xff => index += 1,
            _ => return None,
        }
    }
    None
}

fn valid_chunk_extensions(data: &[u8]) -> bool {
    let mut data = skip_bws(data);
    let mut found = false;
    loop {
        data = skip_bws(data);
        if data.is_empty() {
            return found;
        }
        if data[0] != b';' {
            return false;
        }
        data = skip_bws(&data[1..]);
        let name_end = data.iter().position(|byte| !is_token_byte(*byte)).unwrap_or(data.len());
        if name_end == 0 {
            return false;
        }
        data = &data[name_end..];
        data = skip_bws(data);
        if data.first() == Some(&b'=') {
            data = skip_bws(&data[1..]);
            if data.first() == Some(&b'"') {
                let Some(remaining) = consume_quoted_value(data) else {
                    return false;
                };
                data = remaining;
            } else {
                let value_end = data
                    .iter()
                    .position(|byte| !is_token_byte(*byte))
                    .unwrap_or(data.len());
                if value_end == 0 {
                    return false;
                }
                data = &data[value_end..];
            }
            data = skip_bws(data);
        }
        if !data.is_empty() && data[0] != b';' {
            return false;
        }
        found = true;
    }
}

fn valid_trailer_line(line: &[u8]) -> bool {
    let Some(colon) = line.iter().position(|byte| *byte == b':') else {
        return false;
    };
    let name = match std::str::from_utf8(&line[..colon]) {
        Ok(name) => name,
        Err(_) => return false,
    };
    if !valid_header_name(name)
        || matches!(
            name.to_ascii_lowercase().as_str(),
            "authorization"
                | "proxy-authorization"
                | "content-length"
                | "transfer-encoding"
                | "trailer"
                | "host"
                | "cookie"
                | "set-cookie"
                | "content-encoding"
                | "content-type"
                | "content-range"
                | "connection"
                | "upgrade"
        )
    {
        return false;
    }
    line[colon + 1..]
        .iter()
        .all(|byte| !byte.is_ascii_control() || *byte == b'\t')
}

fn parse_size_line(line: &[u8]) -> Option<u64> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let (size, extensions) = match line.iter().position(|byte| *byte == b';') {
        Some(separator) => (
            trim_bws(&line[..separator]),
            &line[separator..],
        ),
        None => (line, &[][..]),
    };
    if size.is_empty() || !size.iter().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    if !extensions.is_empty() && !valid_chunk_extensions(extensions) {
        return None;
    }
    u64::from_str_radix(&String::from_utf8_lossy(size), 16).ok()
}

#[cfg(test)]
fn chunk_terminal_detected(chunks: &[&[u8]]) -> bool {
    let mut detector = ChunkedDetector::default();
    for chunk in chunks {
        if matches!(detector.feed(chunk), Ok(ChunkProgress::Complete { .. })) {
            return true;
        }
    }
    false
}
fn forward_response_bytes(
    data: &[u8],
    content_length: Option<usize>,
    chunked: bool,
    body_written: &mut usize,
    detector: &mut ChunkedDetector,
) -> Result<(usize, bool), ()> {
    if let Some(length) = content_length {
        let take = data.len().min(length.saturating_sub(*body_written));
        *body_written += take;
        return Ok((take, *body_written >= length));
    }
    if chunked {
        return match detector.feed(data).map_err(|_| ())? {
            ChunkProgress::Complete { consumed } => Ok((consumed, true)),
            ChunkProgress::NeedMore => Ok((data.len(), false)),
        };
    }
    Ok((data.len(), false))
}

fn try_forward(
    client: &TcpStream,
    target_port: u16,
    initial_bytes: &[u8],
    head_request: bool,
) -> Result<(u64, u64, Option<f64>), ForwardError> {
    let t_start = Instant::now();
    let budget = stream_budget();
    let target_addr: SocketAddr = format!("127.0.0.1:{}", target_port)
        .parse()
        .map_err(|_| ForwardError::Connect)?;

    let mut server = match TcpStream::connect_timeout(&target_addr, Duration::from_secs(3)) {
        Ok(s) => s,
        Err(_) => return Err(ForwardError::Connect),
    };

    let _ = server.set_write_timeout(Some(Duration::from_secs(60)));
    if server.write_all(initial_bytes).is_err() {
        return Err(ForwardError::Connect);
    }

    let _ = client.set_nodelay(true);
    let _ = server.set_nodelay(true);
    let _ = client.set_read_timeout(Some(budget));
    let ka = keepalive_secs();
    let _ = server.set_read_timeout(Some(Duration::from_secs(ka)));
    let _ = client.set_write_timeout(Some(Duration::from_secs(60)));
    let _ = server.set_write_timeout(Some(Duration::from_secs(60)));

    let _ = server.shutdown(std::net::Shutdown::Write);

    let mut down_total = 0u64;
    let mut ttfb: Option<f64> = None;
    let mut server_read = server;
    let mut client_write = client
        .try_clone()
        .map_err(|_| ForwardError::ClientGone { down: 0, ttfb: None })?;
    let mut buf = [0u8; 16384];
    let mut head_buf: Vec<u8> = Vec::with_capacity(4096);
    let mut headers_done = false;
    let mut resp_cl: Option<usize> = None;
    let mut resp_chunked = false;
    let mut resp_sse = false;
    let mut sse_tail = [0u8; 4];
    let mut sse_tail_len = 0usize;
    let mut resp_body: usize = 0;
    let mut chunk_detector = ChunkedDetector::default();
    let mut write_failed = false;
    let mut stream_end: Option<StreamEnd> = None;
    let mut interim_responses = 0usize;

    'response: loop {
        match server_read.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let chunk = &buf[..n];
                if !headers_done {
                    head_buf.extend_from_slice(chunk);
                    loop {
                        let Some(h_end) = find_headers_end(&head_buf) else {
                            if head_buf.len() > MAX_HEADERS {
                                return Err(ForwardError::BadResponse { down: down_total, ttfb });
                            }
                            break;
                        };
                        if h_end > MAX_HEADERS {
                            return Err(ForwardError::BadResponse { down: down_total, ttfb });
                        }
                        let (cl, chunked, _sse, sse_keepalive) =
                            match parse_resp_headers(&head_buf[..h_end]) {
                            Ok(framing) => framing,
                            Err(_) => {
                                return Err(ForwardError::BadResponse { down: down_total, ttfb })
                            }
                        };
                        let Some(status) = response_status(&head_buf[..h_end]) else {
                            return Err(ForwardError::BadResponse { down: down_total, ttfb });
                        };
                        if !(100..200).contains(&status) && ttfb.is_none() {
                            ttfb = Some(t_start.elapsed().as_secs_f64() * 1000.0);
                        }
                        if status == 101 {
                            return Err(ForwardError::UnsupportedUpgrade {
                                up: initial_bytes.len() as u64,
                                ttfb,
                            });
                        }
                        if (100..200).contains(&status) {
                            interim_responses += 1;
                            if interim_responses > MAX_INTERIM_RESPONSES {
                                return Err(ForwardError::BadResponse { down: down_total, ttfb });
                            }
                            head_buf.drain(..h_end);
                            if head_buf.is_empty() {
                                break;
                            }
                            continue;
                        }
                        if status == 404 || status >= 500 {
                            return Err(ForwardError::HttpStatus { status, ttfb });
                        }
                        let bodyless_status = matches!(status, 204 | 205 | 304);
                        if (matches!(status, 204 | 205 | 304) && chunked)
                            || (matches!(status, 204 | 205)
                                && cl.is_some_and(|length| length > 0))
                        {
                            return Err(ForwardError::BadResponse { down: down_total, ttfb });
                        }
                        let bodyless = head_request || bodyless_status;
                        resp_cl = if bodyless { Some(0) } else { cl };
                        resp_chunked = if bodyless { false } else { chunked };
                        resp_sse = sse_keepalive && !bodyless_status && !head_request;
                        chunk_detector.track_payload = resp_sse && resp_chunked;
                        headers_done = true;
                        if client_write.write_all(&head_buf[..h_end]).is_err() {
                            write_failed = true;
                            break 'response;
                        }
                        let body = &head_buf[h_end..];
                        let (body_len, complete) = forward_response_bytes(
                            body,
                            resp_cl,
                            resp_chunked,
                            &mut resp_body,
                            &mut chunk_detector,
                        )
                        .map_err(|_| ForwardError::Stream { down: down_total, ttfb })?;
                        if body_len > 0 && client_write.write_all(&body[..body_len]).is_err() {
                            write_failed = true;
                            break 'response;
                        }
                        down_total += body_len as u64;
                        if complete {
                            break 'response;
                        }
                        if resp_sse && !resp_chunked && body_len > 0 {
                            update_payload_tail(&mut sse_tail, &mut sse_tail_len, &body[..body_len]);
                        }
                        break;
                    }
                } else {
                    let (body_len, complete) = forward_response_bytes(
                        chunk,
                        resp_cl,
                        resp_chunked,
                        &mut resp_body,
                        &mut chunk_detector,
                    )
                    .map_err(|_| ForwardError::Stream { down: down_total, ttfb })?;
                    if body_len > 0 && client_write.write_all(&chunk[..body_len]).is_err() {
                        write_failed = true;
                        break 'response;
                    }
                    down_total += body_len as u64;
                    if complete {
                        break 'response;
                    }
                    if resp_sse && !resp_chunked && body_len > 0 {
                        update_payload_tail(&mut sse_tail, &mut sse_tail_len, &chunk[..body_len]);
                    }
                }
            }
            Err(e)
                if (e.kind() == io::ErrorKind::TimedOut
                    || e.kind() == io::ErrorKind::WouldBlock)
                    && t_start.elapsed() < budget =>
            {
                if headers_done && resp_sse {
                    if resp_chunked {
                        if chunk_detector.can_inject_keepalive()
                            && chunk_detector.payload_at_sse_boundary()
                        {
                            if client_write.write_all(KEEPALIVE_CHUNK).is_err() {
                                write_failed = true;
                                break;
                            }
                            down_total += KEEPALIVE_CHUNK.len() as u64;
                        }
                    } else if resp_cl.is_none()
                        && payload_at_boundary(&sse_tail, sse_tail_len)
                    {
                        if client_write.write_all(KEEPALIVE_PLAIN).is_err() {
                            write_failed = true;
                            break;
                        }
                        down_total += KEEPALIVE_PLAIN.len() as u64;
                    }
                }
                continue;
            }
            Err(e) => {
                stream_end = Some(classify_read_error(e.kind()));
                break;
            }
        }
    }

    if write_failed {
        return Err(ForwardError::ClientGone { down: down_total, ttfb });
    }
    if !headers_done {
        return Err(ForwardError::BadResponse { down: down_total, ttfb });
    }
    // Nothing was truncated *inside* a frame the backend promised, so the
    // only distinction left is why it stopped — which is what the usage
    // log records. Both causes are the backend's: the hop is loopback, so
    // a reset means the peer process went away mid-request.
    if resp_cl.is_none() && !resp_chunked {
        if let Some(end) = stream_end {
            return Err(match end {
                StreamEnd::Hung => ForwardError::Stream { down: down_total, ttfb },
                StreamEnd::Reset => ForwardError::StreamCut { down: down_total, ttfb },
            });
        }
    }
    if let Some(len) = resp_cl {
        if resp_body < len {
            return Err(ForwardError::Stream { down: down_total, ttfb });
        }
    }
    if resp_chunked && chunk_detector.state != ChunkState::Done {
        return Err(ForwardError::Stream { down: down_total, ttfb });
    }
    let _ = client_write.shutdown(std::net::Shutdown::Write);
    let _ = client.shutdown(std::net::Shutdown::Both);
    Ok((initial_bytes.len() as u64, down_total, ttfb))
}

struct InflightGuard {
    count: Arc<AtomicUsize>,
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Uniform 503 for "nothing is serving". Used twice: once before the body
/// is read (fail fast) and once after (a model hint may have reordered
/// the candidates and emptied the list). Same body, same status, either way.
fn no_backend_503(client: &mut TcpStream, rid_n: usize, method: &str, path: &str) {
    let err_body = format!(
        "{{\"error\":{{\"message\":\"No LAC inference backend is serving (MLX :{}, llama :{}, Ollama :{} all down or breaker-tripped). Start one with `lac serve mlx`.\",\"type\":\"lac_router_error\",\"code\":503}}}}",
        port_mlx(),
        port_llama(),
        port_ollama()
    );
    let _ = json_response(client, "503 Service Unavailable", &err_body);
    eprintln!("[lac-router rid={}] {} {} -> 503 no-backend", rid_n, method, path);
}

#[allow(clippy::too_many_arguments)]
fn handle_connection(
    mut client: TcpStream,
    preferred: Arc<AtomicUsize>,
    hc: Arc<HealthCache>,
    stats: StatsMap,
    routes: RouteMap,
    inflight: Arc<AtomicUsize>,
    rid: Arc<AtomicUsize>,
    started: Instant,
    expected_token: Arc<String>,
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
        Err(_) => {
            let mut c = client;
            let _ = json_response(&mut c, "400 Bad Request", "{\"error\":{\"message\":\"incomplete request\"}}");
            return;
        }
    };
    let (method, path) = request_line(&initial);
    let initial_text = String::from_utf8_lossy(&initial);
    if !valid_request_line(initial_text.lines().next().unwrap_or("")) {
        let mut c = client;
        let _ = json_response(&mut c, "400 Bad Request", "{\"error\":{\"message\":\"invalid request line\"}}");
        return;
    }
    if method == "OPTIONS" {
        // Answered before the auth gate on purpose: a CORS preflight never
        // carries credentials, so demanding one would just break browsers.
        // The reply is a static 204 with a wildcard CORS header and no
        // backend contact, so it leaks nothing about this deployment.
        let _ = handle_options(client);
        return;
    }

    // Remote Bearer gate: loopback is token-less; any non-loopback peer
    // without the exact token gets 401 and is never routed nor status-read.
    let peer_loopback = client
        .peer_addr()
        .map(|a| is_loopback_peer(&a))
        .unwrap_or(false);
    let proxy_forwarded = forwarded_request(&initial);
    let auth_required = !peer_loopback || proxy_forwarded || !expected_token.is_empty();
    if auth_required {
        let ok = !expected_token.is_empty()
            && bearer_from(&initial)
                .map(|t| constant_time_eq(&t, &expected_token))
                .unwrap_or(false);
        if !ok {
            let mut c = client;
            let _ = unauthorized(&mut c);
            eprintln!("[lac-router rid={}] {} {} -> 401 remote-auth", rid_n, method, path);
            return;
        }
    }

    let initial_headers = find_headers_end(&initial).unwrap_or(initial.len());
    let request_content_length = match parse_content_length_strict(&initial[..initial_headers]) {
        Ok(value) => value,
        Err(_) => {
            let mut c = client;
            let _ = json_response(&mut c, "400 Bad Request", "{\"error\":{\"message\":\"invalid request header\"}}");
            return;
        }
    };
    let chunked_request = match has_chunked_transfer_encoding(&initial[..initial_headers]) {
        Ok(value) => value,
        Err(_) => {
            let mut c = client;
            let _ = json_response(&mut c, "400 Bad Request", "{\"error\":{\"message\":\"invalid Transfer-Encoding\"}}");
            return;
        }
    };
    if request_content_length.is_some_and(|length| length > MAX_BODY) {
        let mut c = client;
        let _ = json_response(&mut c, "413 Payload Too Large", "{\"error\":{\"message\":\"request body exceeds the 16 MiB limit\"}}");
        return;
    }
    if request_content_length.is_some() && chunked_request {
        let mut c = client;
        let _ = json_response(&mut c, "400 Bad Request", "{\"error\":{\"message\":\"ambiguous request framing\"}}");
        return;
    }
    if request_content_length.is_none()
        && !chunked_request
        && !matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS")
    {
        let mut c = client;
        let _ = json_response(&mut c, "400 Bad Request", "{\"error\":{\"message\":\"request body requires Content-Length or chunked framing\"}}");
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
    if pick_backends(pref, &hc, &stats, None).is_empty() {
        no_backend_503(&mut client, rid_n, &method, &path);
        return;
    }

    if expect_continue(&initial[..initial_headers]) {
        let _ = client.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
    }
    let frame = match read_request_body(&client, &initial) {
        Ok(frame) => frame,
        Err(BodyError::TooLarge) => {
            let mut c = client;
            let _ = json_response(&mut c, "413 Payload Too Large", "{\"error\":{\"message\":\"request body exceeds the 16 MiB limit\"}}");
            return;
        }
        Err(BodyError::Invalid) => {
            let mut c = client;
            let _ = json_response(&mut c, "400 Bad Request", "{\"error\":{\"message\":\"invalid or incomplete request body\"}}");
            return;
        }
    };
    if frame.has_following {
        let mut c = client;
        // A body-less method cannot be carrying a body we failed to frame,
        // so trailing bytes can only be a second request.
        let bodyless_method = matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS");
        if bodyless_method || request_content_length.is_some() || chunked_request {
            let _ = json_response(&mut c, "400 Bad Request", "{\"error\":{\"message\":\"pipelined requests are not supported\"}}");
        } else {
            let _ = json_response(&mut c, "400 Bad Request", "{\"error\":{\"message\":\"request body requires Content-Length or chunked framing\"}}");
        }
        return;
    }

    // Zero-cost model hint: only the bytes already buffered, only in
    // AUTO/FASTEST (an explicit pin always wins over a guess).
    let model = sniff_model(&frame.wire[..frame.wire.len().min(8192)]).unwrap_or_default();
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

    let initial = sanitize_upstream_request(&frame.wire);

    let candidates = pick_backends(pref, &hc, &stats, model_hint);
    if candidates.is_empty() {
        no_backend_503(&mut client, rid_n, &method, &path);
        return;
    }

    let header_len = find_headers_end(&initial).unwrap_or(initial.len());
    let t0 = Instant::now();
    let mut tried: Vec<u16> = Vec::new();
    for (bid, port) in candidates {
        match try_forward(&client, port, &initial, method == "HEAD") {
            Ok((up, down, ttfb)) => {
                let up_body = up.saturating_sub(header_len as u64);
                let down_body = down;
                let est = (up_body + down_body) / 4;
                let ms = ttfb.unwrap_or_else(|| ewma_of(&stats, port).min(30_000.0));
                note_success(&stats, port, ms, up_body, down_body);
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
            Err(ForwardError::HttpStatus { status, ttfb }) => {
                note_error(&stats, port);
                tried.push(port);
                let up_body = initial.len().saturating_sub(header_len) as u64;
                let est = up_body / 4;
                log_usage(
                    rid_n,
                    backend_name(bid),
                    port,
                    &model,
                    up_body,
                    0,
                    est,
                    ttfb,
                    "http-error",
                );
                eprintln!(
                    "[lac-router rid={}] {} {} -> backend HTTP {}; trying next",
                    rid_n, method, path, status
                );
                continue;
            }
            Err(ForwardError::UnsupportedUpgrade { up, ttfb }) => {
                let up_body = up.saturating_sub(header_len as u64);
                let est = up_body / 4;
                log_usage(
                    rid_n,
                    backend_name(bid),
                    port,
                    &model,
                    up_body,
                    0,
                    est,
                    ttfb,
                    "unsupported-upgrade",
                );
                let mut c = client;
                let _ = json_response(
                    &mut c,
                    "501 Not Implemented",
                    "{\"error\":{\"message\":\"upstream protocol upgrades are not supported\"}}",
                );
                return;
            }
            Err(ForwardError::BadResponse { down, ttfb }) => {
                note_error(&stats, port);
                tried.push(port);
                let up_body = initial.len().saturating_sub(header_len) as u64;
                let est = (up_body + down) / 4;
                log_usage(
                    rid_n,
                    backend_name(bid),
                    port,
                    &model,
                    up_body,
                    down,
                    est,
                    ttfb,
                    "bad-response",
                );
                eprintln!(
                    "[lac-router rid={}] {} {} -> :{} invalid-response; trying next",
                    rid_n, method, path, port
                );
                continue;
            }
            Err(ForwardError::ClientGone { down, ttfb }) => {
                let up_body = initial.len().saturating_sub(header_len) as u64;
                let est = (up_body + down) / 4;
                log_usage(
                    rid_n,
                    backend_name(bid),
                    port,
                    &model,
                    up_body,
                    down,
                    est,
                    ttfb,
                    "client-gone",
                );
                return;
            }
            Err(ForwardError::StreamCut { down, ttfb }) => {
                // Charged: the upstream hop is loopback, so a reset is the
                // backend process going away, not a network event. A
                // backend that dies after the headers must not keep
                // receiving traffic just because it still accepts
                // connections.
                note_error(&stats, port);
                let up_body = initial.len().saturating_sub(header_len) as u64;
                let est = (up_body + down) / 4;
                log_usage(
                    rid_n,
                    backend_name(bid),
                    port,
                    &model,
                    up_body,
                    down,
                    est,
                    ttfb,
                    "stream-cut",
                );
                eprintln!(
                    "[lac-router rid={}] {} {} -> :{} stream-cut (up={} down={} {:.1}s)",
                    rid_n,
                    method,
                    path,
                    port,
                    up_body,
                    down,
                    t0.elapsed().as_secs_f64()
                );
                return;
            }
            Err(ForwardError::Stream { down, ttfb }) => {
                note_error(&stats, port);
                let up_body = initial.len().saturating_sub(header_len) as u64;
                let est = (up_body + down) / 4;
                log_usage(
                    rid_n,
                    backend_name(bid),
                    port,
                    &model,
                    up_body,
                    down,
                    est,
                    ttfb,
                    "stream-broke",
                );
                eprintln!(
                    "[lac-router rid={}] {} {} -> :{} stream-broke (up={} down={} {:.1}s)",
                    rid_n,
                    method,
                    path,
                    port,
                    up_body,
                    down,
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

    let host = bind_host();
    let token = api_token();
    // Fail closed: a non-loopback bind without a token would expose the
    // gateway unauthenticated — refuse to start instead.
    if !is_loopback_host(&host)
        && env::var("LAC_ALLOW_INSECURE_BIND").ok().as_deref() != Some("1")
    {
        eprintln!(
            "Refusing non-loopback bind {} without explicit LAC_ALLOW_INSECURE_BIND=1. Use tailscale serve --https in front of loopback.",
            host
        );
        std::process::exit(1);
    }
    if !is_loopback_host(&host) && token.is_empty() {
        eprintln!(
            "Refusing to bind lac-router to {} without LAC_API_TOKEN (remote would be open). Set LAC_API_TOKEN.",
            host
        );
        std::process::exit(1);
    }
    let token_required = !token.is_empty();
    let expected_token: Arc<String> = Arc::new(token);

    let bind_addr = bind_address(&host, port);
    let listener = match TcpListener::bind(&bind_addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind lac-router to {}: {}", bind_addr, e);
            std::process::exit(1);
        }
    };
    let bound_addr = listener
        .local_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_else(|_| bind_addr.clone());

    let hc = Arc::new(HealthCache::new(Duration::from_secs(5)));
    let stats: StatsMap = Arc::new(Mutex::new(HashMap::new()));
    let routes: RouteMap = Arc::new(Mutex::new(HashMap::new()));
    let inflight = Arc::new(AtomicUsize::new(0));
    let rid = Arc::new(AtomicUsize::new(1));
    let started = Instant::now();

    eprintln!("========================================================");
    eprintln!("  LAC Unified Intelligent Router v2.8 (Rust native)");
    eprintln!("  Listening on http://{}", bound_addr);
    eprintln!(
        "  Remote auth: {}",
        if token_required {
            "Bearer required on all peers (including loopback)"
        } else if is_loopback_host(&host) {
            "loopback open (local dev)"
        } else {
            "Bearer required for non-loopback (fail-closed)"
        }
    );
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
    eprintln!("  Health & metrics: http://{}/lac/status", bound_addr);
    eprintln!(
        "  Hot-swap backend: http://{}/lac/switch?target=[mlx|llama|ollama|auto|fastest]",
        bound_addr
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
                let tok = Arc::clone(&expected_token);
                thread::spawn(move || {
                    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        handle_connection(client, pref, hc, stats, routes, inflight, rid, started, tok);
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
    fn strips_auth_headers_before_forwarding() {
        let request = b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\n Authorization: Bearer secret\r\nauthorization: Bearer duplicate\r\nProxy-Authorization: Basic secret\r\n proxy-authorization: Basic duplicate\r\nX-Trace: keep\r\n\r\nauthorization-body";
        let sanitized = sanitize_upstream_request(request);
        let text = String::from_utf8(sanitized.clone()).expect("sanitized request is UTF-8");
        let head_end = find_headers_end(&sanitized).expect("headers");
        let headers = String::from_utf8_lossy(&sanitized[..head_end]);
        assert!(!headers.lines().any(|line| {
            line.split_once(':')
                .map(|(name, _)| {
                    let name = name.trim();
                    name.eq_ignore_ascii_case("authorization")
                        || name.eq_ignore_ascii_case("proxy-authorization")
                })
                .unwrap_or(false)
        }));
        assert!(text.contains("X-Trace: keep"));
        assert!(text.ends_with("\r\n\r\nauthorization-body"));
    }

    #[test]
    fn read_errors_split_reset_from_hung() {
        // Everything that is not a lapsed budget reports as a reset: the
        // upstream is loopback, so the peer process went away.
        for kind in [
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::NotConnected,
            io::ErrorKind::UnexpectedEof,
            io::ErrorKind::BrokenPipe,
            io::ErrorKind::InvalidData,
            io::ErrorKind::Other,
        ] {
            assert_eq!(classify_read_error(kind), StreamEnd::Reset, "{kind:?}");
        }
        // A timeout reaching this point has already outlived the budget.
        assert_eq!(
            classify_read_error(io::ErrorKind::TimedOut),
            StreamEnd::Hung
        );
        assert_eq!(
            classify_read_error(io::ErrorKind::WouldBlock),
            StreamEnd::Hung
        );
    }

    #[test]
    fn sanitize_canonicalizes_content_length() {
        let duplicated = b"POST /v1/chat HTTP/1.1\r\nContent-Length: 4\r\nContent-Length: 4\r\n\r\nbody";
        let text = String::from_utf8(sanitize_upstream_request(duplicated)).expect("utf-8");
        assert_eq!(text.matches("Content-Length:").count(), 1, "{text}");
        assert!(text.contains("Content-Length: 4\r\n"), "{text}");

        let padded = b"POST /v1/chat HTTP/1.1\r\nContent-Length: 0004\r\n\r\nbody";
        let text = String::from_utf8(sanitize_upstream_request(padded)).expect("utf-8");
        assert!(text.contains("Content-Length: 4\r\n"), "{text}");
        assert!(!text.contains("0004"), "{text}");

        let expect = b"POST /v1/chat HTTP/1.1\r\nExpect: 100-continue\r\nContent-Length: 4\r\n\r\nbody";
        let text = String::from_utf8(sanitize_upstream_request(expect)).expect("utf-8");
        assert!(!text.contains("Expect"), "{text}");
        assert!(text.ends_with("\r\n\r\nbody"), "{text}");

        // Anything else named Expect is the client's own contract: keep it.
        let other = b"POST /v1/chat HTTP/1.1\r\nExpect: something-else\r\nContent-Length: 4\r\n\r\nbody";
        let text = String::from_utf8(sanitize_upstream_request(other)).expect("utf-8");
        assert!(text.contains("Expect: something-else"), "{text}");
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
        assert!(valid_request_line("POST /v1/chat HTTP/1.1"));
        assert!(valid_request_line("GET http://example.test/x HTTP/1.1"));
        assert!(!valid_request_line("POST /v1/chat HTTP/2"));
        assert!(!valid_request_line("POST  /v1/chat HTTP/1.1"));
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
        let duplicate = b"POST / HTTP/1.1\r\nContent-Length: 4\r\nContent-Length: 4\r\n\r\ntest";
        assert_eq!(parse_content_length_strict(duplicate), Ok(Some(4)));
        let conflicting = b"POST / HTTP/1.1\r\nContent-Length: 4\r\nContent-Length: 5\r\n\r\n";
        assert!(parse_content_length_strict(conflicting).is_err());
        let invalid = b"POST / HTTP/1.1\r\nContent-Length: nope\r\n\r\n";
        assert!(parse_content_length_strict(invalid).is_err());
        let signed = b"POST / HTTP/1.1\r\nContent-Length: +2\r\n\r\n";
        assert!(parse_content_length_strict(signed).is_err());
        let folded = b"POST / HTTP/1.1\r\n Content-Length: 2\r\n\r\n";
        assert!(parse_content_length_strict(folded).is_err());
    }

    #[test]
    fn resp_framing_parsed() {
        let json = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 48\r\nConnection: keep-alive\r\n\r\n";
        assert_eq!(
            parse_resp_headers(json),
            Ok((Some(48), false, false, false))
        );
        let sse = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
        assert_eq!(parse_resp_headers(sse), Ok((None, true, true, true)));
        let encoded_sse = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Encoding: gzip\r\nContent-Length: 3\r\n\r\n";
        assert_eq!(parse_resp_headers(encoded_sse), Ok((Some(3), false, true, false)));
        let both = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 99\r\nContent-Type: text/event-stream\r\n\r\n";
        assert!(parse_resp_headers(both).is_err());
        let loose = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: notchunked\r\n\r\n";
        assert!(parse_resp_headers(loose).is_err());
        let media = b"HTTP/1.1 200 OK\r\nContent-Type: application/json; note=\"text/event-stream\"\r\nContent-Length: 2\r\n\r\n";
        assert_eq!(parse_resp_headers(media), Ok((Some(2), false, false, false)));
        let transfer = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
        assert_eq!(has_chunked_transfer_encoding(transfer), Ok(true));
        let chained = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, chunked\r\n\r\n";
        assert_eq!(has_chunked_transfer_encoding(chained), Ok(true));
        let unsupported = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked, gzip\r\n\r\n";
        assert!(has_chunked_transfer_encoding(unsupported).is_err());
        let duplicate = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked, chunked\r\n\r\n";
        assert!(has_chunked_transfer_encoding(duplicate).is_err());
        let spaced = b"HTTP/1.1 200 OK\r\nTransfer-Encoding : chunked\r\n\r\n";
        assert!(has_chunked_transfer_encoding(spaced).is_err());
        assert!(parse_resp_headers(spaced).is_err());
        let folded = b"HTTP/1.1 200 OK\r\nX-Test: yes\r\n Transfer-Encoding: chunked\r\n\r\n";
        assert!(has_chunked_transfer_encoding(folded).is_err());
        let conflicting = b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n";
        assert!(parse_resp_headers(conflicting).is_err());
        let obs_text = b"HTTP/1.1 200 OK\r\nX-Name: caf\xff\r\nContent-Length: 2\r\n\r\n";
        assert_eq!(parse_resp_headers(obs_text), Ok((Some(2), false, false, false)));
        let bws_name = b"HTTP/1.1 200 OK\r\nX-Trace : fine\r\nContent-Length: 2\r\n\r\n";
        assert_eq!(parse_resp_headers(bws_name), Ok((Some(2), false, false, false)));
        let duplicate_same = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n";
        assert_eq!(parse_resp_headers(duplicate_same), Ok((Some(2), false, false, false)));
    }

    #[test]
    fn chunk_terminal_detected_across_splits_and_trailers() {
        let wire = b"5;ext=1\r\nhello\r\n0\r\nX-Trailer: yes\r\n\r\n";
        for split in 0..=wire.len() {
            assert!(
                chunk_terminal_detected(&[&wire[..split], &wire[split..]]),
                "split at {} lost terminal chunk",
                split
            );
        }
        assert!(chunk_terminal_detected(&[b"0\r\n\r\n"]));
        assert!(!chunk_terminal_detected(&[b"5\r\nhello\r\n"]));
        assert!(!chunk_terminal_detected(&[b"5\r\nhello0\r\n\r\n0\r\n\r\n"]));
        assert!(!chunk_terminal_detected(&[b"5;bad ext=1\r\nhello\r\n0\r\n\r\n"]));
        assert!(chunk_terminal_detected(&[b"5 ;foo=\"a;b\"\r\nhello\r\n0\r\n\r\n"]));
        assert!(!chunk_terminal_detected(&[b"5;bad\x01ext=1\r\nhello\r\n0\r\n\r\n"]));
        assert!(!chunk_terminal_detected(&[b"0;\r\n\r\n"]));
        assert!(!chunk_terminal_detected(&[b"5;=x\r\nhello\r\n0\r\n\r\n"]));
        assert!(!chunk_terminal_detected(&[b"5;foo=\"unterminated\r\nhello\r\n0\r\n\r\n"]));
        assert!(!chunk_terminal_detected(&[b"0\r\nnot-a-header\r\n\r\n"]));
        assert!(!chunk_terminal_detected(&[b"0\r\nContent-Length: 4\r\n\r\n"]));
    }

    #[test]
    fn chunk_feed_reports_exact_terminal_offset() {
        let mut detector = ChunkedDetector::default();
        assert_eq!(
            detector.feed(b"0\r\n\r\nTRAILING"),
            Ok(ChunkProgress::Complete { consumed: 5 })
        );
    }

    #[test]
    fn chunk_feed_distinguishes_incomplete_and_invalid() {
        let mut detector = ChunkedDetector::default();
        assert_eq!(detector.feed(b"5\r\nhel"), Ok(ChunkProgress::NeedMore));
        assert_eq!(detector.feed(b"lo\r\n"), Ok(ChunkProgress::NeedMore));
        assert_eq!(detector.feed(b"0\n"), Err(()));
    }

    #[test]
    fn keepalive_is_allowed_only_at_a_complete_size_boundary() {
        let mut detector = ChunkedDetector::default();
        assert!(detector.can_inject_keepalive());
        assert_eq!(detector.feed(b"1"), Ok(ChunkProgress::NeedMore));
        assert!(!detector.can_inject_keepalive());
        assert_eq!(detector.feed(b"\r\na\r\n"), Ok(ChunkProgress::NeedMore));
        assert!(detector.can_inject_keepalive());
        assert_eq!(detector.feed(b"0\r\n"), Ok(ChunkProgress::NeedMore));
        assert!(!detector.can_inject_keepalive());
        assert_eq!(detector.feed(b"\r\n"), Ok(ChunkProgress::Complete { consumed: 2 }));
    }

    #[test]
    fn keepalive_requires_a_complete_sse_event_boundary() {
        let mut detector = ChunkedDetector {
            track_payload: true,
            ..ChunkedDetector::default()
        };
        assert_eq!(
            detector.feed(b"a\r\ndata: part\r\n"),
            Ok(ChunkProgress::NeedMore)
        );
        assert!(detector.can_inject_keepalive());
        assert!(!detector.payload_at_sse_boundary());
        assert_eq!(detector.feed(b"5\r\nial\n\n\r\n"), Ok(ChunkProgress::NeedMore));
        assert!(detector.can_inject_keepalive());
        assert!(
            detector.payload_at_sse_boundary(),
            "tail={:?} len={}",
            detector.payload_tail,
            detector.payload_len
        );

        let mut crlf = ChunkedDetector {
            track_payload: true,
            ..ChunkedDetector::default()
        };
        assert_eq!(crlf.feed(b"4\r\n\r\n\r\n\r\n"), Ok(ChunkProgress::NeedMore));
        assert!(crlf.can_inject_keepalive());
        assert!(crlf.payload_at_sse_boundary());
        assert!(payload_at_boundary(b"\0x\r\r", 4));
        assert!(payload_at_boundary(b"x\n\r\n", 4));
        assert!(!payload_at_boundary(b"\0\0x\r", 4));
        assert!(!payload_at_boundary(b"\0\0x\n", 4));
        assert!(!payload_at_boundary(b"\0\0\0\0", 1));
        let mut tiny_tail = [0u8; 4];
        let mut tiny_len = 0usize;
        for byte in b"abcd" {
            update_payload_tail(&mut tiny_tail, &mut tiny_len, &[*byte]);
        }
        assert_eq!(tiny_len, 4);
        assert_eq!(tiny_tail, *b"abcd");
    }

    #[test]
    fn response_status_parsed() {
        assert_eq!(response_status(b"HTTP/1.1 200 OK\r\n\r\n"), Some(200));
        assert_eq!(response_status(b"HTTP/1.1 503 Service Unavailable\r\n\r\n"), Some(503));
        assert_eq!(response_status(b"HTTP/1.1 600 Extension\r\n\r\n"), Some(600));
        assert_eq!(response_status(b"HTTP/1.1 200\r\n\r\n"), Some(200));
        assert_eq!(response_status(b"HTTP/1.1 200\tOK\r\n\r\n"), None);
        assert_eq!(response_status(b"HTTP/1.1 200\x0bOK\r\n\r\n"), None);
        assert_eq!(response_status(b"HTTP/1.1 +200 OK\r\n\r\n"), None);
        assert_eq!(response_status(b"garbage 200 OK\r\n\r\n"), None);
        assert_eq!(response_status(b"not-http"), None);
    }

    #[test]
    fn bearer_gate() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("::1"));
        assert!(!is_loopback_host("0.0.0.0"));
        assert!(!is_loopback_host("100.64.0.5"));
        assert_eq!(bind_address("127.0.0.1", 8000), "127.0.0.1:8000");
        assert_eq!(bind_address("::1", 8000), "[::1]:8000");
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
        let req = b"GET /v1/models HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer sekrit\r\n\r\n";
        assert_eq!(bearer_from(req).as_deref(), Some("sekrit"));
        let mixed = b"GET /v1/models HTTP/1.1\r\nAuthorization: bEaReR sekrit\r\n\r\n";
        assert_eq!(bearer_from(mixed).as_deref(), Some("sekrit"));
        let noauth = b"GET /v1/models HTTP/1.1\r\nHost: x\r\n\r\n";
        assert_eq!(bearer_from(noauth), None);
        let bad = b"GET /x HTTP/1.1\r\nAuthorization: Basic abc\r\n\r\n";
        assert_eq!(bearer_from(bad), None);
        // Only the Authorization header counts, and only its first
        // occurrence: a Proxy-Authorization must not stand in for it.
        let proxy_only = b"GET /x HTTP/1.1\r\nProxy-Authorization: Bearer sekrit\r\n\r\n";
        assert_eq!(bearer_from(proxy_only), None);
        // A token is taken whole, never as a prefix.
        let trailing = b"GET /x HTTP/1.1\r\nAuthorization: Bearer sekrit extra\r\n\r\n";
        assert_eq!(bearer_from(trailing).as_deref(), Some("sekrit extra"));
        let empty = b"GET /x HTTP/1.1\r\nAuthorization: Bearer \r\n\r\n";
        assert_eq!(bearer_from(empty), None);
        let no_token = b"GET /x HTTP/1.1\r\nAuthorization: Bearer\r\n\r\n";
        assert_eq!(bearer_from(no_token), None);
        // Lowercase field name is still the field.
        let lower = b"GET /v1/models HTTP/1.1\r\nauthorization: Bearer sekrit\r\n\r\n";
        assert_eq!(bearer_from(lower).as_deref(), Some("sekrit"));
        // Duplicate headers: the first decides, and a wrong one does not
        // fall through to a later correct one.
        let dup_wrong_first = b"GET /x HTTP/1.1\r\nAuthorization: Bearer wrong\r\nAuthorization: Bearer sekrit\r\n\r\n";
        assert_eq!(bearer_from(dup_wrong_first).as_deref(), Some("wrong"));
        let forwarded = b"GET /v1/models HTTP/1.1\r\nX-Forwarded-For: 100.64.0.5\r\n\r\n";
        assert!(forwarded_request(forwarded));
        let peer_lo: SocketAddr = "127.0.0.1:8000".parse().unwrap();
        let peer_remote: SocketAddr = "100.64.0.5:1234".parse().unwrap();
        assert!(is_loopback_peer(&peer_lo));
        assert!(!is_loopback_peer(&peer_remote));
    }
}
