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
                // where sse-chunked fell through to json-slow.
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
                let scenario = if text.contains("sse-mid-chunk") {
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
                } else if text.contains("sse-slow") {
                    "sse-slow"
                } else {
                    "json-slow"
                };
                eprintln!("[fake] scenario={} body_len={}", scenario, body.len());
                match scenario {
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
    let _ = std::fs::create_dir_all(&tmp_home);
    let llama_port = spawn_backend();
    let dead_listener = TcpListener::bind(("127.0.0.1", 0)).expect("dead backend reservation");
    let dead_port = dead_listener.local_addr().expect("dead backend address").port();
    let router_bin = env!("CARGO_BIN_EXE_lac-router");
    let home_env = tmp_home.to_string_lossy().into_owned();
    let llama_env = llama_port.to_string();
    let dead_env = dead_port.to_string();
    let router = support::spawn_router(
        router_bin,
        &[
            ("HOME", &home_env),
            ("LAC_BIND_ADDR", "127.0.0.1"),
            ("MLX_PORT", &dead_env),
            ("LAC_LLAMA_PORT", &llama_env),
            ("LAC_OLLAMA_PORT", &dead_env),
            ("LAC_BACKEND", "llama"),
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

    let (raw_mid_chunk, _, _) = probe(router_port, "sse-mid-chunk");
    let mid_chunk = String::from_utf8_lossy(&raw_mid_chunk).to_string();
    assert!(mid_chunk.contains("5\r\nhello\r\n0\r\n\r\n"));
    assert!(!mid_chunk.contains("E\r\n: keep-alive"));

    // 3b. JSON + silence: NO injection, body parses as JSON object tail.
    let (raw4, _, _) = probe(router_port, "json-slow");
    let text4 = String::from_utf8_lossy(&raw4).to_string();
    assert!(!text4.contains(": keep-alive"), "JSON body must stay clean: {}", text4);
    assert!(text4.contains("slow-json-ok"), "JSON body complete: {}", text4);

    let (short_cl, _, _) = probe(router_port, "short-cl");
    assert!(String::from_utf8_lossy(&short_cl).contains("abc"));
    let (short_chunk, _, _) = probe(router_port, "short-chunk");
    assert!(String::from_utf8_lossy(&short_chunk).contains("ab"));
    let usage = std::fs::read_to_string(format!("{}/.lac/router-usage.jsonl", tmp_home.display()))
        .expect("usage log exists");
    assert_eq!(usage.matches("\"outcome\":\"stream-broke\"").count(), 2);

    drop(router);
    let _ = std::fs::remove_dir_all(&tmp_home);
}
