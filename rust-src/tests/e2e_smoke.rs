mod support;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

struct Fixture {
    port: u16,
    stop: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
    saw_authorization: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Fixture {
    fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("fixture listener");
        listener.set_nonblocking(true).expect("fixture nonblocking");
        let port = listener.local_addr().expect("fixture address").port();
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let saw_authorization = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = Arc::clone(&stop);
            let requests = Arc::clone(&requests);
            let saw_authorization = Arc::clone(&saw_authorization);
            thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let requests = Arc::clone(&requests);
                            let saw_authorization = Arc::clone(&saw_authorization);
                            thread::spawn(move || {
                                serve(stream, requests, saw_authorization);
                            });
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Self {
            port,
            stop,
            requests,
            saw_authorization,
            thread: Some(thread),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(mut stream: TcpStream, requests: Arc<AtomicUsize>, saw_authorization: Arc<AtomicBool>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut request = Vec::new();
    let mut buffer = [0u8; 4096];
    let head_end = loop {
        match stream.read(&mut buffer) {
            Ok(0) => return,
            Ok(n) => {
                request.extend_from_slice(&buffer[..n]);
                if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                    break end + 4;
                }
                if request.len() > 64 * 1024 {
                    return;
                }
            }
            Err(_) => return,
        }
    };
    let head = String::from_utf8_lossy(&request[..head_end]).to_lowercase();
    if head.lines().any(|line| {
        line.split_once(':')
            .map(|(name, _)| {
                let name = name.trim();
                name.eq_ignore_ascii_case("authorization")
                    || name.eq_ignore_ascii_case("proxy-authorization")
            })
            .unwrap_or(false)
    }) {
        saw_authorization.store(true, Ordering::Release);
    }
    let content_length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while request.len() < head_end + content_length {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => request.extend_from_slice(&buffer[..n]),
            Err(_) => return,
        }
    }
    requests.fetch_add(1, Ordering::AcqRel);
    let first_line = head.lines().next().unwrap_or_default();
    let path = first_line.split_whitespace().nth(1).unwrap_or("/");
    let (status, content_type, body) = if path.starts_with("/v1/models") {
        (
            "200 OK",
            "application/json",
            r#"{"object":"list","data":[{"id":"e2e-model"}]}"#.to_string(),
        )
    } else if path.starts_with("/v1/chat/completions") {
        (
            "200 OK",
            "application/json",
            r#"{"choices":[{"message":{"role":"assistant","content":"E2E_SENTINEL"}}]}"#.to_string(),
        )
    } else {
        (
            "404 Not Found",
            "application/json",
            r#"{"error":"not found"}"#.to_string(),
        )
    };
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        content_type,
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
}

fn test_binary(name: &str) -> String {
    if let Ok(directory) = std::env::var("LAC_E2E_BIN_DIR") {
        return PathBuf::from(directory).join(name).to_string_lossy().into_owned();
    }
    match name {
        "lac" => env!("CARGO_BIN_EXE_lac").to_string(),
        "lac-router" => env!("CARGO_BIN_EXE_lac-router").to_string(),
        other => panic!("unknown test binary {other}"),
    }
}


fn request(port: u16, method: &str, path: &str, body: &str) -> (u16, String) {
    let address = format!("127.0.0.1:{}", port);
    let mut stream = TcpStream::connect(&address).expect("gateway connection");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("gateway read timeout");
    let request = format!(
        "{} {} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        method,
        path,
        body.len(),
        body
    );
    stream.write_all(request.as_bytes()).expect("gateway request");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).expect("gateway response");
    let text = String::from_utf8_lossy(&response).to_string();
    let status = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let body = text
        .find("\r\n\r\n")
        .map(|index| text[index + 4..].to_string())
        .unwrap_or(text);
    (status, body)
}

fn request_without_length(port: u16, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("gateway connection");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("gateway timeout");
    stream
        .write_all(
            format!("GET {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n", path)
                .as_bytes(),
        )
        .expect("gateway request");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).expect("gateway response");
    let text = String::from_utf8_lossy(&response).to_string();
    let status = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let body = text
        .find("\r\n\r\n")
        .map(|index| text[index + 4..].to_string())
        .unwrap_or(text);
    (status, body)
}

fn wait_for_router(port: u16, token: Option<&str>) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            let auth = token
                .map(|value| format!("Authorization: Bearer {}\r\n", value))
                .unwrap_or_default();
            let request = format!(
                "GET /lac/status HTTP/1.1\r\nHost: localhost\r\n{}Connection: close\r\n\r\n",
                auth
            );
            if stream.write_all(request.as_bytes()).is_ok() {
                let mut response = Vec::new();
                if stream.read_to_end(&mut response).is_ok()
                    && String::from_utf8_lossy(&response).contains("200 OK")
                {
                    return;
                }
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("router did not become ready");
}

fn wait_for_models(port: u16, token: Option<&str>) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(800)));
            let auth = token
                .map(|value| format!("Authorization: Bearer {}\r\n", value))
                .unwrap_or_default();
            let request = format!(
                "GET /v1/models HTTP/1.1\r\nHost: localhost\r\n{}Connection: close\r\n\r\n",
                auth
            );
            if stream.write_all(request.as_bytes()).is_ok() {
                let mut response = Vec::new();
                if stream.read_to_end(&mut response).is_ok()
                    && response.starts_with(b"HTTP/1.1 200")
                {
                    return;
                }
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("gateway models endpoint on :{} never became ready", port);
}

