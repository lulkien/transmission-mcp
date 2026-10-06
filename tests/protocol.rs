//! Protocol-level tests: drive the real binary over stdio.
//!
//! These spawn `transmission-mcp` exactly as an MCP client would, point it at
//! the scripted RPC stub, and speak newline-delimited JSON-RPC to it. A hung
//! server fails the test rather than hanging the suite.

mod common;

use std::process::Stdio;
use std::time::Duration;

use common::{Reply, Seen, Stub};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::timeout;

/// Protocol version the test client negotiates.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// How long a single response may take before the test fails.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

/// A spawned server plus the JSON-RPC plumbing to talk to it.
struct Harness {
    /// Held so the process is killed when the harness is dropped.
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Harness {
    /// Spawn the server against `stub`.
    ///
    /// Starting the process needs no `.await`; reading its replies does.
    fn start(stub: &Stub) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_transmission-mcp"))
            .env("TRANSMISSION_HOST", &stub.host)
            .env("TRANSMISSION_PORT", stub.port.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn the transmission-mcp binary");

        let stdin = child.stdin.take().expect("child stdin");
        let stdout = BufReader::new(child.stdout.take().expect("child stdout"));
        Self {
            _child: child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    /// Send a request and return the response carrying its id.
    async fn request(&mut self, method: &str, params: &Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await;
        self.response(id).await
    }

    /// Send a notification (no id, no response).
    async fn notify(&mut self, method: &str, params: &Value) {
        self.write(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .await;
    }

    /// Perform the MCP handshake and return the `initialize` result.
    async fn initialize(&mut self) -> Value {
        let response = self
            .request(
                "initialize",
                &json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "transmission-mcp-tests", "version": "0.0.0" }
                }),
            )
            .await;
        self.notify("notifications/initialized", &json!({})).await;
        response
    }

    /// Call a tool and return its result object.
    async fn call_tool(&mut self, name: &str, arguments: &Value) -> Value {
        let response = self
            .request(
                "tools/call",
                &json!({ "name": name, "arguments": arguments }),
            )
            .await;
        assert!(
            response.get("error").is_none(),
            "tools/call {name} failed at the protocol level: {response}"
        );
        response["result"].clone()
    }

    /// The text of the first content block of a tool result.
    fn text_of(result: &Value) -> &str {
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("no text content in {result}"))
    }

    /// Write one JSON-RPC message as a line.
    async fn write(&mut self, message: &Value) {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .expect("write to the server");
        self.stdin.flush().await.expect("flush to the server");
    }

    /// Read messages until the one carrying `id` arrives.
    async fn response(&mut self, id: i64) -> Value {
        loop {
            let message = self.read().await;
            if message.get("id").and_then(Value::as_i64) == Some(id) {
                return message;
            }
        }
    }

    /// Read one JSON-RPC message.
    async fn read(&mut self) -> Value {
        let mut line = String::new();
        let read = timeout(RESPONSE_TIMEOUT, self.stdout.read_line(&mut line))
            .await
            .expect("the server did not answer in time")
            .expect("read from the server");
        assert!(read > 0, "the server closed its stdout");
        serde_json::from_str(&line)
            .unwrap_or_else(|error| panic!("server sent a non-JSON line {line:?}: {error}"))
    }
}

/// The single request the stub received.
fn only_request(stub: &Stub) -> Seen {
    let requests = stub.requests();
    assert_eq!(requests.len(), 1, "expected exactly one RPC call");
    requests.into_iter().next().expect("one request")
}

#[tokio::test]
async fn initialize_advertises_the_tools_and_resources_capabilities() {
    let stub = Stub::start(vec![Reply::success(&json!({}))]).await;
    let mut harness = Harness::start(&stub);

    let result = harness.initialize().await["result"].clone();

    assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(result["serverInfo"]["name"], "transmission-mcp");
    assert!(
        result["capabilities"]["tools"].is_object(),
        "tools capability missing: {result}"
    );
    assert!(
        result["capabilities"]["resources"].is_object(),
        "resources capability missing: {result}"
    );
}

