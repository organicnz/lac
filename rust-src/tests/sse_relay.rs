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
//! One sequential #[test]: process env is global, scenarios run in order.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
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
fn spawn_backend(port: u16) {
    thread::spawn(move || {
        let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
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
                let scenario = if text.contains("sse-chunked") {
                    "sse-chunked"
                } else if text.contains("hold-open") {
                    "hold-open"
                } else if text.contains("sse-slow") {
                    "sse-slow"
                } else {
                    "json-slow"
                };
                eprintln!("[fake] scenario={} body_len={}", scenario, body.len());
                match scenario {
                    "sse-chunked" => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
                        for ev in ["data: A\n\n", "data: B\n\n", "data: C\n\n", "data: [DONE]\n\n"] {
                            let _ = s.write_all(format!("{:X}\r\n{}\r\n", ev.len(), ev).as_bytes());
                            thread::sleep(Duration::from_millis(100));
                        }
                        let _ = s.write_all(b"0\r\n\r\n");
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
}

struct RouterChild(std::process::Child);
impl Drop for RouterChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
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
    let tmp_home = std::env::temp_dir().join(format!("lac-test-relay-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp_home);
    let orig_home = std::env::var("HOME").ok();

    let llama_port = free_port();
    let dead_port = free_port(); // nothing listens: mlx/ollama stay down
    let router_port = free_port();

    unsafe {
        std::env::set_var("HOME", &tmp_home);
        std::env::set_var("MLX_PORT", dead_port.to_string());
        std::env::set_var("LAC_LLAMA_PORT", llama_port.to_string());
        std::env::set_var("LAC_OLLAMA_PORT", dead_port.to_string());
        std::env::set_var("LAC_ROUTER_PORT", router_port.to_string());
        std::env::set_var("LAC_BACKEND", "llama");
        std::env::set_var("LAC_ROUTER_KEEPALIVE_SECS", "1");
    }

    spawn_backend(llama_port);
    let router_bin = env!("CARGO_BIN_EXE_lac-router");
    let child = std::process::Command::new(router_bin)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("router binary runs");
    let _guard = RouterChild(child);
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

    // 2. Hold-open: complete CL response ends the relay promptly.
    let (raw2, _, total2) = probe(router_port, "hold-open");
    assert!(total2 < 3.0, "relay ends at Content-Length, not backend close: {:.2}s", total2);
    assert!(String::from_utf8_lossy(&raw2).contains("hold-open ok"));

    // 3a. SSE + silence: keep-alive comment present, event intact.
    let (raw3, _, _) = probe(router_port, "sse-slow");
    let body3 = String::from_utf8_lossy(&raw3).to_string();
    assert!(body3.contains(": keep-alive"), "SSE keep-alive alive: {}", &body3[..body3.len().min(200)]);
    assert!(body3.contains("slow-event"), "event intact: {}", &body3[body3.len().saturating_sub(200)..]);

    // 3b. JSON + silence: NO injection, body parses as JSON object tail.
    let (raw4, _, _) = probe(router_port, "json-slow");
    let text4 = String::from_utf8_lossy(&raw4).to_string();
    assert!(!text4.contains(": keep-alive"), "JSON body must stay clean: {}", text4);
    assert!(text4.contains("slow-json-ok"), "JSON body complete: {}", text4);

    unsafe {
        match orig_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
    }
    let _ = std::fs::remove_dir_all(&tmp_home);
}
