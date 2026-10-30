//! End-to-end checks for the `vision` binary.

use std::error::Error;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn version_matches_the_workspace() -> Result<(), Box<dyn Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_vision"))
        .arg("--version")
        .output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("vision 0.0.1-alpha"));
    Ok(())
}

#[test]
fn shot_writes_a_png() -> Result<(), Box<dyn Error>> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let directory = std::env::temp_dir().join(format!("vision-cli-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    let input = directory.join("page.html");
    let output = directory.join("page.png");
    std::fs::write(
        &input,
        "<style>body{background:#f2f2f2}</style><h1>CLI</h1>",
    )?;

    let result = Command::new(env!("CARGO_BIN_EXE_vision"))
        .arg("shot")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .output()?;
    assert!(result.status.success());
    let bytes = std::fs::read(&output)?;
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

    let _ = std::fs::remove_dir_all(&directory);
    Ok(())
}
