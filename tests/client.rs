//! RPC client behaviour: the CSRF handshake and the error taxonomy.

mod common;

use common::{Reply, Stub};
use serde_json::json;
use transmission_mcp::client::TransmissionClient;
use transmission_mcp::config::Config;

fn config(stub: &Stub) -> Config {
    Config {
        host: stub.host.clone(),
        port: stub.port,
        username: None,
        password: None,
    }
}

fn client(stub: &Stub) -> TransmissionClient {
    TransmissionClient::new(&config(stub)).expect("build the RPC client")
}

#[tokio::test]
async fn repeats_the_request_once_a_conflict_reveals_the_session_id() {
    let stub = Stub::start(vec![
        Reply::conflict("session-abc"),
        Reply::success(&json!({ "version": "4.0.6" })),
    ])
    .await;

    let response = client(&stub)
        .request("session-get", json!({}))
        .await
        .expect("the retry succeeds");
    assert_eq!(response["arguments"]["version"], "4.0.6");

    let requests = stub.requests();
    assert_eq!(requests.len(), 2, "the request is sent again after a 409");
    assert_eq!(
        requests[0].session_id, None,
        "the first attempt carries no session id"
    );
    assert_eq!(
        requests[1].session_id.as_deref(),
        Some("session-abc"),
        "the retry carries the id from the 409 response"
    );
    assert_eq!(requests[0].method(), Some("session-get"));
    assert_eq!(requests[1].method(), Some("session-get"));
}

#[tokio::test]
async fn caches_the_session_id_for_later_calls() {
    let stub = Stub::start(vec![
        Reply::conflict("session-abc"),
        Reply::success(&json!({})),
    ])
    .await;

    let client = client(&stub);
    client
        .request("session-get", json!({}))
        .await
        .expect("first call performs the handshake");
    client
        .request("session-stats", json!({}))
        .await
        .expect("second call reuses the cached id");

    let requests = stub.requests();
    assert_eq!(requests.len(), 3, "one handshake plus two calls");
    assert_eq!(
        requests[2].session_id.as_deref(),
        Some("session-abc"),
        "the second call presents the cached id without a 409"
    );
}

#[tokio::test]
async fn reports_a_second_conflict_as_an_http_error() {
    let stub = Stub::start(vec![Reply::conflict("one"), Reply::conflict("two")]).await;

    let error = client(&stub)
        .request("session-get", json!({}))
        .await
        .expect_err("a 409 that survives the retry is an error");

    assert_eq!(error.kind(), "http_status");
    assert_eq!(error.to_string(), "HTTP error 409: 409: Conflict");
    assert_eq!(
        stub.requests().len(),
        2,
        "the handshake is retried once only"
    );
}

#[tokio::test]
async fn reports_a_non_success_status_with_its_body() {
    let stub = Stub::start(vec![Reply::text(500, "daemon exploded")]).await;

    let error = client(&stub)
        .request("torrent-get", json!({}))
        .await
        .expect_err("a 500 is an error");

    assert_eq!(error.kind(), "http_status");
    assert_eq!(error.to_string(), "HTTP error 500: daemon exploded");
}

#[tokio::test]
async fn reports_a_non_json_body_as_a_decode_error() {
    let stub = Stub::start(vec![Reply::text(200, "<html>not json</html>")]).await;

    let error = client(&stub)
        .request("session-get", json!({}))
        .await
        .expect_err("an unparseable body is an error");

    assert_eq!(error.kind(), "decode");
    assert!(
        error.to_string().starts_with("Invalid response: "),
        "unexpected message: {error}"
    );
}

#[tokio::test]
async fn reports_a_refused_connection_as_a_network_error() {
    // Bind and immediately drop, so the port is free but nothing listens.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a throwaway listener");
    let port = listener.local_addr().expect("read the address").port();
    drop(listener);

    let client = TransmissionClient::new(&Config {
        host: "127.0.0.1".to_owned(),
        port,
        username: None,
        password: None,
    })
    .expect("build the RPC client");

    let error = client
        .request("session-get", json!({}))
        .await
        .expect_err("a refused connection is an error");

    assert_eq!(error.kind(), "network");
    assert!(
        error.to_string().starts_with("Network error: "),
        "unexpected message: {error}"
    );
    assert!(
        error.to_string().contains("connect"),
        "the cause chain should name the failure, got: {error}"
    );
}

#[tokio::test]
async fn sends_basic_auth_when_credentials_are_configured() {
    let stub = Stub::start(vec![Reply::success(&json!({}))]).await;
    let client = TransmissionClient::new(&Config {
        host: stub.host.clone(),
        port: stub.port,
        username: Some("admin".to_owned()),
        password: Some("hunter2".to_owned()),
    })
    .expect("build the RPC client");

    assert!(client.authenticated());
    client
        .request("session-get", json!({}))
        .await
        .expect("the call succeeds");

    let requests = stub.requests();
    assert_eq!(
        requests[0].authorization.as_deref(),
        // base64("admin:hunter2")
        Some("Basic YWRtaW46aHVudGVyMg=="),
        "credentials are sent as HTTP basic auth"
    );
}

#[tokio::test]
async fn omits_basic_auth_when_credentials_are_empty() {
    let stub = Stub::start(vec![Reply::success(&json!({}))]).await;
    let client = TransmissionClient::new(&Config {
        host: stub.host.clone(),
        port: stub.port,
        username: Some(String::new()),
        password: Some(String::new()),
    })
    .expect("build the RPC client");

    assert!(!client.authenticated(), "empty credentials mean no auth");
    client
        .request("session-get", json!({}))
        .await
        .expect("the call succeeds");
    assert_eq!(stub.requests()[0].authorization, None);
}

#[test]
fn builds_the_rpc_url_from_host_and_port() {
    let config = Config {
        host: "seedbox.internal".to_owned(),
        port: 9091,
        username: None,
        password: None,
    };
    assert_eq!(
        config.rpc_url(),
        "http://seedbox.internal:9091/transmission/rpc"
    );
}

#[test]
fn defaults_host_and_port() {
    let config = Config::default();
    assert_eq!(config.rpc_url(), "http://localhost:9091/transmission/rpc");
    assert_eq!(config.basic_auth(), None);
}
