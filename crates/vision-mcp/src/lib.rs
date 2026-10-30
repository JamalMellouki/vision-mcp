//! Model Context Protocol server for Vision.
//!
//! The server speaks JSON-RPC over standard input and output. Diagnostics go to
//! standard error so they cannot corrupt the protocol stream.

use std::collections::HashMap;
use std::error::Error;
use std::io::{BufRead, Write};

use base64::Engine as _;
use serde_json::{Value, json};
use vision::{Capture, Config, Engine, Session, Snapshot, Target, Viewport};

static BASE64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;
const MAX_HTML_BYTES: usize = 8 * 1024 * 1024;
const MAX_SESSIONS: usize = 16;
const MAX_TEXT_BYTES: usize = 1024 * 1024;

/// Runs the MCP server on standard input and output until the stream closes.
pub fn serve_stdio() -> Result<(), Box<dyn Error>> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    let mut server = Server::default();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = server.handle_line(&line) {
            serde_json::to_writer(&mut output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

#[derive(Default)]
struct Server {
    next_session: u64,
    sessions: HashMap<String, Session>,
}

impl Server {
    fn handle_line(&mut self, line: &str) -> Option<Value> {
        let request: Value = match serde_json::from_str(line) {
            Ok(request) => request,
            Err(error) => {
                return Some(error_response(
                    Value::Null,
                    -32700,
                    &format!("parse error: {error}"),
                ));
            }
        };
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            return Some(error_response(
                request.get("id").cloned().unwrap_or(Value::Null),
                -32600,
                "invalid request",
            ));
        };
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let id = request.get("id").cloned();

        let result = self.dispatch(method, &params);
        let id = id?;
        Some(match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => error_response(id, code, &message),
        })
    }

    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                let protocol = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .filter(|version| !version.is_empty())
                    .unwrap_or("2024-11-05");
                Ok(json!({
                    "protocolVersion": protocol,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "vision", "version": vision::VERSION},
                    "instructions": "Render local HTML, inspect geometry, audit layouts, and drive an interactive session."
                }))
            }
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or((-32602, "missing tool name".to_owned()))?;
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                self.call_tool(name, &arguments)
                    .map_err(|message| (-32602, message))
            }
            "ping" => Ok(json!({})),
            "shutdown" => Ok(Value::Null),
            "notifications/initialized" | "notifications/cancelled" => Ok(Value::Null),
            _ => Err((-32601, format!("unknown method: {method}"))),
        }
    }

    fn call_tool(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        let result = match name {
            "vision_shot" => self.shot(args),
            "vision_inspect" => self.inspect(args),
            "vision_audit" => self.audit(args),
            "vision_open" => self.open(args),
            "vision_click" => self.click(args),
            "vision_focus" => self.focus(args),
            "vision_type" => self.type_text(args),
            "vision_scroll" => self.scroll(args),
            "vision_capture" => self.capture(args),
            "vision_close" => self.close(args),
            _ => return Err(format!("unknown tool: {name}")),
        };
        Ok(result.unwrap_or_else(tool_error))
    }

    fn shot(&self, args: &Value) -> Result<Value, String> {
        let snapshot = snapshot_from_args(args)?;
        let engine =
            Engine::native(engine_config(args, false)).map_err(|error| error.to_string())?;
        let capture = engine
            .render(&snapshot)
            .map_err(|error| error.to_string())?;
        let output = args.get("output").and_then(Value::as_str);
        if let Some(path) = output {
            std::fs::write(path, &capture.png).map_err(|error| error.to_string())?;
            let metadata = metadata(&capture, json!({"output": path}));
            return Ok(json!({
                "content": [{"type": "text", "text": metadata.to_string()}],
                "isError": false
            }));
        }
        Ok(image_result(&capture, metadata(&capture, json!({}))))
    }

    fn inspect(&self, args: &Value) -> Result<Value, String> {
        let snapshot = snapshot_from_args(args)?;
        let engine =
            Engine::native(engine_config(args, false)).map_err(|error| error.to_string())?;
        let capture = engine
            .render(&snapshot)
            .map_err(|error| error.to_string())?;
        let metadata = metadata(&capture, json!({}));
        Ok(json!({
            "content": [{"type": "text", "text": metadata.to_string()}],
            "isError": false
        }))
    }

    fn audit(&self, args: &Value) -> Result<Value, String> {
        let snapshot = snapshot_from_args(args)?;
        let engine =
            Engine::native(engine_config(args, true)).map_err(|error| error.to_string())?;
        let capture = engine
            .render(&snapshot)
            .map_err(|error| error.to_string())?;
        Ok(image_result(&capture, metadata(&capture, json!({}))))
    }

    fn open(&mut self, args: &Value) -> Result<Value, String> {
        if self.sessions.len() >= MAX_SESSIONS {
            return Err(format!(
                "session limit reached ({MAX_SESSIONS}); close a session first"
            ));
        }
        let snapshot = snapshot_from_args(args)?;
        let engine =
            Engine::native(engine_config(args, false)).map_err(|error| error.to_string())?;
        let mut session = engine.open(snapshot).map_err(|error| error.to_string())?;
        let capture = session.capture().map_err(|error| error.to_string())?;
        let id = format!("s{}", self.next_session);
        self.next_session += 1;
        self.sessions.insert(id.clone(), session);
        Ok(image_result(
            &capture,
            metadata(&capture, json!({"session": id})),
        ))
    }

    fn click(&mut self, args: &Value) -> Result<Value, String> {
        let id = session_id(args)?;
        let target = target_from_args(args)?;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or_else(|| format!("unknown session: {id}"))?;
        let action = session.click(&target).map_err(|error| error.to_string())?;
        let capture = session.capture().map_err(|error| error.to_string())?;
        Ok(image_result(
            &capture,
            metadata(&capture, json!({"session": id, "action": action})),
        ))
    }

    fn focus(&mut self, args: &Value) -> Result<Value, String> {
        let id = session_id(args)?;
        let target = target_from_args(args)?;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or_else(|| format!("unknown session: {id}"))?;
        let action = session.focus(&target).map_err(|error| error.to_string())?;
        let capture = session.capture().map_err(|error| error.to_string())?;
        Ok(image_result(
            &capture,
            metadata(&capture, json!({"session": id, "action": action})),
        ))
    }

    fn type_text(&mut self, args: &Value) -> Result<Value, String> {
        let id = session_id(args)?;
        let target = target_from_args(args)?;
        let text = args
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing text".to_owned())?;
        if text.len() > MAX_TEXT_BYTES {
            return Err(format!("text exceeds {MAX_TEXT_BYTES} bytes"));
        }
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or_else(|| format!("unknown session: {id}"))?;
        let action = session
            .type_text(&target, text)
            .map_err(|error| error.to_string())?;
        let capture = session.capture().map_err(|error| error.to_string())?;
        Ok(image_result(
            &capture,
            metadata(&capture, json!({"session": id, "action": action})),
        ))
    }

    fn scroll(&mut self, args: &Value) -> Result<Value, String> {
        let id = session_id(args)?;
        let dx = number(args, "dx").unwrap_or(0.0) as f32;
        let dy = number(args, "dy").unwrap_or(0.0) as f32;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or_else(|| format!("unknown session: {id}"))?;
        let action = session.scroll(dx, dy);
        let capture = session.capture().map_err(|error| error.to_string())?;
        Ok(image_result(
            &capture,
            metadata(&capture, json!({"session": id, "action": action})),
        ))
    }

    fn capture(&mut self, args: &Value) -> Result<Value, String> {
        let id = session_id(args)?;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or_else(|| format!("unknown session: {id}"))?;
        let capture = session.capture().map_err(|error| error.to_string())?;
        Ok(image_result(
            &capture,
            metadata(&capture, json!({"session": id})),
        ))
    }

    fn close(&mut self, args: &Value) -> Result<Value, String> {
        let id = session_id(args)?;
        self.sessions
            .remove(&id)
            .ok_or_else(|| format!("unknown session: {id}"))?;
        Ok(json!({
            "content": [{"type": "text", "text": format!("closed {id}")}],
            "isError": false
        }))
    }
}

