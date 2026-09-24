use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

pub struct PortLock {
    file: File,
}

impl PortLock {
    pub fn acquire() -> Self {
        let path = std::env::temp_dir().join("lac-router-test-port.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .expect("open router test port lock");
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if try_lock(&file) {
                return Self { file };
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("timed out waiting for router test port lock");
    }
}

impl Drop for PortLock {
    fn drop(&mut self) {
        unlock(&self.file);
    }
}

pub struct RouterProcess {
    pub child: Child,
    pub port: u16,
}

impl Drop for RouterProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn spawn_router(binary: &str, envs: &[(&str, &str)]) -> RouterProcess {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    command.env("LAC_ROUTER_PORT", "0");
    let mut child = command.spawn().expect("router binary runs");
    let stderr = child.stderr.take().expect("router stderr pipe");
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if !line.contains("Listening on http://") {
                continue;
            }
            if let Some((_, port)) = line.rsplit_once(':') {
                if let Ok(port) = port.trim().parse::<u16>() {
                    let _ = sender.send(port);
                }
            }
        }
    });
    let port = receiver
        .recv_timeout(Duration::from_secs(15))
        .unwrap_or_else(|_| {
            let _ = child.kill();
            let _ = child.wait();
            panic!("router did not report its bound port");
        });
    RouterProcess { child, port }
}

#[cfg(unix)]
fn try_lock(file: &File) -> bool {
    const LOCK_EX: i32 = 2;
    const LOCK_NB: i32 = 4;
    unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) == 0 }
}

#[cfg(unix)]
fn unlock(file: &File) {
    const LOCK_UN: i32 = 8;
    unsafe {
        let _ = flock(file.as_raw_fd(), LOCK_UN);
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}

#[cfg(not(unix))]
fn try_lock(_file: &File) -> bool {
    true
}

#[cfg(not(unix))]
fn unlock(_file: &File) {}