#[tokio::test]
async fn tools_list_preserves_the_original_tool_surface() {
    let stub = Stub::start(vec![Reply::success(&json!({}))]).await;
    let mut harness = Harness::start(&stub);
    harness.initialize().await;

    let response = harness.request("tools/list", &json!({})).await;
    let tools = response["result"]["tools"]
        .as_array()
        .expect("tools is an array")
        .clone();

    let mut names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "add_torrent",
            "get_session_stats",
            "get_torrent_info",
            "remove_torrent",
            "search_torrents",
            "set_speed_limits",
            "set_torrent_priority",
            "start_torrent",
            "stop_torrent",
        ]
    );

    let find = |name: &str| {
        tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("tool {name} is missing"))
    };

    assert_eq!(
        find("add_torrent")["description"],
        "Add a new torrent by URL or magnet link"
    );
    assert_eq!(
        find("get_session_stats")["description"],
        "Get Transmission session statistics"
    );

    let add = &find("add_torrent")["inputSchema"];
    assert_eq!(add["type"], "object");
    assert_eq!(add["properties"]["url"]["type"], "string");
    assert_eq!(
        add["properties"]["url"]["description"],
        "Torrent URL, magnet link, or base64-encoded .torrent file"
    );
    assert_eq!(add["properties"]["paused"]["type"], "boolean");
    assert_eq!(add["properties"]["paused"]["default"], false);
    assert_eq!(add["required"], json!(["url"]));

    let priority = &find("set_torrent_priority")["inputSchema"];
    assert_eq!(
        priority["properties"]["priority"]["enum"],
        json!(["high", "normal", "low"])
    );
    assert_eq!(priority["required"], json!(["torrent_id", "priority"]));

    let search = &find("search_torrents")["inputSchema"];
    assert_eq!(
        search["properties"]["status_filter"]["enum"],
        json!(["all", "downloading", "seeding", "paused", "completed"])
    );
    assert_eq!(search["properties"]["status_filter"]["default"], "all");
    assert_eq!(search["required"], json!(["query"]));

    let limits = &find("set_speed_limits")["inputSchema"];
    assert_eq!(
        limits["properties"]["download_limit"]["type"],
        // An optional numeric argument is advertised as nullable: schemars
        // cannot express "optional" without it. A null value is treated as
        // "not supplied", which is what omitting the key did before.
        json!(["integer", "null"])
    );
    assert_eq!(
        limits["properties"]["download_limit"]["description"],
        "Download speed limit in KB/s (0 = unlimited)"
    );
    assert!(
        limits["required"].is_null(),
        "set_speed_limits takes no required argument: {limits}"
    );

    assert_eq!(
        find("get_session_stats")["inputSchema"],
        // No argument struct means no properties, exactly as before.
        json!({ "type": "object", "properties": {} })
    );
}

#[tokio::test]
async fn tools_call_returns_the_rendered_report() {
    let stub = Stub::start(vec![Reply::success(&json!({
        "torrents": [{
            "id": 3,
            "name": "Ubuntu 24.04",
            "status": 6,
            "totalSize": 2_147_483_648_i64,
            "percentDone": 1.0,
            "rateDownload": 0,
            "rateUpload": 1024,
            "uploadRatio": 1.5,
            "eta": -1,
            "peersConnected": 4,
            "downloadDir": "/downloads",
            "error": 3,
            "errorString": "No data found"
        }]
    }))])
    .await;
    let mut harness = Harness::start(&stub);
    harness.initialize().await;

    let result = harness
        .call_tool("get_torrent_info", &json!({ "torrent_id": 3 }))
        .await;

    assert_eq!(result["isError"], false);
    assert_eq!(
        Harness::text_of(&result),
        "Torrent Information:\n\
         Name: Ubuntu 24.04\n\
         ID: 3\n\
         Status: Seeding\n\
         Size: 2048.00 MB\n\
         Progress: 100.0%\n\
         Download Rate: 0.0 KB/s\n\
         Upload Rate: 1.0 KB/s\n\
         Ratio: 1.50\n\
         ETA: Unknown seconds\n\
         Peers: 4\n\
         Download Dir: /downloads\n\
         Error: No data found\n"
    );

    let request = only_request(&stub);
    assert_eq!(request.method(), Some("torrent-get"));
    assert_eq!(request.arguments().expect("arguments")["ids"], json!([3]));
}

#[tokio::test]
async fn tools_call_reports_a_missing_argument_as_text() {
    let stub = Stub::start(vec![Reply::success(&json!({}))]).await;
    let mut harness = Harness::start(&stub);
    harness.initialize().await;

    let result = harness.call_tool("add_torrent", &json!({})).await;

    assert_eq!(
        Harness::text_of(&result),
        "Error executing add_torrent: 'url'"
    );
    assert_eq!(
        result["isError"], false,
        "a tool-level failure stays ordinary content, as before"
    );
}

#[tokio::test]
async fn tools_call_reports_an_unreachable_daemon_as_text() {
    // A port with nothing listening: the call fails at the transport layer.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a throwaway listener");
    let port = listener.local_addr().expect("address").port();
    drop(listener);

    let mut harness = Harness::start(&Stub::unreachable("127.0.0.1", port));
    harness.initialize().await;

    let result = harness.call_tool("get_session_stats", &json!({})).await;
    let text = Harness::text_of(&result);

    assert!(
        text.starts_with("Error executing get_session_stats: Network error: "),
        "unexpected message: {text}"
    );
}

