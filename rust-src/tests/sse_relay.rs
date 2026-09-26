//! Downstream framing regression tests (proven by live probe 2026-09-17).
//!
//! Spins up the real `lac-router` binary against a scripted fake backend
//! and asserts, at the byte level:
//!   1. chunked SSE passes through byte-identical AND incrementally
//!      (no buffering, terminator intact);
//!   2. a complete Content-Length response ends the relay promptly even
//!      when the backend holds the socket open (no read-until-EOF stall);
//!   3. idle `: keep-alive` comments enter SSE bodies but NEVER JSON
//!      bodies (LAC_ROUTER_KEEPALIVE_SECS=1 keeps this test fast).
//! One sequential #[test]: child environments are explicit; scenarios run in order.

mod support;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

fn spawn_healthy_backend() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("healthy backend bind");
    let port = listener.local_addr().expect("healthy backend address").port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            thread::spawn(move || {
                let mut s = stream;
                let _ = read_exact_head(&mut s);
                let body = r#"{"choices":[{"message":{"content":"healthy-failover"}}]}"#;
                let _ = s.write_all(format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                ).as_bytes());
            });
        }
    });
    port
}

fn read_exact_head(stream: &mut TcpStream) -> Vec<u8> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 65536 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    buf
}

fn req_content_length(head: &[u8]) -> usize {
    let s = String::from_utf8_lossy(head).to_lowercase();
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("content-length:") {
            if let Ok(n) = v.trim().parse() {
                return n;
            }
        }
    }
    0
}

