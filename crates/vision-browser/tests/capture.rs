//! Browser capture integration checks.
//!
//! Set `VISION_BROWSER` to a Chromium executable to run the page test. The
//! validation checks run without a browser.

use std::error::Error;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;

use vision::Viewport;
use vision_browser::{CaptureOptions, WaitStrategy, capture};

#[test]
fn remote_urls_are_rejected_by_default() -> Result<(), Box<dyn Error>> {
    let mut options = CaptureOptions::new("https://example.com/");
    options.wait = WaitStrategy::Load;
    let error = capture(&options).err().ok_or("expected an error")?;
    assert!(error.to_string().contains("remote"));
    Ok(())
}

#[test]
fn captures_a_hydrated_page() -> Result<(), Box<dyn Error>> {
    let Ok(browser) = std::env::var("VISION_BROWSER") else {
        eprintln!("VISION_BROWSER is not set; skipping browser integration test");
        return Ok(());
    };
    let server = FixtureServer::start()?;
    let mut options = CaptureOptions::new(server.url());
    options.viewport = Viewport::new(320, 240);
    options.wait = WaitStrategy::NetworkIdle {
        quiet: Duration::from_millis(100),
    };
    options.timeout = Duration::from_secs(60);
    options.executable = Some(PathBuf::from(browser));
    options.no_sandbox = true;

    let snapshot = capture(&options)?;
    assert!(snapshot.html.contains("hydrated"));
    assert!(snapshot.html.contains("value=\"typed by script\""));
    assert!(snapshot.html.contains("style.css"));
    assert!(
        snapshot
            .stylesheets
            .iter()
            .any(|sheet| sheet.href == "/style.css")
    );
    assert!(
        snapshot
            .resources
            .iter()
            .any(|resource| resource.url == "/dot.svg")
    );
    assert_eq!(snapshot.viewport.width, 320);
    assert_eq!(snapshot.viewport.height, 240);
    server.stop();
    Ok(())
}

struct FixtureServer {
    port: u16,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl FixtureServer {
    fn start() -> Result<Self, Box<dyn Error>> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if flag.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                if let Ok(mut stream) = stream {
                    let _ = respond(&mut stream);
                }
            }
        });
        Ok(Self { port, stop })
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn respond(stream: &mut std::net::TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut buffer = [0_u8; 4096];
    let read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..read]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let (mime, body) = match path {
        "/" => (
            "text/html; charset=utf-8",
            r#"<!doctype html><html><head><link rel="stylesheet" href="/style.css"></head><body><div id="app">loading</div><input id="name"><img src="/dot.svg" width="16" height="16"><script>setTimeout(() => { document.getElementById('app').textContent = 'hydrated'; document.getElementById('name').value = 'typed by script'; }, 80);</script></body></html>"#,
        ),
        "/style.css" => (
            "text/css; charset=utf-8",
            "#app { font-weight: bold; } body { color: #123456; }",
        ),
        "/dot.svg" => (
            "image/svg+xml",
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#00ff00"/></svg>"##,
        ),
        _ => ("text/plain", "not found"),
    };
    let status = if path == "/" || path == "/style.css" || path == "/dot.svg" {
        "200 OK"
    } else {
        "404 Not Found"
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body.as_bytes())?;
    Ok(())
}