#[tokio::test]
async fn tools_call_rejects_an_unknown_tool() {
    let stub = Stub::start(vec![Reply::success(&json!({}))]).await;
    let mut harness = Harness::start(&stub);
    harness.initialize().await;

    let response = harness
        .request(
            "tools/call",
            &json!({ "name": "no_such_tool", "arguments": {} }),
        )
        .await;

    assert_eq!(response["error"]["code"], -32602);
    assert_eq!(response["error"]["message"], "tool not found");
}

#[tokio::test]
async fn resources_list_matches_the_original_three() {
    let stub = Stub::start(vec![Reply::success(&json!({}))]).await;
    let mut harness = Harness::start(&stub);
    harness.initialize().await;

    let response = harness.request("resources/list", &json!({})).await;
    let resources = response["result"]["resources"]
        .as_array()
        .expect("resources is an array")
        .clone();

    let described: Vec<(&str, &str)> = resources
        .iter()
        .map(|resource| {
            (
                resource["uri"].as_str().expect("uri"),
                resource["name"].as_str().expect("name"),
            )
        })
        .collect();
    assert_eq!(
        described,
        [
            ("transmission://session", "Transmission Session Info"),
            ("transmission://torrents", "All Torrents"),
            ("transmission://stats", "Transmission Statistics"),
        ]
    );
    assert_eq!(resources[0]["mimeType"], "application/json");
}

#[tokio::test]
async fn resources_read_returns_pretty_printed_arguments() {
    let arguments = json!({
        "activeTorrentCount": 2,
        "torrentCount": 5
    });
    let stub = Stub::start(vec![Reply::success(&arguments)]).await;
    let mut harness = Harness::start(&stub);
    harness.initialize().await;

    let response = harness
        .request("resources/read", &json!({ "uri": "transmission://stats" }))
        .await;
    let content = &response["result"]["contents"][0];

    assert_eq!(content["uri"], "transmission://stats");
    assert_eq!(content["mimeType"], "application/json");
    assert_eq!(
        content["text"].as_str().expect("text"),
        serde_json::to_string_pretty(&arguments).expect("pretty print")
    );
    assert_eq!(only_request(&stub).method(), Some("session-stats"));
}

#[tokio::test]
async fn resources_read_fetches_the_torrent_list() {
    let stub = Stub::start(vec![Reply::success(&json!({ "torrents": [] }))]).await;
    let mut harness = Harness::start(&stub);
    harness.initialize().await;

    harness
        .request(
            "resources/read",
            &json!({ "uri": "transmission://torrents" }),
        )
        .await;

    let request = only_request(&stub);
    assert_eq!(request.method(), Some("torrent-get"));
    let fields = request.arguments().expect("arguments")["fields"]
        .as_array()
        .expect("fields is an array")
        .clone();
    assert!(fields.iter().any(|field| field == "trackerStats"));
    assert!(
        !fields.iter().any(|field| field == "files"),
        "the list resource uses the shorter field set"
    );
}

#[tokio::test]
async fn resources_read_reports_a_failure_as_text() {
    let stub = Stub::start(vec![Reply::text(500, "daemon exploded")]).await;
    let mut harness = Harness::start(&stub);
    harness.initialize().await;

    let response = harness
        .request(
            "resources/read",
            &json!({ "uri": "transmission://session" }),
        )
        .await;
    let content = &response["result"]["contents"][0];

    assert_eq!(
        content["text"].as_str().expect("text"),
        "Error reading resource transmission://session: HTTP error 500: daemon exploded"
    );
}

#[tokio::test]
async fn resources_read_reports_an_unknown_uri_as_text() {
    let stub = Stub::start(vec![Reply::success(&json!({}))]).await;
    let mut harness = Harness::start(&stub);
    harness.initialize().await;

    let response = harness
        .request("resources/read", &json!({ "uri": "transmission://nope" }))
        .await;

    assert_eq!(
        response["result"]["contents"][0]["text"]
            .as_str()
            .expect("text"),
        "Error reading resource transmission://nope: Unknown resource: transmission://nope"
    );
    assert_eq!(stub.requests().len(), 0, "no RPC call is attempted");
}

#[tokio::test]
async fn refuses_to_start_on_an_invalid_port() {
    let output = Command::new(env!("CARGO_BIN_EXE_transmission-mcp"))
        .env("TRANSMISSION_PORT", "not-a-port")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .expect("run the binary");

    assert!(
        !output.status.success(),
        "the server should refuse to start"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("TRANSMISSION_PORT is not a valid port"),
        "unexpected stderr: {stderr}"
    );
    assert!(output.stdout.is_empty(), "nothing is written to stdout");
}
