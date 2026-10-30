//! Command entry point for the Vision MCP server.

fn main() {
    let mut args = std::env::args().skip(1);
    if let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("vision-mcp {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--help" | "-h" => {
                println!("usage: vision-mcp\n\nRuns the Vision MCP server on stdin and stdout.");
                return;
            }
            _ => {
                eprintln!("vision-mcp: unknown argument: {arg}");
                std::process::exit(2);
            }
        }
    }
    if let Err(error) = vision_mcp::serve_stdio() {
        eprintln!("vision-mcp: {error}");
        std::process::exit(1);
    }
}