fn engine_config(args: &Value, audit: bool) -> Config {
    Config {
        audit,
        elements: true,
        javascript: args
            .get("javascript")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        ..Config::default()
    }
}

fn snapshot_from_args(args: &Value) -> Result<Snapshot, String> {
    let snapshot = if let Some(path) = args.get("input").and_then(Value::as_str) {
        Snapshot::from_file(path).map_err(|error| error.to_string())?
    } else if let Some(html) = args.get("html").and_then(Value::as_str) {
        if html.len() > MAX_HTML_BYTES {
            return Err(format!("html exceeds {MAX_HTML_BYTES} bytes"));
        }
        Snapshot::from_html(html)
    } else {
        return Err("provide input or html".to_owned());
    };
    let raw = args
        .get("viewport")
        .and_then(Value::as_str)
        .unwrap_or("1280x800");
    let dpr = number(args, "dpr").unwrap_or(1.0) as f32;
    let viewport = parse_viewport(raw, dpr)?;
    Ok(snapshot.with_viewport(viewport))
}

fn parse_viewport(raw: &str, dpr: f32) -> Result<Viewport, String> {
    let split = raw
        .find(['x', 'X'])
        .ok_or_else(|| format!("invalid viewport '{raw}'; expected WIDTHxHEIGHT"))?;
    let width = raw[..split]
        .parse::<u32>()
        .map_err(|error| format!("invalid viewport width: {error}"))?;
    let height = raw[split + 1..]
        .parse::<u32>()
        .map_err(|error| format!("invalid viewport height: {error}"))?;
    if width == 0 || height == 0 {
        return Err("viewport dimensions must be greater than zero".to_owned());
    }
    if width > 8192 || height > 8192 {
        return Err("viewport dimensions must not exceed 8192".to_owned());
    }
    let device_pixels = f64::from(width) * f64::from(height) * f64::from(dpr) * f64::from(dpr);
    if device_pixels > 64_000_000.0 {
        return Err("viewport exceeds 64 million device pixels".to_owned());
    }
    if !(0.5..=4.0).contains(&dpr) {
        return Err("device pixel ratio must be between 0.5 and 4.0".to_owned());
    }
    Ok(Viewport::new(width, height).with_dpr(dpr))
}