/// Scripted backend: scenario comes from the POST body's "scenario" field.
fn spawn_backend() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("fake backend bind");
    let port = listener.local_addr().expect("fake backend address").port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            thread::spawn(move || {
                let mut s = stream;
                let raw = read_exact_head(&mut s);
                // read_exact_head may have over-read the pipelined body
                // (headers+body arrive in one TCP segment). Split so the
                // scenario token isn't discarded.
                let head_end = raw
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|p| p + 4)
                    .unwrap_or(raw.len());
                let head_bytes = &raw[..head_end];
                let text = String::from_utf8_lossy(head_bytes).to_string();
                let path = text.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/").to_string();
                if path.starts_with("/v1/models") {
                    let body = r#"{"object":"list","data":[{"id":"qwen3.8-27b"}]}"#;
                    let _ = s.write_all(format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(), body
                    ).as_bytes());
                    return;
                }
                // Start with over-read body bytes, then drain the rest up
                // to Content-Length (bounded). Fixes body_len=0 misroute
                // where sse-chunked fell through to json-slow. Note the 8
                // KiB clamp: a larger declared Content-Length leaves bytes
                // queued, so closing the socket below emits RST rather than
                // FIN and the router logs stream-cut.
                let mut body: Vec<u8> = raw[head_end..].to_vec();
                let want = req_content_length(head_bytes).min(8192);
                let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                let mut chunk = [0u8; 1024];
                while body.len() < want.max(body.len()).min(8192) && body.len() < 8192 {
                    if body.len() >= want && want > 0 {
                        break;
                    }
                    if want == 0 && !body.is_empty() {
                        break;
                    }
                    match s.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let room = 8192 - body.len();
                            body.extend_from_slice(&chunk[..n.min(room)]);
                            if body.len() >= 1024 && want == 0 {
                                break;
                            }
                        }
                    }
                    if want == 0 {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&body);
                if std::env::var("SSE_RELAY_DEBUG").is_ok() {
                    let _ = std::fs::write(
                        std::env::temp_dir().join(format!("sse-relay-req-{}.bin", std::process::id())),
                        [raw.clone(), b"\r\n\r\n".to_vec(), body.clone()].concat(),
                    );
                }
                let scenario = if text.contains("many-interims") {
                    "many-interims"
                } else if text.contains("client-gone") {
                    "client-gone"
                } else if text.contains("sse-crlf-slow") {
                    "sse-crlf-slow"
                } else if text.contains("bad-response") {
                    "bad-response"
                } else if text.contains("reset-205") {
                    "reset-205"
                } else if text.contains("sse-partial-slow") {
                    "sse-partial-slow"
                } else if text.contains("transfer-coded-sse") {
                    "transfer-coded-sse"
                } else if text.contains("sse-partial-event") {
                    "sse-partial-event"
                } else if text.contains("encoded-sse") {
                    "encoded-sse"
                } else if text.contains("interim-final") {
                    "interim-final"
                } else if text.contains("upgrade") {
                    "upgrade"
                } else if text.contains("cl-overrun") {
                    "cl-overrun"
                } else if text.contains("chunk-overrun") {
                    "chunk-overrun"
                } else if text.contains("sse-trailer-delay") {
                    "sse-trailer-delay"
                } else if text.contains("sse-partial-size") {
                    "sse-partial-size"
                } else if text.contains("sse-mid-chunk") {
                    "sse-mid-chunk"
                } else if text.contains("sse-chunked-slow") {
                    "sse-chunked-slow"
                } else if text.contains("sse-chunked") {
                    "sse-chunked"
                } else if text.contains("short-cl") {
                    "short-cl"
                } else if text.contains("short-chunk") {
                    "short-chunk"
                } else if text.contains("sse-trailer") {
                    "sse-trailer"
                } else if text.contains("sse-split") {
                    "sse-split"
                } else if text.contains("hold-open") {
                    "hold-open"
                } else if text.contains("eof-json") {
                    "eof-json"
                } else if text.contains("rst-cut") {
                    "rst-cut"
                } else if text.contains("sse-slow") {
                    "sse-slow"
                } else {
                    "json-slow"
                };
                eprintln!("[fake] scenario={} body_len={}", scenario, body.len());
                match scenario {
                    "many-interims" => {
                        for _ in 0..9 {
                            let _ = s.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
                        }
                        let body = r#"{"choices":[{"message":{"content":"too-many-interims"}}]}"#;
                        let _ = s.write_all(format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        ).as_bytes());
                    }
                    "client-gone" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: gone\n\n");
                        thread::sleep(Duration::from_secs(2));
                    }
                    "sse-crlf-slow" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: crlf-event\r\n\r\n");
                        thread::sleep(Duration::from_secs(3));
                    }
                    "bad-response" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n");
                    }
                    "reset-205" => {
                        let _ = s.write_all(b"HTTP/1.1 205 Reset Content\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n0\r\n\r\n");
                        thread::sleep(Duration::from_secs(3));
                    }
                    "sse-partial-slow" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: partial");
                        thread::sleep(Duration::from_secs(2));
                        let _ = s.write_all(b" event\n\n");
                        thread::sleep(Duration::from_secs(2));
                    }
                    "transfer-coded-sse" => {
                        let body = b"data: transfer-coded-sse-event\n\n";
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: gzip, chunked\r\nConnection: close\r\n\r\n");
                        let _ = s.write_all(format!("{:X}\r\n", body.len()).as_bytes());
                        let _ = s.write_all(body);
                        let _ = s.write_all(b"\r\n");
                        thread::sleep(Duration::from_secs(2));
                        let _ = s.write_all(b"0\r\n\r\n");
                    }
                    "sse-partial-event" => {
                        let first = b"data: partial";
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
                        let _ = s.write_all(format!("{:X}\r\n", first.len()).as_bytes());
                        let _ = s.write_all(first);
                        let _ = s.write_all(b"\r\n");
                        let second = b" event\n\n";
                        let _ = s.write_all(format!("{:X}\r\n", second.len()).as_bytes());
                        thread::sleep(Duration::from_secs(2));
                        let _ = s.write_all(second);
                        let _ = s.write_all(b"\r\n0\r\n\r\n");
                    }
                    "encoded-sse" => {
                        let body = b"data: encoded-sse-event\n\n";
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Encoding: gzip\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
                        let _ = s.write_all(format!("{:X}\r\n", body.len()).as_bytes());
                        let _ = s.write_all(body);
                        let _ = s.write_all(b"\r\n");
                        thread::sleep(Duration::from_secs(2));
                        let _ = s.write_all(b"0\r\n\r\n");
                    }
                    "interim-final" => {
                        let body = r#"{"choices":[{"message":{"content":"interim-final"}}]}"#;
                        let _ = s.write_all(format!(
                            "HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        ).as_bytes());
                    }
                    "upgrade" => {
                        let _ = s.write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\n\r\n");
                    }
                    "sse-mid-chunk" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nhe");
                        thread::sleep(Duration::from_secs(3));
                        let _ = s.write_all(b"llo\r\n0\r\n\r\n");
                    }
                    "sse-chunked-slow" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
                        thread::sleep(Duration::from_secs(3));
                        let event = b"data: chunked-slow-event\n\n";
                        let _ = s.write_all(format!("{:X}\r\n", event.len()).as_bytes());
                        let _ = s.write_all(event);
                        let _ = s.write_all(b"\r\n0\r\n\r\n");
                    }
                    "sse-chunked" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
                        for ev in ["data: A\n\n", "data: B\n\n", "data: C\n\n", "data: [DONE]\n\n"] {
                            let _ = s.write_all(format!("{:X}\r\n{}\r\n", ev.len(), ev).as_bytes());
                            thread::sleep(Duration::from_millis(100));
                        }
                        let _ = s.write_all(b"0\r\n\r\n");
                    }
                    "sse-trailer" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
                        let event = b"data: trailer-event\n\n";
                        let _ = s.write_all(format!("{:X}\r\n", event.len()).as_bytes());
                        let _ = s.write_all(event);
                        let _ = s.write_all(b"\r\n0\r\nX-Trailer: yes\r\n\r\n");
                        thread::sleep(Duration::from_secs(3));
                    }
                    "sse-split" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
                        let event = b"data: 0\r\n\r\n";
                        let _ = s.write_all(format!("{:X}\r", event.len()).as_bytes());
                        thread::sleep(Duration::from_millis(100));
                        let _ = s.write_all(b"\ndata: 0\r");
                        thread::sleep(Duration::from_millis(100));
                        let _ = s.write_all(b"\n\r\n\r\n0\r\n\r\n");
                    }
                    "cl-overrun" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabcEXTRA");
                    }
                    "chunk-overrun" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n0\r\n\r\nEXTRA");
                    }
                    "sse-trailer-delay" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n0\r\n");
                        thread::sleep(Duration::from_secs(2));
                        let _ = s.write_all(b"\r\n");
                        thread::sleep(Duration::from_secs(2));
                    }
                    "sse-partial-size" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1");
                        thread::sleep(Duration::from_secs(2));
                        let _ = s.write_all(b"\r\na\r\n0\r\n\r\n");
                    }
                    "short-cl" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\nabc");
                    }
                    "short-chunk" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nab\r\n");
                    }
                    "hold-open" => {
                        let payload = r#"{"choices":[{"message":{"content":"hold-open ok"}}]}"#;
                        let _ = s.write_all(format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{}",
                            payload.len(), payload
                        ).as_bytes());
                        thread::sleep(Duration::from_secs(4));
                    }
                    "eof-json" => {
                        // EOF-delimited (HTTP/1.0, no Content-Length, not
                        // chunked) and closed immediately: a clean cut at a
                        // message boundary, not a truncated frame.
                        let _ = s.write_all(
                            b"HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"eof\":\"ok\"}",
                        );
                    }
                    "rst-cut" => {
                        // A chunked request is never drained by the
                        // dispatch above, so its body is still queued when
                        // we close: the kernel sends RST, not FIN. Pause
                        // before the write so the router has finished
                        // sending, and generously after it: a reset purges
                        // the peer's receive buffer, so the router has to
                        // read the response first or it reports
                        // bad-response instead of stream-cut.
                        thread::sleep(Duration::from_millis(100));
                        let _ = s.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: rst",
                        );
                        thread::sleep(Duration::from_millis(1000));
                    }
                    "sse-slow" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n");
                        thread::sleep(Duration::from_secs(3));
                        let ev = "data: slow-event\n\n";
                        let _ = s.write_all(format!("{:X}\r\n{}\r\n0\r\n\r\n", ev.len(), ev).as_bytes());
                    }
                    _ => {
                        let payload = r#"{"choices":[{"message":{"content":"slow-json-ok"}}]}"#;
                        let _ = s.write_all(format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            payload.len()
                        ).as_bytes());
                        thread::sleep(Duration::from_secs(3));
                        let _ = s.write_all(payload.as_bytes());
                    }
                }
            });
        }
    });
    port
}

