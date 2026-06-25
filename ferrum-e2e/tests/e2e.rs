/// End-to-end integration tests for Ferrum WAF.
///
/// Mirrors the third-party user flow:
///   1. A ferrum binary (built by `xferrum build`) is located via `FERRUM_BIN`.
///   2. A minimal upstream HTTP server is spawned.
///   3. The ferrum binary is spawned with a per-test config written to a temp file.
///   4. HTTP requests are made through ferrum; responses are checked.
use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Bind a free ephemeral port and return the listener.
/// Caller passes the listener to `spawn_upstream` or extracts the port for config.
fn bind_free() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").unwrap()
}

/// Minimal upstream: accepts connections, reads HTTP request headers, sends 200 OK.
fn spawn_upstream(listener: TcpListener) {
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if line == "\r\n" {
                    break;
                }
            }
            use std::io::Write;
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK");
        }
    });
}

/// Write a test ferrum.toml and spawn the ferrum process.
///
/// Returns `(Child, ferrum_port, tempdir)` — keep `tempdir` alive for the process lifetime.
fn spawn_ferrum(config: &str) -> (Child, u16, tempfile::TempDir) {
    let bin = std::env::var("FERRUM_BIN").unwrap_or_else(|_| "./target/debug/ferrum".into());
    assert!(
        Path::new(&bin).exists(),
        "ferrum binary not found at '{bin}'. Run `make build` first or set FERRUM_BIN."
    );

    let dir = tempfile::TempDir::new().unwrap();
    let cfg_path = dir.path().join("ferrum.toml");
    std::fs::write(&cfg_path, config).unwrap();

    let child = Command::new(&bin)
        .env("FERRUM_CONFIG", &cfg_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {bin}: {e}"));

    // Parse the listen port from the config
    let port: u16 = config
        .lines()
        .find(|l| l.trim_start().starts_with("listen"))
        .and_then(|l| l.split(':').last())
        .and_then(|p| p.trim().trim_matches('"').parse().ok())
        .expect("cannot find listen port in config");

    // Wait for ferrum to accept connections (up to 10 s)
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        if Instant::now() >= deadline {
            panic!("ferrum did not start on port {port} within 10 s");
        }
        thread::sleep(Duration::from_millis(100));
    }

    (child, port, dir)
}