fn session_id(args: &Value) -> Result<String, String> {
    args.get("session")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "missing session".to_owned())
}

fn target_from_args(args: &Value) -> Result<Target, String> {
    if let Some(id) = args.get("id").and_then(Value::as_str) {
        return Ok(Target::Id(id.to_owned()));
    }
    if let Some(selector) = args.get("selector").and_then(Value::as_str) {
        return Ok(Target::Selector(selector.to_owned()));
    }
    if let Some(text) = args.get("text_target").and_then(Value::as_str) {
        return Ok(Target::Text(text.to_owned()));
    }
    if let (Some(x), Some(y)) = (number(args, "x"), number(args, "y")) {
        return Ok(Target::Point(vision::Point {
            x: x as f32,
            y: y as f32,
        }));
    }
    Err("target needs id, selector, text_target, or x and y".to_owned())
}

fn number(args: &Value, name: &str) -> Option<f64> {
    args.get(name).and_then(Value::as_f64)
}

fn metadata(capture: &Capture, extra: Value) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("width".to_owned(), json!(capture.width));
    map.insert("height".to_owned(), json!(capture.height));
    map.insert("document_height".to_owned(), json!(capture.document_height));
    map.insert("elements".to_owned(), json!(capture.elements));
    map.insert("audit".to_owned(), json!(capture.audit));
    map.insert("compat".to_owned(), json!(capture.compat));
    map.insert("timings".to_owned(), json!(capture.timings));
    if let Value::Object(extra) = extra {
        map.extend(extra);
    }
    Value::Object(map)
}

