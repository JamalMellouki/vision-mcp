//! Output formatting.

use std::error::Error;
use std::path::Path;

use vision::Capture;

/// Writes pretty JSON to stdout.
pub(crate) fn json(value: &serde_json::Value) -> Result<(), Box<dyn Error>> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Prints a one-line render summary.
pub(crate) fn summary(path: &Path, capture: &Capture) {
    println!(
        "wrote {} ({}x{}, {:.1} ms)",
        path.display(),
        capture.width,
        capture.height,
        capture.timings.total_ms
    );
}
