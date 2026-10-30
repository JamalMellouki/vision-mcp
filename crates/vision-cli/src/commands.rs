//! Command implementations.

use std::error::Error;

use serde_json::json;
use vision::{Config, Engine, Snapshot, Viewport};

use crate::cli::{AuditArgs, CaptureArgs, Command, InspectArgs, RenderArgs, ShotArgs};
use crate::output;

/// Runs a command.
pub(crate) fn run(command: Command) -> Result<(), Box<dyn Error>> {
    match command {
        Command::Shot(args) => shot(args),
        Command::Inspect(args) => inspect(args),
        Command::Audit(args) => audit(args),
        Command::Capture(args) => capture(args),
        Command::Mcp => {
            vision_mcp::serve_stdio()?;
            Ok(())
        }
    }
}

fn render_config(args: &RenderArgs) -> Config {
    Config {
        javascript: !args.no_javascript,
        ..Config::default()
    }
}

fn shot(args: ShotArgs) -> Result<(), Box<dyn Error>> {
    let viewport = parse_viewport(&args.render.viewport, args.render.dpr)?;
    let engine = Engine::native(render_config(&args.render))?;
    let snapshot = Snapshot::from_file(&args.render.input)?.with_viewport(viewport);
    let capture = engine.render(&snapshot)?;
    let output = args
        .output
        .unwrap_or_else(|| args.render.input.with_extension("png"));
    std::fs::write(&output, &capture.png)?;
    output::summary(&output, &capture);
    Ok(())
}

fn inspect(args: InspectArgs) -> Result<(), Box<dyn Error>> {
    let viewport = parse_viewport(&args.render.viewport, args.render.dpr)?;
    let engine = Engine::native(render_config(&args.render))?;
    let snapshot = Snapshot::from_file(&args.render.input)?.with_viewport(viewport);
    let capture = engine.render(&snapshot)?;
    output::json(&json!({
        "width": capture.width,
        "height": capture.height,
        "document_height": capture.document_height,
        "elements": capture.elements,
        "compat": capture.compat,
        "timings": capture.timings,
    }))?;
    Ok(())
}

fn audit(args: AuditArgs) -> Result<(), Box<dyn Error>> {
    let viewport = parse_viewport(&args.render.viewport, args.render.dpr)?;
    let engine = Engine::native(Config {
        audit: true,
        javascript: !args.render.no_javascript,
        ..Config::default()
    })?;
    let snapshot = Snapshot::from_file(&args.render.input)?.with_viewport(viewport);
    let capture = engine.render(&snapshot)?;
    output::json(&json!({
        "width": capture.width,
        "height": capture.height,
        "document_height": capture.document_height,
        "audit": capture.audit,
        "compat": capture.compat,
        "timings": capture.timings,
    }))?;
    Ok(())
}

fn capture(args: CaptureArgs) -> Result<(), Box<dyn Error>> {
    let viewport = parse_viewport(&args.viewport, args.dpr)?;
    let mut options = vision_browser::CaptureOptions::new(&args.url);
    options.viewport = viewport;
    options.wait = parse_wait(&args.wait, args.quiet)?;
    options.timeout = std::time::Duration::from_secs(args.timeout.max(1));
    options.allow_remote = args.allow_remote;
    options.no_sandbox = args.no_sandbox;
    options.executable = args.browser;
    options.user_agent = args.user_agent;

    let snapshot = vision_browser::capture(&options)?;
    if let Some(path) = &args.snapshot {
        let bytes = serde_json::to_vec_pretty(&snapshot)?;
        std::fs::write(path, bytes)?;
        eprintln!("wrote {}", path.display());
    }

    let engine = Engine::native(Config {
        audit: args.audit,
        ..Config::default()
    })?;
    let capture = engine.render(&snapshot)?;
    std::fs::write(&args.output, &capture.png)?;
    output::summary(&args.output, &capture);
    Ok(())
}

fn parse_wait(raw: &str, quiet: u64) -> Result<vision_browser::WaitStrategy, Box<dyn Error>> {
    match raw {
        "load" => Ok(vision_browser::WaitStrategy::Load),
        "network-idle" => Ok(vision_browser::WaitStrategy::NetworkIdle {
            quiet: std::time::Duration::from_millis(quiet),
        }),
        "fonts" => Ok(vision_browser::WaitStrategy::FontsReady),
        value if value.starts_with("selector:") => {
            let selector = &value["selector:".len()..];
            if selector.trim().is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "selector wait requires a CSS selector",
                )
                .into());
            }
            Ok(vision_browser::WaitStrategy::Selector(selector.to_owned()))
        }
        value => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "invalid wait strategy '{value}'; expected load, network-idle, fonts, or selector:<css>"
            ),
        )
        .into()),
    }
}

fn parse_viewport(raw: &str, dpr: f32) -> Result<Viewport, Box<dyn Error>> {
    let split = raw.find(['x', 'X']).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid viewport '{raw}'; expected WIDTHxHEIGHT"),
        )
    })?;
    let (width, height) = (&raw[..split], &raw[split + 1..]);
    let width: u32 = width.parse()?;
    let height: u32 = height.parse()?;
    if width == 0 || height == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "viewport dimensions must be greater than zero",
        )
        .into());
    }
    if !(0.5..=4.0).contains(&dpr) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "device pixel ratio must be between 0.5 and 4.0",
        )
        .into());
    }
    Ok(Viewport::new(width, height).with_dpr(dpr))
}
