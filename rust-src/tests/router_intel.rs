//! Router intelligence integration tests.
//!
//! Spins up the real `lac-router` binary against in-process fake backends
//! on localhost and asserts merge, failover, model-hint routing, fastest
//! mode, and the usage log. One sequential #[test]: process env is global,
//! Child environments are explicit; scenarios run in order inside a single test.

mod support;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

fn read_headers(stream: &mut TcpStream) -> Vec<u8> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
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

fn read_body(stream: &mut TcpStream, already: &[u8], len: usize) -> Vec<u8> {
    let head_end = already
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(already.len());
    let mut body = already[head_end.min(already.len())..].to_vec();
    let mut chunk = [0u8; 4096];
    while body.len() < len {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    body.truncate(len);
    body
}

fn content_length(head: &[u8]) -> usize {
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

struct Fake {
    hits_models: Arc<AtomicUsize>,
    hits_chat: Arc<AtomicUsize>,
    down: Arc<AtomicBool>,
    http_fail: Arc<AtomicBool>,
}

/// Serve a canned backend until the process exits (detached threads).
fn spawn_fake(ids: Vec<&'static str>, chat_delay_ms: u64) -> (Fake, u16) {
    let hits_models = Arc::new(AtomicUsize::new(0));
    let hits_chat = Arc::new(AtomicUsize::new(0));
    let down = Arc::new(AtomicBool::new(false));
    let hm = Arc::clone(&hits_models);
    let hc = Arc::clone(&hits_chat);
    let dn = Arc::clone(&down);
    let http_fail = Arc::new(AtomicBool::new(false));
    let hf = Arc::clone(&http_fail);
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("fake backend bind");
    let port = listener.local_addr().expect("fake backend address").port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let hm = Arc::clone(&hm);
            let hc = Arc::clone(&hc);
            let dn = Arc::clone(&dn);
            let hf = Arc::clone(&hf);
            let ids = ids.clone();
            thread::spawn(move || {
                let mut s = stream;
                if dn.load(Ordering::SeqCst) {
                    return; // flap down: accept then hang up
                }
                let req = read_headers(&mut s);
                let head = String::from_utf8_lossy(&req);
                let path = head
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                let is_models = path == "/v1/models" || path.starts_with("/v1/models?");
                if !is_models {
                    let _ = read_body(&mut s, &req, content_length(&req));
                }
                if dn.load(Ordering::SeqCst) {
                    return;
                }
                let (body, code) = if is_models {
                    hm.fetch_add(1, Ordering::SeqCst);
                    let items: Vec<String> =
                        ids.iter().map(|id| format!("{{\"id\":\"{}\"}}", id)).collect();
                    (
                        format!("{{\"object\":\"list\",\"data\":[{}]}}", items.join(",")),
                        "200 OK",
                    )
                } else if path.starts_with("/v1/chat/completions") {
                    hc.fetch_add(1, Ordering::SeqCst);
                    if chat_delay_ms > 0 {
                        thread::sleep(Duration::from_millis(chat_delay_ms));
                    }
                    if hf.load(Ordering::SeqCst) {
                        ("{\"error\":\"backend failure\"}".to_string(), "500 Internal Server Error")
                    } else {
                        (
                            "{\"id\":\"t1\",\"object\":\"chat.completion\",\"choices\":[{\"message\":{\"content\":\"hello test\"}}]}".to_string(),
                            "200 OK",
                        )
                    }
                } else {
                    ("{\"error\":\"not found\"}".to_string(), "404 Not Found")
                };
                let resp = format!(
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    code,
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            });
        }
    });
    (
        Fake {
            hits_models,
            hits_chat,
            down,
            http_fail,
        },
        port,
    )
}

/// Raw HTTP over one connection; headers+body sent in a single write so
/// the router's zero-copy model sniff sees the full picture.
fn raw_request(port: u16, text: &str) -> String {
    let mut s = TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", port).parse().unwrap(),
        Duration::from_secs(5),
    )
    .unwrap();
    let _ = s.set_read_timeout(Some(Duration::from_secs(15)));
    s.write_all(text.as_bytes()).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    String::from_utf8_lossy(&out).to_string()
}

fn get(port: u16, path: &str) -> (u16, String) {
    let resp = raw_request(port, &format!("GET {} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", path));
    split_response(&resp)
}

fn post_chat(port: u16, model: &str) -> (u16, String) {
    let body = format!("{{\"model\":\"{}\",\"messages\":[{{\"role\":\"user\",\"content\":\"hi\"}}]}}", model);
    let req = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    split_response(&raw_request(port, &req))
}

fn post_chat_split_body(port: u16, model: &str) -> (u16, String) {
    let body = format!(
        "{{\"model\":\"{}\",\"messages\":[{{\"role\":\"user\",\"content\":\"{}\"}}]}}",
        model,
        "x".repeat(32_000)
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("gateway connection");
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .expect("gateway timeout");
    let request_head = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(request_head.as_bytes())
        .expect("request head");
    thread::sleep(Duration::from_millis(100));
    let split = body.len() / 2;
    stream
        .write_all(&body.as_bytes()[..split])
        .expect("request body prefix");
    thread::sleep(Duration::from_millis(100));
    stream
        .write_all(&body.as_bytes()[split..])
        .expect("request body suffix");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).expect("gateway response");
    split_response(&String::from_utf8_lossy(&response))
}