struct Guard(Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn clean_request_passes_to_upstream() {
    let upstream = bind_free();
    let ferrum_listener = bind_free();
    let metrics_listener = bind_free();
    let upstream_port = upstream.local_addr().unwrap().port();
    let ferrum_port = ferrum_listener.local_addr().unwrap().port();
    let metrics_port = metrics_listener.local_addr().unwrap().port();
    drop(ferrum_listener);
    drop(metrics_listener);
    spawn_upstream(upstream);

    let config = format!(
        r#"
[server]
listen         = "127.0.0.1:{ferrum_port}"
upstream       = "127.0.0.1:{upstream_port}"
metrics_listen = "127.0.0.1:{metrics_port}"

[sensors.sqli]
plugin   = "sensor-regex"
patterns = ["(?i)union.*select"]
target   = "uri"

[terminators.block_403]
plugin = "block"
status = 403

[[rules]]
id        = "block-sqli"
input     = "sqli"
threshold = 100
if_action = "block_403"
"#
    );

    let (child, port, _dir) = spawn_ferrum(&config);
    let _guard = Guard(child);

    let resp = reqwest::blocking::get(format!("http://127.0.0.1:{port}/hello")).unwrap();
    assert_eq!(resp.status().as_u16(), 200);
}

#[test]
fn sqli_in_uri_returns_403() {
    let upstream = bind_free();
    let ferrum_listener = bind_free();
    let metrics_listener = bind_free();
    let upstream_port = upstream.local_addr().unwrap().port();
    let ferrum_port = ferrum_listener.local_addr().unwrap().port();
    let metrics_port = metrics_listener.local_addr().unwrap().port();
    drop(ferrum_listener);
    drop(metrics_listener);
    spawn_upstream(upstream);

    let config = format!(
        r#"
[server]
listen         = "127.0.0.1:{ferrum_port}"
upstream       = "127.0.0.1:{upstream_port}"
metrics_listen = "127.0.0.1:{metrics_port}"

[sensors.sqli]
plugin   = "sensor-regex"
patterns = ["(?i)union.*select"]
target   = "uri"

[terminators.block_403]
plugin = "block"
status = 403

[[rules]]
id        = "block-sqli"
input     = "sqli"
threshold = 100
if_action = "block_403"
"#
    );

    let (child, port, _dir) = spawn_ferrum(&config);
    let _guard = Guard(child);

    let resp =
        reqwest::blocking::get(format!("http://127.0.0.1:{port}/search?q=UNION+SELECT+1")).unwrap();
    assert_eq!(resp.status().as_u16(), 403);
}

#[test]
fn sqli_in_body_returns_403() {
    let upstream = bind_free();
    let ferrum_listener = bind_free();
    let metrics_listener = bind_free();
    let upstream_port = upstream.local_addr().unwrap().port();
    let ferrum_port = ferrum_listener.local_addr().unwrap().port();
    let metrics_port = metrics_listener.local_addr().unwrap().port();
    drop(ferrum_listener);
    drop(metrics_listener);
    spawn_upstream(upstream);

    let config = format!(
        r#"
[server]
listen         = "127.0.0.1:{ferrum_port}"
upstream       = "127.0.0.1:{upstream_port}"
metrics_listen = "127.0.0.1:{metrics_port}"

[sensors.sqli_body]
plugin   = "sensor-regex"
patterns = ["(?i)union.*select"]
target   = "body"

[terminators.block_403]
plugin = "block"
status = 403

[[rules]]
id          = "block-sqli"
input       = "sqli_body"
threshold   = 100
if_action   = "block_403"
buffer_body = true
"#
    );

    let (child, port, _dir) = spawn_ferrum(&config);
    let _guard = Guard(child);

    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/login"))
        .body("1 UNION SELECT * FROM users")
        .send()
        .unwrap();
    assert_eq!(resp.status().as_u16(), 403);
}

#[test]
fn or_transformer_fires_on_either_input() {
    let upstream = bind_free();
    let ferrum_listener = bind_free();
    let metrics_listener = bind_free();
    let upstream_port = upstream.local_addr().unwrap().port();
    let ferrum_port = ferrum_listener.local_addr().unwrap().port();
    let metrics_port = metrics_listener.local_addr().unwrap().port();
    drop(ferrum_listener);
    drop(metrics_listener);
    spawn_upstream(upstream);

    let config = format!(
        r#"
[server]
listen         = "127.0.0.1:{ferrum_port}"
upstream       = "127.0.0.1:{upstream_port}"
metrics_listen = "127.0.0.1:{metrics_port}"

[sensors.sqli_uri]
plugin   = "sensor-regex"
patterns = ["(?i)union.*select"]
target   = "uri"

[sensors.sqli_body]
plugin   = "sensor-regex"
patterns = ["(?i)union.*select"]
target   = "body"

[transformers.sqli_combined]
plugin = "or"
inputs = ["sqli_uri", "sqli_body"]

[terminators.block_403]
plugin = "block"
status = 403

[[rules]]
id          = "block-sqli"
input       = "sqli_combined"
threshold   = 100
if_action   = "block_403"
buffer_body = true
"#
    );

    let (child, port, _dir) = spawn_ferrum(&config);
    let _guard = Guard(child);

    let resp =
        reqwest::blocking::get(format!("http://127.0.0.1:{port}/?q=UNION+SELECT+1")).unwrap();
    assert_eq!(resp.status().as_u16(), 403, "URI SQLi should be blocked");

    let resp = reqwest::blocking::get(format!("http://127.0.0.1:{port}/hello")).unwrap();
    assert_eq!(resp.status().as_u16(), 200, "clean request should pass");
}

#[test]
fn redirect_terminator_returns_302() {
    let upstream = bind_free();
    let ferrum_listener = bind_free();
    let metrics_listener = bind_free();
    let upstream_port = upstream.local_addr().unwrap().port();
    let ferrum_port = ferrum_listener.local_addr().unwrap().port();
    let metrics_port = metrics_listener.local_addr().unwrap().port();
    drop(ferrum_listener);
    drop(metrics_listener);
    spawn_upstream(upstream);

    let config = format!(
        r#"
[server]
listen         = "127.0.0.1:{ferrum_port}"
upstream       = "127.0.0.1:{upstream_port}"
metrics_listen = "127.0.0.1:{metrics_port}"

[sensors.bot]
plugin   = "sensor-regex"
patterns = ["(?i)sqlmap"]
target   = "header:User-Agent"

[terminators.redirect_login]
plugin      = "redirect"
target_url  = "/login"
status_code = 302

[[rules]]
id        = "block-bot"
input     = "bot"
threshold = 100
if_action = "redirect_login"
"#
    );

    let (child, port, _dir) = spawn_ferrum(&config);
    let _guard = Guard(child);

    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let resp = client
        .get(format!("http://127.0.0.1:{port}/"))
        .header("User-Agent", "sqlmap/1.0")
        .send()
        .unwrap();
    assert_eq!(resp.status().as_u16(), 302);
    assert_eq!(resp.headers().get("location").unwrap(), "/login");
}