/// Raw POST through the router; returns (raw bytes, arrival timeline, total).
fn probe(router_port: u16, scenario: &str) -> (Vec<u8>, Vec<(f64, usize)>, f64) {
    let body = format!("{{\"model\":\"qwen3.8-27b\",\"scenario\":\"{}\"}}", scenario);
    let req = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(), body
    );
    let mut s = TcpStream::connect_timeout(&format!("127.0.0.1:{}", router_port).parse().unwrap(), Duration::from_secs(5)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let t0 = Instant::now();
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    let mut times = Vec::new();
    let mut chunk = [0u8; 65536];
    loop {
        match s.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                times.push((t0.elapsed().as_secs_f64(), n));
                raw.extend_from_slice(&chunk[..n]);
            }
            Err(_) => break,
        }
    }
    let total = t0.elapsed().as_secs_f64();
    (raw, times, total)
}

/// Read the answer to EOF, tolerating a reset. When the router rejects a
/// request it can close while our bytes are still queued, and the kernel
/// answers that with an RST instead of a FIN — whatever the router already
/// sent is still the answer, so the caller's status assertion stays the
/// real check and a lost response fails there, not here.
fn read_response(stream: &mut TcpStream) -> Vec<u8> {
    let mut response = Vec::new();
    if let Err(error) = stream.read_to_end(&mut response) {
        // Keep the bytes the router did send, but make a truncated read
        // visible: a timeout here is not the same event as a reset.
        if std::env::var("SSE_RELAY_DEBUG").is_ok() {
            eprintln!("[probe] read ended with {error:?} after {} bytes", response.len());
        }
    }
    response
}

fn probe_raw(router_port: u16, request: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", router_port).parse().unwrap(),
        Duration::from_secs(5),
    )
    .unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    stream.write_all(request).unwrap();
    read_response(&mut stream)
}

fn probe_raw_parts(router_port: u16, first: &[u8], second: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", router_port).parse().unwrap(),
        Duration::from_secs(5),
    )
    .unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    stream.write_all(first).unwrap();
    thread::sleep(Duration::from_millis(50));
    stream.write_all(second).unwrap();
    read_response(&mut stream)
}

fn probe_header_only(router_port: u16, request: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", router_port).parse().unwrap(),
        Duration::from_secs(5),
    )
    .unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    stream.write_all(request).unwrap();
    read_response(&mut stream)
}

fn probe_client_gone(router_port: u16, request: &[u8]) {
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", router_port).parse().unwrap(),
        Duration::from_secs(5),
    )
    .unwrap();
    stream.write_all(request).unwrap();
    let _ = stream.shutdown(std::net::Shutdown::Both);
    drop(stream);
}

fn probe_timed(router_port: u16, request: &[u8]) -> (Vec<u8>, f64) {
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", router_port).parse().unwrap(),
        Duration::from_secs(5),
    )
    .unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    let t0 = Instant::now();
    stream.write_all(request).unwrap();
    let response = read_response(&mut stream);
    (response, t0.elapsed().as_secs_f64())
}

/// `Expect: 100-continue`: the router must answer the expectation itself,
/// then relay the body that follows it.
fn probe_expect_continue(router_port: u16) -> Vec<u8> {
    let body = r#"{"model":"qwen3.8-27b"}"#;
    let head = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nExpect: 100-continue\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", router_port).parse().unwrap(),
        Duration::from_secs(5),
    )
    .unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    stream.write_all(head.as_bytes()).unwrap();
    let mut interim = Vec::new();
    let mut chunk = [0u8; 256];
    while !interim.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream
            .read(&mut chunk)
            .expect("router answers 100-continue before the body");
        assert!(n > 0, "router closed instead of sending 100 Continue");
        interim.extend_from_slice(&chunk[..n]);
    }
    assert!(
        String::from_utf8_lossy(&interim).starts_with("HTTP/1.1 100 Continue"),
        "expected interim response, got: {:?}",
        String::from_utf8_lossy(&interim)
    );
    stream.write_all(body.as_bytes()).unwrap();
    read_response(&mut stream)
}