fn image_result(capture: &Capture, metadata: Value) -> Value {
    let data = BASE64.encode(&capture.png);
    json!({
        "content": [
            {"type": "image", "mimeType": "image/png", "data": data},
            {"type": "text", "text": metadata.to_string()}
        ],
        "isError": false
    })
}

fn tool_error(message: String) -> Value {
    json!({
        "content": [{"type": "text", "text": message}],
        "isError": true
    })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message}
    })
}

fn tools() -> Value {
    json!([
        {
            "name": "vision_shot",
            "description": "Render a local HTML file or HTML string to PNG. Returns an image, or writes it to output.",
            "inputSchema": page_schema(true)
        },
        {
            "name": "vision_inspect",
            "description": "Render a page and return element rectangles, compatibility notes, and timings as JSON.",
            "inputSchema": page_schema(false)
        },
        {
            "name": "vision_audit",
            "description": "Render a page and return layout and accessibility findings with a screenshot.",
            "inputSchema": page_schema(false)
        },
        {
            "name": "vision_open",
            "description": "Open an interactive session and return its identifier and first screenshot.",
            "inputSchema": page_schema(false)
        },
        {
            "name": "vision_click",
            "description": "Click an element in a session and return the resulting screenshot and action.",
            "inputSchema": {
                "type": "object",
                "properties": merged_target_properties(),
                "required": ["session"]
            }
        },
        {
            "name": "vision_focus",
            "description": "Focus an element in a session and return the resulting screenshot.",
            "inputSchema": {
                "type": "object",
                "properties": merged_target_properties(),
                "required": ["session"]
            }
        },
        {
            "name": "vision_type",
            "description": "Type text into an input or textarea in a session.",
            "inputSchema": {
                "type": "object",
                "properties": merged_typed_properties(),
                "required": ["session", "text"]
            }
        },
        {
            "name": "vision_scroll",
            "description": "Scroll a session by a delta and return the resulting screenshot.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session": {"type": "string"},
                    "dx": {"type": "number", "default": 0},
                    "dy": {"type": "number", "default": 0}
                },
                "required": ["session"]
            }
        },
        {
            "name": "vision_capture",
            "description": "Return the current screenshot of a session.",
            "inputSchema": {
                "type": "object",
                "properties": {"session": {"type": "string"}},
                "required": ["session"]
            }
        },
        {
            "name": "vision_close",
            "description": "Close a session and release its layout state.",
            "inputSchema": {
                "type": "object",
                "properties": {"session": {"type": "string"}},
                "required": ["session"]
            }
        }
    ])
}

fn page_schema(with_output: bool) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "input".to_owned(),
        json!({"type": "string", "description": "Path to a local HTML file."}),
    );
    properties.insert(
        "html".to_owned(),
        json!({"type": "string", "description": "HTML source."}),
    );
    properties.insert(
        "viewport".to_owned(),
        json!({"type": "string", "default": "1280x800", "description": "WIDTHxHEIGHT in CSS pixels."}),
    );
    properties.insert(
        "dpr".to_owned(),
        json!({"type": "number", "default": 1.0, "description": "Device pixel ratio."}),
    );
    properties.insert(
        "javascript".to_owned(),
        json!({"type": "boolean", "default": true, "description": "Execute page scripts. Disable for static-only rendering."}),
    );
    if with_output {
        properties.insert(
            "output".to_owned(),
            json!({"type": "string", "description": "Optional PNG path."}),
        );
    }
    json!({
        "type": "object",
        "properties": properties,
        "anyOf": [{"required": ["input"]}, {"required": ["html"]}]
    })
}

