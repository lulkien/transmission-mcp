//! A scripted HTTP endpoint that stands in for Transmission's RPC server.
//!
//! Tests point the server at this instead of a real daemon so every request
//! it makes can be asserted on, and every response (including the `409`
//! CSRF handshake) can be scripted.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// One scripted HTTP response.
#[derive(Debug, Clone)]
pub struct Reply {
    /// HTTP status code to answer with.
    pub status: u16,
    /// Extra response headers.
    pub headers: Vec<(String, String)>,
    /// Response body.
    pub body: String,
}

impl Reply {
    /// A `200 OK` carrying a JSON body.
    #[must_use]
    pub fn json(body: &serde_json::Value) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body: body.to_string(),
        }
    }

    /// A successful RPC envelope with the given `arguments`.
    #[must_use]
    pub fn success(arguments: &serde_json::Value) -> Self {
        Self::json(&serde_json::json!({
            "result": "success",
            "arguments": arguments,
        }))
    }

    /// A `409 Conflict` carrying a CSRF session id.
    #[must_use]
    pub fn conflict(session_id: &str) -> Self {
        Self {
            status: 409,
            headers: vec![(
                "X-Transmission-Session-Id".to_owned(),
                session_id.to_owned(),
            )],
            body: "409: Conflict".to_owned(),
        }
    }

    /// A bare response with the given status and body.
    #[must_use]
    pub fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.to_owned(),
        }
    }
}

/// One request the stub received.
#[derive(Debug, Clone)]
pub struct Seen {
    /// Value of the `X-Transmission-Session-Id` request header, if sent.
    pub session_id: Option<String>,
    /// The parsed JSON request body, if it was valid JSON.
    pub body: Option<serde_json::Value>,
    /// Value of the `Authorization` request header, if sent.
    pub authorization: Option<String>,
}

impl Seen {
    /// The RPC method name this request carried.
    #[must_use]
    pub fn method(&self) -> Option<&str> {
        self.body.as_ref()?.get("method")?.as_str()
    }

    /// The `arguments` member of this request.
    #[must_use]
    pub fn arguments(&self) -> Option<&serde_json::Value> {
        self.body.as_ref()?.get("arguments")
    }
}

/// A running stub RPC endpoint.
pub struct Stub {
    /// Host to point `TRANSMISSION_HOST` at.
    pub host: String,
    /// Port to point `TRANSMISSION_PORT` at.
    pub port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Stub {
    /// Start a stub that answers with `replies`, in order.
    ///
    /// Once the queue is down to its last entry that entry answers every
    /// further request, so a test only scripts the calls it cares about.
    pub async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the stub listener");
        let address = listener.local_addr().expect("read the stub address");
        let queue = Arc::new(Mutex::new(VecDeque::from(replies)));
        let seen = Arc::new(Mutex::new(Vec::new()));
        tokio::spawn(accept_loop(listener, queue, Arc::clone(&seen)));
        Self {
            host: address.ip().to_string(),
            port: address.port(),
            seen,
        }
    }

    /// Every request the stub has answered so far, in order.
    #[must_use]
    pub fn requests(&self) -> Vec<Seen> {
        self.seen.lock().expect("seen lock").clone()
    }

    /// A handle pointing at an address nothing is listening on.
    ///
    /// Used by tests that need the *absence* of a daemon.
    #[must_use]
    pub fn unreachable(host: &str, port: u16) -> Self {
        Self {
            host: host.to_owned(),
            port,
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

/// Accept connections and answer each with the next scripted reply.
async fn accept_loop(
    listener: TcpListener,
    queue: Arc<Mutex<VecDeque<Reply>>>,
    seen: Arc<Mutex<Vec<Seen>>>,
) {
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        let queue = Arc::clone(&queue);
        let seen = Arc::clone(&seen);
        tokio::spawn(async move {
            let _ = handle(socket, &queue, &seen).await;
        });
    }
}

/// Read one request, record it, and write the next scripted reply.
async fn handle(
    mut socket: TcpStream,
    queue: &Mutex<VecDeque<Reply>>,
    seen: &Mutex<Vec<Seen>>,
) -> std::io::Result<()> {
    let request = read_request(&mut socket).await?;
    seen.lock().expect("seen lock").push(request);

    let reply = {
        let mut queue = queue.lock().expect("queue lock");
        if queue.len() > 1 {
            queue.pop_front()
        } else {
            queue.front().cloned()
        }
        .unwrap_or_else(|| Reply::text(500, "stub has no reply left"))
    };

    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reason(reply.status),
        reply.body.len()
    );
    for (name, value) in &reply.headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");

    socket.write_all(head.as_bytes()).await?;
    socket.write_all(reply.body.as_bytes()).await?;
    socket.flush().await?;
    socket.shutdown().await
}

/// Read a full HTTP/1.1 request from `socket`.
async fn read_request(socket: &mut TcpStream) -> std::io::Result<Seen> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let head_end = loop {
        if let Some(position) = find_subslice(&buffer, b"\r\n\r\n") {
            break position + 4;
        }
        let read = socket.read(&mut chunk).await?;
        if read == 0 {
            return Ok(Seen {
                session_id: None,
                body: None,
                authorization: None,
            });
        }
        buffer.extend_from_slice(&chunk[..read]);
    };

    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut content_length = 0_usize;
    let mut session_id = None;
    let mut authorization = None;
    for line in head.lines() {
        if let Some((name, value)) = line.split_once(':') {
            let value = value.trim();
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.parse().unwrap_or(0);
            } else if name.eq_ignore_ascii_case("x-transmission-session-id") {
                session_id = Some(value.to_owned());
            } else if name.eq_ignore_ascii_case("authorization") {
                authorization = Some(value.to_owned());
            }
        }
    }

    let mut body = buffer[head_end..].to_vec();
    while body.len() < content_length {
        let read = socket.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }

    Ok(Seen {
        session_id,
        body: serde_json::from_slice(&body).ok(),
        authorization,
    })
}

/// Byte-offset of `needle` inside `haystack`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// HTTP reason phrase for a status code.
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        401 => "Unauthorized",
        403 => "Forbidden",
        409 => "Conflict",
        500 => "Internal Server Error",
        _ => "Status",
    }
}