/// Chunked upload one byte past the 16 MiB + 64 KiB request ceiling. The
/// last byte is sent on its own after a pause so the router has drained
/// everything else first: it can then answer 413 on a clean socket instead
/// of a reset that would swallow the response.
fn probe_paced_oversized_chunked(router_port: u16) -> Vec<u8> {
    let head = b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
    // MAX_BODY (16 MiB) + MAX_HEADERS: the router buffers the request up to
    // header_len + this, and rejects on the first byte beyond it.
    let ceiling = 16 * 1024 * 1024 + 65_536;
    // Declare a chunk far bigger than we send, so the framing never
    // completes and only the byte ceiling can trip.
    let size_line = b"2000000\r\n";
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", router_port).parse().unwrap(),
        Duration::from_secs(5),
    )
    .unwrap();
    // Generous: a loaded CI runner may need a while to move 16 MiB.
    stream.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    stream.write_all(head).unwrap();
    stream.write_all(size_line).unwrap();
    let block = vec![b'a'; 64 * 1024];
    let mut remaining = ceiling - size_line.len();
    while remaining > 0 {
        let take = block.len().min(remaining);
        stream.write_all(&block[..take]).unwrap();
        remaining -= take;
    }
    thread::sleep(Duration::from_millis(200));
    stream.write_all(b"a").unwrap();
    read_response(&mut stream)
}

/// Drive the `rst-cut` scenario. The request is chunked, so the fake
/// backend never drains its body: when the backend closes, unread bytes
/// are queued and the kernel resets the connection instead of finishing
/// it. The router must charge that — the upstream hop is loopback, so a
/// reset is the backend process going away.
fn probe_chunked_reset(router_port: u16) -> (Vec<u8>, f64) {
    let mut body =
        "{\"model\":\"qwen3.8-27b\",\"scenario\":\"rst-cut\",\"pad\":\"".to_string();
    body.push_str(&"a".repeat(16 * 1024));
    body.push_str("\"}");
    let request = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:X}\r\n{}\r\n0\r\n\r\n",
        body.len(),
        body
    );
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", router_port).parse().unwrap(),
        Duration::from_secs(5),
    )
    .unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    let t0 = Instant::now();
    stream.write_all(request.as_bytes()).unwrap();
    let response = read_response(&mut stream);
    (response, t0.elapsed().as_secs_f64())
}

fn count_outcome(path: &str, outcome: &str) -> usize {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .matches(&format!("\"outcome\":\"{outcome}\""))
        .count()
}

/// Every outcome that counts against a backend.
fn count_faults(path: &str) -> usize {
    ["stream-broke", "stream-cut", "bad-response", "http-error", "client-gone"]
        .iter()
        .map(|outcome| count_outcome(path, outcome))
        .sum()
}