fn split_response(resp: &str) -> (u16, String) {
    let code = resp
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = resp.find("\r\n\r\n").map(|i| resp[i + 4..].to_string()).unwrap_or_default();
    (code, body)
}

fn wait_router(port: u16) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        if let Ok(mut s) = TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", port).parse().unwrap(),
            Duration::from_millis(300),
        ) {
            let _ = s.set_read_timeout(Some(Duration::from_millis(800)));
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
fn router_intelligence() {
    let _port_lock = support::PortLock::acquire();
    // Isolate HOME: the router persists its backend choice and usage log
    // under ~/.lac — the test must never rewrite the operator's files.
    // (All scenarios run sequentially in this one test; env is global.)
    let tmp_home = std::env::temp_dir().join(format!("lac-test-home-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp_home);
    let (mlx, mlx_port) = spawn_fake(vec!["t-m1", "t-dup"], 0);
    let (llama, llama_port) = spawn_fake(vec!["t-m2", "t-dup"], 0);
    let dead_listener = TcpListener::bind(("127.0.0.1", 0)).expect("ollama dead reservation");
    let ollama_port = dead_listener.local_addr().expect("ollama dead address").port();

    let router_bin = env!("CARGO_BIN_EXE_lac-router");
    let home_env = tmp_home.to_string_lossy().into_owned();
    let mlx_env = mlx_port.to_string();
    let llama_env = llama_port.to_string();
    let ollama_env = ollama_port.to_string();
    let router = support::spawn_router(
        router_bin,
        &[
            ("HOME", &home_env),
            ("LAC_BIND_ADDR", "127.0.0.1"),
            ("MLX_PORT", &mlx_env),
            ("LAC_LLAMA_PORT", &llama_env),
            ("LAC_OLLAMA_PORT", &ollama_env),
        ],
    );
    let router_port = router.port;
    wait_router(router_port);

    // 1. Merged catalog: union with first-wins dedupe.
    let (code, body) = get(router_port, "/v1/models");
    assert_eq!(code, 200, "models body: {}", body);
    assert!(body.contains("t-m1"), "mlx model present: {}", body);
    assert!(body.contains("t-m2"), "llama model present: {}", body);
    assert_eq!(body.matches("t-dup").count(), 1, "dup deduped: {}", body);

    // 2. Model hint: t-m2 was mapped to llama by the merge above.
    for _ in 0..3 {
        let (code, _) = post_chat(router_port, "t-m2");
        assert_eq!(code, 200);
        if llama.hits_chat.load(Ordering::SeqCst) >= 1 {
            break;
        }
    }
    assert!(
        llama.hits_chat.load(Ordering::SeqCst) >= 1,
        "hinted model reaches its backend"
    );

    // 3. Failover: flap mlx down, wait out the 1s health cache, re-probe.
    mlx.down.store(true, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(2000));
    let before = llama.hits_chat.load(Ordering::SeqCst);
    let (code, _) = post_chat(router_port, "t-m1");
    assert_eq!(code, 200);
    assert!(
        llama.hits_chat.load(Ordering::SeqCst) > before,
        "traffic fails over to llama"
    );
    mlx.down.store(false, Ordering::SeqCst);

    mlx.http_fail.store(true, Ordering::SeqCst);
    let before_http_failover = llama.hits_chat.load(Ordering::SeqCst);
    let (code, body) = post_chat(router_port, "unmapped-model");
    assert_eq!(code, 200, "HTTP 500 backend must fail over: {body}");
    assert!(llama.hits_chat.load(Ordering::SeqCst) > before_http_failover);
    let before_split = llama.hits_chat.load(Ordering::SeqCst);
    let (code, body) = post_chat_split_body(router_port, "large-unmapped-model");
    assert_eq!(code, 200, "split request body must fail over intact: {body}");
    assert!(llama.hits_chat.load(Ordering::SeqCst) > before_split);
    mlx.http_fail.store(false, Ordering::SeqCst);

    // 4. Fastest mode + stats surface.
    let (code, _) = get(router_port, "/lac/switch?target=fastest");
    assert_eq!(code, 200);
    thread::sleep(Duration::from_millis(2000));
    let (code, status) = get(router_port, "/lac/status");
    assert_eq!(code, 200);
    assert!(status.contains("\"preferred\": \"fastest\""), "{}", status);
    assert!(status.contains("\"ewma_ms\""), "stats block: {}", status);
    assert!(status.contains("\"models_mapped\""), "{}", status);
    drop(mlx.hits_models);

    let usage = std::fs::read_to_string(tmp_home.join(".lac/router-usage.jsonl"))
        .expect("usage log exists");
    assert!(usage.contains("\"outcome\":\"proxied\""), "{}", &usage[usage.len().saturating_sub(400)..]);

    drop(router);
    let _ = std::fs::remove_dir_all(&tmp_home);
}