fn merged_target_properties() -> Value {
    json!({
        "session": {"type": "string"},
        "id": {"type": "string"},
        "selector": {"type": "string"},
        "text_target": {"type": "string"},
        "x": {"type": "number"},
        "y": {"type": "number"}
    })
}

fn merged_typed_properties() -> Value {
    let mut properties = merged_target_properties()
        .as_object()
        .cloned()
        .unwrap_or_default();
    properties.insert(
        "text".to_owned(),
        json!({"type": "string", "description": "Text to write."}),
    );
    Value::Object(properties)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_tools_contains_session_tools() {
        let tools = tools();
        let names = tools
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| tool.get("name").and_then(Value::as_str))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        assert!(names.contains(&"vision_shot"));
        assert!(names.contains(&"vision_open"));
        assert!(names.contains(&"vision_click"));
        assert!(names.contains(&"vision_focus"));
        assert!(names.contains(&"vision_type"));
    }

    #[test]
    fn page_tools_expose_the_javascript_switch() {
        let tools = tools();
        let shot = tools
            .as_array()
            .and_then(|tools| {
                tools
                    .iter()
                    .find(|tool| tool.get("name").and_then(Value::as_str) == Some("vision_shot"))
            })
            .ok_or("vision_shot missing")
            .unwrap_or_else(|error| panic!("{error}"));
        let javascript = shot
            .get("inputSchema")
            .and_then(|schema| schema.get("properties"))
            .and_then(|properties| properties.get("javascript"))
            .ok_or("javascript property missing")
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            javascript.get("default").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn initialize_returns_tools_capability() {
        let mut server = Server::default();
        let response = server
            .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
            .unwrap_or_else(|| panic!("missing response"));
        assert!(
            response
                .get("result")
                .and_then(|result| result.get("capabilities"))
                .and_then(|capabilities| capabilities.get("tools"))
                .is_some()
        );
    }

    #[test]
    fn session_tools_round_trip() {
        let mut server = Server::default();
        let open = server
            .handle_line(
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"vision_open","arguments":{"html":"<input id=name>","viewport":"320x240"}}}"#,
            )
            .unwrap_or_else(|| panic!("missing open response"));
        let text = open
            .get("result")
            .and_then(|result| result.get("content"))
            .and_then(Value::as_array)
            .and_then(|content| {
                content
                    .iter()
                    .find(|item| item.get("type").and_then(Value::as_str) == Some("text"))
            })
            .and_then(|item| item.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("missing open metadata"));
        let metadata: Value = serde_json::from_str(text).unwrap_or_else(|_| panic!("bad metadata"));
        let session = metadata
            .get("session")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("missing session id"));

        for (id, name, arguments) in [
            (2, "vision_click", json!({"session": session, "id": "name"})),
            (
                3,
                "vision_type",
                json!({"session": session, "id": "name", "text": "hello"}),
            ),
            (4, "vision_scroll", json!({"session": session, "dy": 20})),
            (5, "vision_close", json!({"session": session})),
        ] {
            let request = json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments}
            });
            let response = server
                .handle_line(&request.to_string())
                .unwrap_or_else(|| panic!("missing response for {name}"));
            assert_eq!(
                response
                    .get("result")
                    .and_then(|result| result.get("isError")),
                Some(&Value::Bool(false))
            );
        }
    }

    #[test]
    fn shot_tool_returns_png() {
        let mut server = Server::default();
        let response = server
            .handle_line(
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"vision_shot","arguments":{"html":"<h1>Hello</h1>","viewport":"200x100"}}}"#,
            )
            .unwrap_or_else(|| panic!("missing response"));
        let content = response
            .get("result")
            .and_then(|result| result.get("content"))
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("missing content"));
        assert!(
            content
                .iter()
                .any(|item| item.get("type").and_then(Value::as_str) == Some("image"))
        );
    }
}
