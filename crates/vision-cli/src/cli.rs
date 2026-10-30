//! Command-line argument definitions.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Vision command line.
#[derive(Debug, Parser)]
#[command(
    name = "vision",
    version,
    about = "Render HTML and CSS, inspect element geometry, and audit layouts."
)]
pub(crate) struct Cli {
    /// The command to run.
    #[command(subcommand)]
    pub(crate) command: Command,
}

/// Available commands.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Render a page to PNG.
    Shot(ShotArgs),
    /// Print element geometry as JSON.
    Inspect(InspectArgs),
    /// Run layout and accessibility checks.
    Audit(AuditArgs),
    /// Capture a URL through a local browser and render it.
    Capture(CaptureArgs),
    /// Run the Model Context Protocol server on stdin and stdout.
    Mcp,
}

/// Shared rendering arguments.
#[derive(Debug, Args)]
pub(crate) struct RenderArgs {
    /// HTML file to render.
    pub(crate) input: PathBuf,
    /// Viewport in CSS pixels, for example 1280x800.
    #[arg(long, default_value = "1280x800")]
    pub(crate) viewport: String,
    /// Device pixel ratio.
    #[arg(long, default_value_t = 1.0)]
    pub(crate) dpr: f32,
    /// Do not execute page JavaScript.
    #[arg(long)]
    pub(crate) no_javascript: bool,
}

/// Arguments for `vision shot`.
#[derive(Debug, Args)]
pub(crate) struct ShotArgs {
    #[command(flatten)]
    pub(crate) render: RenderArgs,
    /// Output PNG path. Defaults to the input path with a .png extension.
    #[arg(short, long)]
    pub(crate) output: Option<PathBuf>,
}

/// Arguments for `vision inspect`.
#[derive(Debug, Args)]
pub(crate) struct InspectArgs {
    #[command(flatten)]
    pub(crate) render: RenderArgs,
}

/// Arguments for `vision audit`.
#[derive(Debug, Args)]
pub(crate) struct AuditArgs {
    #[command(flatten)]
    pub(crate) render: RenderArgs,
}

/// Arguments for `vision capture`.
#[derive(Debug, Args)]
pub(crate) struct CaptureArgs {
    /// HTTP or HTTPS URL to capture.
    pub(crate) url: String,
    /// Viewport in CSS pixels, for example 1280x800.
    #[arg(long, default_value = "1280x800")]
    pub(crate) viewport: String,
    /// Device pixel ratio.
    #[arg(long, default_value_t = 1.0)]
    pub(crate) dpr: f32,
    /// Wait strategy: load, network-idle, fonts, or selector:<css selector>.
    #[arg(long, default_value = "network-idle")]
    pub(crate) wait: String,
    /// Quiet period for network-idle, in milliseconds.
    #[arg(long, default_value_t = 500)]
    pub(crate) quiet: u64,
    /// Overall timeout in seconds.
    #[arg(long, default_value_t = 30)]
    pub(crate) timeout: u64,
    /// Allow a non-localhost URL.
    #[arg(long)]
    pub(crate) allow_remote: bool,
    /// Pass --no-sandbox to Chromium. Use only in a disposable container.
    #[arg(long)]
    pub(crate) no_sandbox: bool,
    /// Explicit Chromium executable.
    #[arg(long)]
    pub(crate) browser: Option<PathBuf>,
    /// Optional user agent.
    #[arg(long)]
    pub(crate) user_agent: Option<String>,
    /// PNG output path.
    #[arg(short, long, default_value = "capture.png")]
    pub(crate) output: PathBuf,
    /// Optional snapshot JSON output path.
    #[arg(long)]
    pub(crate) snapshot: Option<PathBuf>,
    /// Run layout and accessibility checks.
    #[arg(long)]
    pub(crate) audit: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shot_accepts_no_javascript() {
        let cli = Cli::try_parse_from(["vision", "shot", "page.html", "--no-javascript"])
            .unwrap_or_else(|error| panic!("{error}"));
        let Command::Shot(args) = cli.command else {
            panic!("expected shot command");
        };
        assert!(args.render.no_javascript);
    }

    #[test]
    fn audit_accepts_no_javascript() {
        let cli = Cli::try_parse_from(["vision", "audit", "page.html", "--no-javascript"])
            .unwrap_or_else(|error| panic!("{error}"));
        let Command::Audit(args) = cli.command else {
            panic!("expected audit command");
        };
        assert!(args.render.no_javascript);
    }
}
