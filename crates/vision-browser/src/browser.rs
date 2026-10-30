//! Browser process discovery and launch.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};

const BROWSER_NAMES: [&str; 6] = [
    "chromium",
    "chromium-browser",
    "google-chrome",
    "google-chrome-stable",
    "chrome",
    "chrome.exe",
];

pub(crate) struct BrowserProcess {
    child: Child,
    profile: PathBuf,
}

impl BrowserProcess {
    pub(crate) fn launch(executable: &Path, no_sandbox: bool) -> Result<Self> {
        let profile = temporary_profile();
        std::fs::create_dir_all(&profile)?;

        let mut command = Command::new(executable);
        command
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--disable-dev-shm-usage")
            .arg("--disable-background-networking")
            .arg("--disable-extensions")
            .arg("--disable-sync")
            .arg("--hide-scrollbars")
            .arg("--mute-audio")
            .arg("--no-default-browser-check")
            .arg("--no-first-run")
            .arg("--metrics-recording-only")
            .arg("--remote-debugging-port=0")
            .arg("--remote-debugging-address=127.0.0.1")
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--window-size=1280,800");
        if no_sandbox {
            command.arg("--no-sandbox").arg("--disable-setuid-sandbox");
        }
        command
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = command.spawn()?;
        Ok(Self { child, profile })
    }

    pub(crate) fn websocket_url(&mut self, timeout: Duration) -> Result<String> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.child.try_wait()?.is_some() {
                return Err(Error::BrowserExited);
            }
            if let Some(url) = devtools_url(&self.profile) {
                return Ok(url);
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout(
                    "waiting for the DevTools endpoint".to_owned(),
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for BrowserProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

/// Finds a browser without invoking a shell.
pub(crate) fn executable(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        if path.is_file() {
            return Ok(path.to_owned());
        }
        return Err(Error::BrowserNotFound);
    }
    if let Some(path) = std::env::var_os("VISION_BROWSER") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
    }
    if let Some(path) = std::env::var_os("VISION_CHROMIUM") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
    }

    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            for name in BROWSER_NAMES {
                let candidate = directory.join(name);
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
    }

    if let Some(path) = playwright_browser() {
        return Ok(path);
    }
    Err(Error::BrowserNotFound)
}

fn playwright_browser() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let root = PathBuf::from(home).join(".cache").join("ms-playwright");
    let entries = std::fs::read_dir(root).ok()?;
    let mut candidates = entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("chromium-"))
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.reverse();
    for directory in candidates {
        for entry in std::fs::read_dir(directory).ok()? {
            let path = entry.ok()?.path();
            if !path.is_dir() {
                continue;
            }
            for executable in ["chrome", "chrome.exe"] {
                let candidate = path.join(executable);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn temporary_profile() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("vision-browser-{}-{stamp}", std::process::id()))
}

fn devtools_url(profile: &Path) -> Option<String> {
    let content = std::fs::read_to_string(profile.join("DevToolsActivePort")).ok()?;
    let mut lines = content.lines();
    let port = lines.next()?.trim().parse::<u16>().ok()?;
    let path = lines.next()?.trim();
    if port == 0 || !path.starts_with('/') {
        return None;
    }
    Some(format!("ws://127.0.0.1:{port}{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_devtools_active_port_without_reserving_a_port() {
        let directory = temporary_profile();
        std::fs::create_dir_all(&directory).unwrap_or_else(|error| panic!("{error}"));
        std::fs::write(
            directory.join("DevToolsActivePort"),
            "41234\n/devtools/browser/session-id\n",
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            devtools_url(&directory).as_deref(),
            Some("ws://127.0.0.1:41234/devtools/browser/session-id")
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn rejects_incomplete_devtools_port_files() {
        let directory = temporary_profile();
        std::fs::create_dir_all(&directory).unwrap_or_else(|error| panic!("{error}"));
        std::fs::write(directory.join("DevToolsActivePort"), "41234\n")
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(devtools_url(&directory).is_none());
        let _ = std::fs::remove_dir_all(directory);
    }
}