fn run_cli(binary: &str, home: &PathBuf, args: &[&str], extra_env: &[(&str, &str)]) -> (bool, String) {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .args(args)
        .env("HOME", home)
        .env("LAC_ROUTER_PORT", extra_env.first().map(|(_, value)| *value).unwrap_or("0"))
        .env("LAC_BIND_ADDR", "127.0.0.1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in extra_env.iter().skip(1) {
        command.env(key, value);
    }
    let output = command.output().expect("CLI process");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

#[test]
fn process_level_gateway_and_cli_smoke() {
    let _port_lock = support::PortLock::acquire();
    let fixture = Fixture::start();
    let dead_listener = TcpListener::bind(("127.0.0.1", 0)).expect("dead backend reservation");
    let dead_port = dead_listener.local_addr().expect("dead backend address").port();
    let home = std::env::temp_dir().join(format!(
        "lac-e2e-home-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&home).expect("temporary home");
    let router_bin = test_binary("lac-router");
    let home_env = home.to_string_lossy().into_owned();
    let dead_env = dead_port.to_string();
    let llama_env = fixture.port.to_string();
    let router = support::spawn_router(
        &router_bin,
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
    wait_for_router(router_port, None);
    wait_for_models(router_port, None);

    let (status, body) = request(router_port, "GET", "/lac/status", "");
    assert_eq!(status, 200);
    assert!(body.contains(&format!("\"port\": {}", fixture.port)));
    assert!(body.contains("\"up\": true"));

    let (status, body) = request(router_port, "GET", "/v1/models", "");
    assert_eq!(status, 200);
    assert!(body.contains("e2e-model"));
    let (status, body) = request(router_port, "HEAD", "/v1/models", "");
    assert_eq!(status, 200);
    assert!(body.is_empty());

    let (status, _) = request_without_length(router_port, "/v1/unknown");
    assert_eq!(status, 502);

    let (status, body) = request(
        router_port,
        "POST",
        "/v1/chat/completions",
        r#"{"model":"e2e-model","messages":[{"role":"user","content":"smoke"}]}"#,
    );
    assert_eq!(status, 200);
    assert!(body.contains("E2E_SENTINEL"));

    let lac = test_binary("lac");
    let (ok, output) = run_cli(
        &lac,
        &home,
        &["status", "--json"],
        &[("LAC_ROUTER_PORT", &router_port.to_string())],
    );
    assert!(ok, "lac status failed: {output}");
    assert!(output.contains("\"gateway\":true"));
    assert!(output.contains("\"active\":\"llama\""));

    let (ok, output) = run_cli(
        &lac,
        &home,
        &["chat", "smoke"],
        &[
            ("LAC_ROUTER_PORT", &router_port.to_string()),
            ("LAC_CHAT_MODEL", "e2e-model"),
        ],
    );
    assert!(ok, "lac chat failed: {output}");
    assert!(output.contains("E2E_SENTINEL"));

    let marker = home.join("injected");
    let (ok, output) = run_cli(
        &lac,
        &home,
        &["pull", "repo/'; touch injected; echo '"],
        &[("LAC_ROUTER_PORT", "0"), ("PATH", "")],
    );
    assert!(!ok, "missing downloader must fail: {output}");
    assert!(!marker.exists(), "model input must never become executable source");

    let fake_bin = home.join("fake-bin");
    std::fs::create_dir_all(&fake_bin).expect("fake downloader directory");
    let fake_hf = fake_bin.join("hf");
    std::fs::write(&fake_hf, "#!/bin/sh\nexit 0\n").expect("fake downloader");
    let mut permissions = std::fs::metadata(&fake_hf).expect("fake downloader metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fake_hf, permissions).expect("fake downloader permissions");
    let (ok, output) = run_cli(
        &lac,
        &home,
        &["pull", "org/model"],
        &[("LAC_ROUTER_PORT", "0"), ("PATH", fake_bin.to_str().expect("fake PATH"))],
    );
    assert!(ok, "Hugging Face CLI success path failed: {output}");
    assert!(output.contains("Successfully downloaded"));

    assert!(fixture.requests.load(Ordering::Acquire) >= 3);
    drop(router);

    let token_router = support::spawn_router(
        &router_bin,
        &[
            ("HOME", &home_env),
            ("LAC_BIND_ADDR", "127.0.0.1"),
            ("MLX_PORT", &dead_env),
            ("LAC_LLAMA_PORT", &llama_env),
            ("LAC_OLLAMA_PORT", &dead_env),
            ("LAC_BACKEND", "llama"),
            ("LAC_API_TOKEN", "e2e-secret"),
        ],
    );
    let token_port = token_router.port;
    wait_for_router(token_port, Some("e2e-secret"));
    wait_for_models(token_port, Some("e2e-secret"));
    let (ok, output) = run_cli(
        &lac,
        &home,
        &["status", "--json"],
        &[
            ("LAC_ROUTER_PORT", &token_port.to_string()),
            ("LAC_API_TOKEN", "e2e-secret"),
        ],
    );
    assert!(ok, "authenticated lac status failed: {output}");
    let (ok, output) = run_cli(
        &lac,
        &home,
        &["chat", "authenticated smoke"],
        &[
            ("LAC_ROUTER_PORT", &token_port.to_string()),
            ("LAC_API_TOKEN", "e2e-secret"),
            ("LAC_CHAT_MODEL", "e2e-model"),
        ],
    );
    assert!(ok, "authenticated lac chat failed: {output}");
    assert!(output.contains("E2E_SENTINEL"));
    assert!(
        !fixture.saw_authorization.load(Ordering::Acquire),
        "router authorization must not reach the backend"
    );
    drop(token_router);

    drop(fixture);
    let _ = std::fs::remove_dir_all(home);
}
