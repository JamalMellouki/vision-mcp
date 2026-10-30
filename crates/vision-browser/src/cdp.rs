//! Minimal Chrome DevTools Protocol transport.

use std::collections::VecDeque;
use std::io::ErrorKind;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket, connect};

use crate::error::{Error, Result};

pub(crate) struct Cdp {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
    events: VecDeque<Value>,
}

impl Cdp {
    pub(crate) fn connect(url: &str) -> Result<Self> {
        let (mut socket, _) = connect(url)?;
        if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream.set_nodelay(true)?;
            stream.set_read_timeout(Some(Duration::from_millis(50)))?;
        }
        Ok(Self {
            socket,
            next_id: 1,
            events: VecDeque::new(),
        })
    }

    pub(crate) fn command(
        &mut self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let mut request = json!({
            "id": id,
            "method": method,
            "params": params
        });
        if let Some(session) = session {
            request["sessionId"] = Value::String(session.to_owned());
        }
        self.socket.send(Message::text(request.to_string()))?;

        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if Instant::now() >= deadline {
                return Err(Error::Timeout(format!("waiting for {method}")));
            }
            let Some(event) = self.read_event()? else {
                continue;
            };
            if event.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = event.get("error") {
                    return Err(Error::Protocol(
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown error")
                            .to_owned(),
                    ));
                }
                return Ok(event.get("result").cloned().unwrap_or(Value::Null));
            }
            self.events.push_back(event);
        }
    }

    pub(crate) fn drain_events(&mut self) -> Vec<Value> {
        self.events.drain(..).collect()
    }

    pub(crate) fn next_event(&mut self, wait: Duration) -> Result<Option<Value>> {
        if let Some(event) = self.events.pop_front() {
            return Ok(Some(event));
        }
        let deadline = Instant::now() + wait;
        loop {
            if let Some(event) = self.read_event()? {
                return Ok(Some(event));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
        }
    }

    fn read_event(&mut self) -> Result<Option<Value>> {
        loop {
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    return Ok(Some(serde_json::from_str(text.as_ref())?));
                }
                Ok(Message::Binary(bytes)) => {
                    if bytes.is_empty() {
                        return Ok(None);
                    }
                    return Err(Error::Data("unexpected binary CDP message".to_owned()));
                }
                Ok(Message::Ping(payload)) => {
                    self.socket.send(Message::Pong(payload))?;
                }
                Ok(Message::Pong(_) | Message::Frame(_)) => {}
                Ok(Message::Close(frame)) => {
                    return Err(Error::Protocol(match frame {
                        Some(frame) => format!("CDP closed the connection: {}", frame.reason),
                        None => "CDP closed the connection".to_owned(),
                    }));
                }
                Err(tungstenite::Error::Io(error))
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(Error::WebSocket(error)),
            }
        }
    }
}