fn wait_router(port: u16) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        if let Ok(mut s) = TcpStream::connect_timeout(&format!("127.0.0.1:{}", port).parse().unwrap(), Duration::from_millis(300)) {
            s.set_read_timeout(Some(Duration::from_millis(800))).ok();
            let _ = s.write_all(b"GET /lac/status HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            if String::from_utf8_lossy(&buf).contains("200 OK") {
                return;
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
    panic!("router on :{} never became ready", port);
}

#[test]
fn relay_framing_regressions() {
    let _port_lock = support::PortLock::acquire();
    let tmp_home = std::env::temp_dir().join(format!("lac-test-relay-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_home);
    let _ = std::fs::create_dir_all(&tmp_home);
    let llama_port = spawn_backend();
    let healthy_port = spawn_healthy_backend();
    let dead_listener = TcpListener::bind(("127.0.0.1", 0)).expect("dead backend reservation");
    let dead_port = dead_listener.local_addr().expect("dead backend address").port();
    let router_bin = env!("CARGO_BIN_EXE_lac-router");
    let home_env = tmp_home.to_string_lossy().into_owned();
    let llama_env = llama_port.to_string();
    let healthy_env = healthy_port.to_string();
    let dead_env = dead_port.to_string();
    let router = support::spawn_router(
        router_bin,
        &[
            ("HOME", &home_env),
            ("LAC_BIND_ADDR", "127.0.0.1"),
            ("MLX_PORT", &dead_env),
            ("LAC_LLAMA_PORT", &llama_env),
            ("LAC_OLLAMA_PORT", &healthy_env),
            ("LAC_BACKEND", "auto"),
            ("LAC_ROUTER_KEEPALIVE_SECS", "1"),
        ],
    );
    let router_port = router.port;
    wait_router(router_port);

    // 1. Chunked SSE: byte-identical AND incremental.
    let (raw, times, _) = probe(router_port, "sse-chunked");
    let head_end = raw.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4).unwrap_or(raw.len());
    let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
    assert!(head.contains("200 OK"), "status: {}", head);
    assert!(head.to_lowercase().contains("transfer-encoding: chunked"), "framing preserved: head={} body={:?}", head, &String::from_utf8_lossy(&raw[head_end..]).to_string());
    let mut expected = Vec::new();
    for ev in ["data: A\n\n", "data: B\n\n", "data: C\n\n", "data: [DONE]\n\n"] {
        expected.extend_from_slice(format!("{:X}\r\n{}\r\n", ev.len(), ev).as_bytes());
    }
    expected.extend_from_slice(b"0\r\n\r\n");
    assert_eq!(&raw[head_end..], &expected[..], "chunk wire byte-identical");
    assert!(times.len() >= 3, "streamed incrementally, not buffered: {:?}", times);
    assert!(times[0].0 < 0.5, "first event prompt: {:?}", times);
    assert!(!raw[head_end..].windows(13).any(|w| w == b": keep-alive\n"), "no injection needed (backend kept talking)");

    let (raw_split, _, split_total) = probe(router_port, "sse-split");
    let split_head_end = raw_split
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(raw_split.len());
    let split_body = &raw_split[split_head_end..];
    let split_event = b"data: 0\r\n\r\n";
    let mut split_expected = Vec::new();
    split_expected.extend_from_slice(format!("{:X}\r", split_event.len()).as_bytes());
    split_expected.extend_from_slice(b"\n");
    split_expected.extend_from_slice(split_event);
    split_expected.extend_from_slice(b"\r\n0\r\n\r\n");
    assert_eq!(split_body, split_expected.as_slice());
    assert!(split_total < 2.0, "split chunk must not be truncated: {split_total}");

    let (raw_trailer, _, trailer_total) = probe(router_port, "sse-trailer");
    let trailer_text = String::from_utf8_lossy(&raw_trailer).to_string();
    assert!(trailer_text.contains("trailer-event"));
    assert!(trailer_total < 2.0, "trailer terminal must close promptly: {trailer_total}");
    assert!(!trailer_text.contains(": keep-alive"));

    // 2. Hold-open: complete CL response ends the relay promptly.
    let (raw2, _, total2) = probe(router_port, "hold-open");
    assert!(total2 < 3.0, "relay ends at Content-Length, not backend close: {:.2}s", total2);
    assert!(String::from_utf8_lossy(&raw2).contains("hold-open ok"));

    // 3a. SSE + silence: keep-alive comment present, event intact.
    let (raw3, _, _) = probe(router_port, "sse-slow");
    let body3 = String::from_utf8_lossy(&raw3).to_string();
    assert!(body3.contains(": keep-alive"), "SSE keep-alive alive: {}", &body3[..body3.len().min(200)]);
    assert!(body3.contains("slow-event"), "event intact: {}", &body3[body3.len().saturating_sub(200)..]);

    let (raw_chunked_slow, _, _) = probe(router_port, "sse-chunked-slow");
    let chunked_slow = String::from_utf8_lossy(&raw_chunked_slow).to_string();
    assert!(chunked_slow.contains("E\r\n: keep-alive\n\n\r\n"));
    assert!(chunked_slow.contains("chunked-slow-event"));

    let (raw_crlf_slow, _, _) = probe(router_port, "sse-crlf-slow");
    let crlf_slow = String::from_utf8_lossy(&raw_crlf_slow).to_string();
    assert!(crlf_slow.contains(": keep-alive"));
    assert!(crlf_slow.contains("crlf-event"));

    let (raw_mid_chunk, _, _) = probe(router_port, "sse-mid-chunk");
    let mid_chunk = String::from_utf8_lossy(&raw_mid_chunk).to_string();
    assert!(mid_chunk.contains("5\r\nhello\r\n0\r\n\r\n"));
    assert!(!mid_chunk.contains("E\r\n: keep-alive"));

    // 3b. JSON + silence: NO injection, body parses as JSON object tail.
    let (raw4, _, _) = probe(router_port, "json-slow");
    let text4 = String::from_utf8_lossy(&raw4).to_string();
    assert!(!text4.contains(": keep-alive"), "JSON body must stay clean: {}", text4);
    assert!(text4.contains("slow-json-ok"), "JSON body complete: {}", text4);

    let (cl_overrun, _, _) = probe(router_port, "cl-overrun");
    let cl_head_end = cl_overrun
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .unwrap_or(cl_overrun.len());
    assert_eq!(&cl_overrun[cl_head_end..], b"abc");
    assert!(!cl_overrun.windows(5).any(|window| window == b"EXTRA"));

    let (chunk_overrun, _, _) = probe(router_port, "chunk-overrun");
    let chunk_head_end = chunk_overrun
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .unwrap_or(chunk_overrun.len());
    assert_eq!(&chunk_overrun[chunk_head_end..], b"0\r\n\r\n");
    assert!(!chunk_overrun.windows(5).any(|window| window == b"EXTRA"));

    let (interim_final, _, _) = probe(router_port, "interim-final");
    let interim_text = String::from_utf8_lossy(&interim_final);
    assert!(interim_text.starts_with("HTTP/1.1 200 OK"));
    assert!(interim_text.contains("interim-final"));

    let (many_interims, _, _) = probe(router_port, "many-interims");
    assert!(String::from_utf8_lossy(&many_interims).contains("healthy-failover"));

    let (upgrade, _, _) = probe(router_port, "upgrade");
    assert!(String::from_utf8_lossy(&upgrade).starts_with("HTTP/1.1 501"));

    let (encoded_sse, _, _) = probe(router_port, "encoded-sse");
    let encoded_sse_text = String::from_utf8_lossy(&encoded_sse);
    assert!(encoded_sse_text.starts_with("HTTP/1.1 200 OK"));
    assert!(encoded_sse_text.contains("encoded-sse-event"));
    assert!(!encoded_sse_text.contains(": keep-alive"));

    let (transfer_coded_sse, _, _) = probe(router_port, "transfer-coded-sse");
    let transfer_coded_sse_text = String::from_utf8_lossy(&transfer_coded_sse);
    assert!(transfer_coded_sse_text.starts_with("HTTP/1.1 200 OK"));
    assert!(transfer_coded_sse_text.contains("transfer-coded-sse-event"));
    assert!(!transfer_coded_sse_text.contains(": keep-alive"));

    let (partial_event, _, _) = probe(router_port, "sse-partial-event");
    let partial_event_text = String::from_utf8_lossy(&partial_event);
    assert!(partial_event_text.contains("data: partial"));
    assert!(partial_event_text.contains(" event\n\n"));
    assert!(!partial_event_text.contains(": keep-alive"));

    let (partial_slow, _, _) = probe(router_port, "sse-partial-slow");
    let partial_slow_text = String::from_utf8_lossy(&partial_slow);
    assert!(partial_slow_text.contains("data: partial"));
    let slow_event_end = partial_slow_text.find(" event\n\n").expect("completed event");
    let keepalive = partial_slow_text
        .find(": keep-alive")
        .expect("keep-alive after completed event");
    assert!(keepalive > slow_event_end);

    let (reset_205, _, _) = probe(router_port, "reset-205");
    assert!(String::from_utf8_lossy(&reset_205).contains("healthy-failover"));

    let (bad_response, _, _) = probe(router_port, "bad-response");
    assert!(String::from_utf8_lossy(&bad_response).contains("healthy-failover"));
    let (after_bad_response, _, _) = probe(router_port, "json-slow");
    assert!(String::from_utf8_lossy(&after_bad_response).contains("slow-json-ok"));

    let (trailer_delay, _, trailer_delay_total) = probe(router_port, "sse-trailer-delay");
    let trailer_delay_text = String::from_utf8_lossy(&trailer_delay);
    assert!(trailer_delay_text.contains("0\r\n\r\n"));
    assert!(!trailer_delay_text.contains("E\r\n: keep-alive"));
    assert!(trailer_delay_total < 4.0);

    let (partial_size, _, partial_size_total) = probe(router_port, "sse-partial-size");
    let partial_size_text = String::from_utf8_lossy(&partial_size);
    assert!(partial_size_text.contains("1\r\na\r\n0\r\n\r\n"));
    assert!(!partial_size_text.contains("E\r\n: keep-alive"));
    assert!(partial_size_total < 4.0);

    let unsupported_request = b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\nTransfer-Encoding: gzip\r\nConnection: close\r\n\r\nhi";
    assert!(probe_raw(router_port, unsupported_request).starts_with(b"HTTP/1.1 400"));

    let malformed_early = b"GET /lac/status HTTP/1.1\r\nX-Bad: bad\x0bvalue\r\n\r\n";
    assert!(probe_raw(router_port, malformed_early).starts_with(b"HTTP/1.1 400"));

    let withheld_status = b"GET /lac/status HTTP/1.1\r\nHost: x\r\nContent-Length: 100\r\nConnection: close\r\n\r\n";
    assert!(probe_header_only(router_port, withheld_status).starts_with(b"HTTP/1.1 200"));
    let withheld_options = b"OPTIONS /v1/chat HTTP/1.1\r\nHost: x\r\nContent-Length: 100\r\n\r\n";
    assert!(probe_header_only(router_port, withheld_options).starts_with(b"HTTP/1.1 204"));
    let withheld_models = b"GET /v1/models HTTP/1.1\r\nHost: x\r\nContent-Length: 100\r\nConnection: close\r\n\r\n";
    assert!(probe_header_only(router_port, withheld_models).starts_with(b"HTTP/1.1 200"));

    let bodyless_post = b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
    assert!(probe_raw(router_port, bodyless_post).starts_with(b"HTTP/1.1 400"));

    let oversized = b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Length: 16777217\r\nConnection: close\r\n\r\n";
    assert!(probe_raw(router_port, oversized).starts_with(b"HTTP/1.1 413"));

    let unframed_body = b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n{\"unframed\":true}";
    let unframed_response = probe_raw(router_port, unframed_body);
    assert!(String::from_utf8_lossy(&unframed_response).contains("requires Content-Length"));
    let delayed_unframed_headers = b"PUT /v1/chat/completions HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
    let delayed_unframed_body = b"{\"delayed\":true}";
    let delayed_unframed = probe_raw_parts(
        router_port,
        delayed_unframed_headers,
        delayed_unframed_body,
    );
    assert!(String::from_utf8_lossy(&delayed_unframed).starts_with("HTTP/1.1 400"));

    let pipeline = b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nConnection: close\r\n\r\nGET /v1/models HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
    assert!(probe_raw(router_port, pipeline).starts_with(b"HTTP/1.1 400"));

    let split_headers = b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\nConnection: close\r\n\r\n";
    let split_tail = b"hiGET /v1/models HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
    assert!(probe_raw_parts(router_port, split_headers, split_tail).starts_with(b"HTTP/1.1 400"));

    let chunk_pipeline = b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2\r\nhi\r\n0\r\n\r\nGET /v1/models HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer second-secret\r\nConnection: close\r\n\r\n";
    assert!(probe_raw(router_port, chunk_pipeline).starts_with(b"HTTP/1.1 400"));

    let split_chunked_headers = b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
    let split_chunked_body = b"5\r\nhello\r\n0\r\n\r\n";
    assert!(probe_raw_parts(
        router_port,
        split_chunked_headers,
        split_chunked_body
    )
    .starts_with(b"HTTP/1.1 200"));

    let mut split_mid_first = split_chunked_headers.to_vec();
    split_mid_first.extend_from_slice(b"5\r\nhel");
    assert!(probe_raw_parts(
        router_port,
        &split_mid_first,
        b"lo\r\n0\r\n\r\n"
    )
    .starts_with(b"HTTP/1.1 200"));

    let malformed_chunked = b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nzz\r\nhi\r\n";
    assert!(probe_raw(router_port, malformed_chunked).starts_with(b"HTTP/1.1 400"));

    let (short_cl, _, _) = probe(router_port, "short-cl");
    assert!(String::from_utf8_lossy(&short_cl).contains("abc"));
    let (short_chunk, _, _) = probe(router_port, "short-chunk");
    assert!(String::from_utf8_lossy(&short_chunk).contains("ab"));

    // EOF-delimited response cut cleanly at a message boundary: relayed in
    // full and counted as a success. The body arriving is not the point —
    // a clean FIN never reaches the stream-error arms — so assert the log
    // gained exactly one proxied line and no stream-* line.
    let usage_path = format!("{}/.lac/router-usage.jsonl", tmp_home.display());
    let proxied_before = count_outcome(&usage_path, "proxied");
    let faults_before = count_faults(&usage_path);
    let (eof_json, _, _) = probe(router_port, "eof-json");
    assert!(
        String::from_utf8_lossy(&eof_json).contains("{\"eof\":\"ok\"}"),
        "EOF-delimited body relayed: {:?}",
        String::from_utf8_lossy(&eof_json)
    );
    assert_eq!(
        count_outcome(&usage_path, "proxied") - proxied_before,
        1,
        "clean EOF is one success"
    );
    assert_eq!(
        count_faults(&usage_path) - faults_before,
        0,
        "clean EOF is not a fault: {:?}",
        std::fs::read_to_string(&usage_path).unwrap_or_default()
    );

    // Strict request-line/header grammar: bare LF, obs-fold, oversized.
    let bare_lf = b"GET /v1/models HTTP/1.1\r\nHost: x\nX-Extra: y\r\n\r\n";
    assert!(probe_raw(router_port, bare_lf).starts_with(b"HTTP/1.1 400"));
    let obs_fold = b"GET /v1/models HTTP/1.1\r\nHost: x\r\nX-Trace:\r\n continued\r\nConnection: close\r\n\r\n";
    assert!(probe_raw(router_port, obs_fold).starts_with(b"HTTP/1.1 400"));
    let mut oversized_headers = b"GET /v1/models HTTP/1.1\r\nHost: x\r\nX-Pad: ".to_vec();
    oversized_headers.extend(std::iter::repeat_n(b'a', 70_000));
    oversized_headers.extend_from_slice(b"\r\n\r\n");
    assert!(probe_raw(router_port, &oversized_headers).starts_with(b"HTTP/1.1 400"));

    // Both framing headers at once is rejected, not silently resolved.
    let ambiguous = b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nhi";
    assert!(String::from_utf8_lossy(&probe_raw(router_port, ambiguous))
        .contains("ambiguous request framing"));

    // Expect: 100-continue is answered by the router, then relayed.
    let expect_response = probe_expect_continue(router_port);
    assert!(
        String::from_utf8_lossy(&expect_response).contains("slow-json-ok"),
        "body after 100 Continue relayed: {:?}",
        String::from_utf8_lossy(&expect_response)
    );

    // HEAD: headers (with Content-Length) and no body, without waiting for
    // a payload the backend only sends after its delay.
    let (head_response, head_total) =
        probe_timed(router_port, b"HEAD /v1/models HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    assert!(head_response.starts_with(b"HTTP/1.1 200"));
    let head_end = head_response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(head_response.len());
    assert!(
        String::from_utf8_lossy(&head_response).to_lowercase().contains("content-length:"),
        "HEAD keeps the declared length: {:?}",
        String::from_utf8_lossy(&head_response)
    );
    assert!(
        head_response[head_end..].is_empty(),
        "HEAD must carry no body: {:?}",
        &head_response[head_end..]
    );
    assert!(head_total < 2.0, "HEAD must not await the payload: {head_total}s");

    // Chunked upload past the byte ceiling: 413, not a dropped connection.
    let paced_413 = probe_paced_oversized_chunked(router_port);
    assert!(
        String::from_utf8_lossy(&paced_413).starts_with("HTTP/1.1 413"),
        "oversized chunked upload rejected: {:?}",
        &String::from_utf8_lossy(&paced_413)[..String::from_utf8_lossy(&paced_413)
            .len()
            .min(120)]
    );

    let gone_body = r#"{"model":"qwen3.8-27b","scenario":"client-gone"}"#;
    let gone_request = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        gone_body.len(),
        gone_body
    );
    probe_client_gone(router_port, gone_request.as_bytes());
    thread::sleep(Duration::from_secs(3));

    // A reset mid-response is charged, and the upstream is loopback, so a
    // reset means the backend process went away. Three in a row must take
    // it out of rotation. Runs last: the breaker is open afterwards.
    for _ in 0..3 {
        let (_reset, reset_total) = probe_chunked_reset(router_port);
        assert!(reset_total < 5.0, "reset must not stall the relay: {reset_total}s");
    }
    let (after_resets, _, _) = probe(router_port, "hold-open");
    let after_resets_text = String::from_utf8_lossy(&after_resets).to_string();
    assert!(
        after_resets_text.contains("healthy-failover") && !after_resets_text.contains("hold-open ok"),
        "the reset backend must be ejected, with traffic failing over: {after_resets_text}"
    );

    let usage = std::fs::read_to_string(format!("{}/.lac/router-usage.jsonl", tmp_home.display()))
        .expect("usage log exists");
    assert_eq!(usage.matches("\"outcome\":\"stream-broke\"").count(), 2);
    assert_eq!(usage.matches("\"outcome\":\"unsupported-upgrade\"").count(), 1);
    assert_eq!(usage.matches("\"outcome\":\"bad-response\"").count(), 3);
    assert_eq!(usage.matches("\"outcome\":\"client-gone\"").count(), 1);
    assert_eq!(usage.matches("\"outcome\":\"stream-cut\"").count(), 3);
    assert!(usage.matches("\"outcome\":\"proxied\"").count() >= 1);

    // Budgets are env-tunable, so a second router with 1s budgets can prove
    // the timeouts actually fire — and that an EOF-delimited stream cut by
    // the budget is NOT charged as a backend fault.
    let budget_home = std::env::temp_dir().join(format!("lac-test-relay-budget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&budget_home);
    let _ = std::fs::create_dir_all(&budget_home);
    let budget_home_env = budget_home.to_string_lossy().into_owned();
    let budget_router = support::spawn_router(
        router_bin,
        &[
            ("HOME", &budget_home_env),
            ("LAC_BIND_ADDR", "127.0.0.1"),
            ("MLX_PORT", &dead_env),
            ("LAC_LLAMA_PORT", &llama_env),
            ("LAC_OLLAMA_PORT", &dead_env),
            ("LAC_BACKEND", "auto"),
            ("LAC_ROUTER_KEEPALIVE_SECS", "1"),
            ("LAC_ROUTER_STREAM_BUDGET_SECS", "1"),
            ("LAC_ROUTER_HEADER_BUDGET_SECS", "1"),
            ("LAC_ROUTER_BODY_IDLE_SECS", "1"),
        ],
    );
    let budget_port = budget_router.port;
    wait_router(budget_port);

    let (slow_headers, slow_headers_total) = probe_timed(
        budget_port,
        b"GET /v1/models HTTP/1.1\r\nHost: x\r\n",
    );
    assert!(
        String::from_utf8_lossy(&slow_headers).starts_with("HTTP/1.1 400"),
        "stalled headers rejected: {:?}",
        String::from_utf8_lossy(&slow_headers)
    );
    assert!(
        slow_headers_total < 4.0,
        "header budget bounded the wait: {slow_headers_total}s"
    );

    let body = b"{\"partial\":true}";
    let mut stalled_body = b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Length: 100\r\nConnection: close\r\n\r\n".to_vec();
    stalled_body.extend_from_slice(body);
    let (stalled, stalled_total) = probe_timed(budget_port, &stalled_body);
    assert!(
        String::from_utf8_lossy(&stalled).starts_with("HTTP/1.1 400"),
        "stalled body rejected: {:?}",
        String::from_utf8_lossy(&stalled)
    );
    assert!(stalled_total < 4.0, "body idle timeout bounded the wait: {stalled_total}s");

    // Three streams the backend never finishes. Each is cut by the budget,
    // and silence for the whole budget IS the backend's fault: three of
    // them take it out of rotation, so later traffic stops piling onto it.
    let mut cuts_total = 0.0;
    for _ in 0..3 {
        let (raw, _, total) = probe(budget_port, "sse-slow");
        cuts_total += total;
        assert!(
            String::from_utf8_lossy(&raw).starts_with("HTTP/1.1 200"),
            "stream cut after headers: {:?}",
            String::from_utf8_lossy(&raw)
        );
        assert!(
            !String::from_utf8_lossy(&raw).contains("slow-event"),
            "budget cut the stream before the backend's late event"
        );
        assert!(total < 4.0, "stream budget bounded the wait: {total}s");
    }
    assert!(cuts_total < 12.0, "budget cuts stay prompt: {cuts_total}s");

    let (after_cuts, _, _) = probe(budget_port, "hold-open");
    assert!(
        String::from_utf8_lossy(&after_cuts).starts_with("HTTP/1.1 503"),
        "a backend that went silent must leave rotation: {:?}",
        String::from_utf8_lossy(&after_cuts)
    );

    let budget_usage = std::fs::read_to_string(format!("{}/.lac/router-usage.jsonl", budget_home.display()))
        .expect("budget usage log exists");
    assert_eq!(budget_usage.matches("\"outcome\":\"stream-broke\"").count(), 3);
    assert_eq!(budget_usage.matches("\"outcome\":\"stream-cut\"").count(), 0);

    drop(budget_router);
    drop(router);
    let _ = std::fs::remove_dir_all(&budget_home);
    let _ = std::fs::remove_dir_all(&tmp_home);
}
