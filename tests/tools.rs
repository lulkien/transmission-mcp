//! Service-layer behaviour: the RPC payload each tool sends, and the text it
//! reports back.
//!
//! Arguments are built by deserializing JSON rather than by struct literals, so
//! the test exercises the same `serde` attributes the server uses — including
//! the defaults it declares.

mod common;

use common::{Reply, Stub};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use transmission_mcp::client::TransmissionClient;
use transmission_mcp::config::Config;
use transmission_mcp::tools;

fn client(stub: &Stub) -> TransmissionClient {
    TransmissionClient::new(&Config {
        host: stub.host.clone(),
        port: stub.port,
        username: None,
        password: None,
    })
    .expect("build the RPC client")
}

/// Deserialize tool arguments exactly as the server does.
fn args<T: DeserializeOwned>(value: &Value) -> T {
    serde_json::from_value(value.clone()).expect("deserialize tool arguments")
}

/// The single request the stub received.
fn only_request(stub: &Stub) -> common::Seen {
    let requests = stub.requests();
    assert_eq!(requests.len(), 1, "expected exactly one RPC call");
    requests.into_iter().next().expect("one request")
}

#[tokio::test]
async fn add_torrent_sends_a_magnet_link_as_filename() {
    let stub = Stub::start(vec![Reply::success(&json!({
        "torrent-added": { "id": 3, "name": "Debian ISO" }
    }))])
    .await;

    let message = tools::add_torrent(
        &client(&stub),
        args(&json!({ "url": "magnet:?xt=urn:btih:abc" })),
    )
    .await
    .expect("the tool reports a message");

    assert_eq!(message, "Successfully added torrent 'Debian ISO' (ID: 3)");
    let request = only_request(&stub);
    assert_eq!(request.method(), Some("torrent-add"));
    assert_eq!(
        request.arguments(),
        Some(&json!({ "filename": "magnet:?xt=urn:btih:abc", "paused": false }))
    );
}

#[tokio::test]
async fn add_torrent_sends_a_non_url_as_metainfo() {
    let stub = Stub::start(vec![Reply::success(&json!({
        "torrent-added": { "id": 4, "name": "local" }
    }))])
    .await;

    tools::add_torrent(
        &client(&stub),
        args(&json!({ "url": "ZDQ6aW5mbw==", "paused": true })),
    )
    .await
    .expect("the tool reports a message");

    assert_eq!(
        only_request(&stub).arguments(),
        Some(&json!({ "metainfo": "ZDQ6aW5mbw==", "paused": true }))
    );
}

#[tokio::test]
async fn add_torrent_passes_the_download_directory_when_given() {
    let stub = Stub::start(vec![Reply::success(&json!({
        "torrent-added": { "id": 5, "name": "Ubuntu" }
    }))])
    .await;

    tools::add_torrent(
        &client(&stub),
        args(&json!({ "url": "https://example.test/u.torrent", "download_dir": "/srv/media" })),
    )
    .await
    .expect("the tool reports a message");

    assert_eq!(
        only_request(&stub).arguments(),
        Some(&json!({
            "filename": "https://example.test/u.torrent",
            "download-dir": "/srv/media",
            "paused": false
        }))
    );
}

#[tokio::test]
async fn add_torrent_reports_a_duplicate() {
    let stub = Stub::start(vec![Reply::success(&json!({
        "torrent-duplicate": { "id": 7, "name": "Already Here" }
    }))])
    .await;

    let message = tools::add_torrent(
        &client(&stub),
        args(&json!({ "url": "magnet:?xt=urn:btih:dup" })),
    )
    .await
    .expect("the tool reports a message");

    assert_eq!(message, "Torrent already exists: 'Already Here' (ID: 7)");
}

#[tokio::test]
async fn add_torrent_reports_an_rpc_failure() {
    let stub = Stub::start(vec![Reply::json(&json!({
        "result": "invalid or corrupt torrent file",
        "arguments": {}
    }))])
    .await;

    let message = tools::add_torrent(&client(&stub), args(&json!({ "url": "not-a-torrent" })))
        .await
        .expect("the tool reports a message");

    assert_eq!(
        message,
        "Failed to add torrent: invalid or corrupt torrent file"
    );
}

#[tokio::test]
async fn add_torrent_names_a_missing_url() {
    let stub = Stub::start(vec![Reply::success(&json!({}))]).await;

    let error = tools::add_torrent(&client(&stub), args(&json!({})))
        .await
        .expect_err("a missing url is an error");

    assert_eq!(error.kind(), "invalid_argument");
    // Rendered by the server as `Error executing add_torrent: 'url'`, which is
    // the message produced for a missing key.
    assert_eq!(error.to_string(), "'url'");
    assert_eq!(stub.requests().len(), 0, "no RPC call is attempted");
}

#[tokio::test]
async fn remove_torrent_can_delete_local_data() {
    let stub = Stub::start(vec![Reply::success(&json!({}))]).await;

    let message = tools::remove_torrent(
        &client(&stub),
        args(&json!({ "torrent_id": 5, "delete_local_data": true })),
    )
    .await
    .expect("the tool reports a message");

    assert_eq!(
        message,
        "Torrent 5 successfully removed and local data deleted"
    );
    let request = only_request(&stub);
    assert_eq!(request.method(), Some("torrent-remove"));
    assert_eq!(
        request.arguments(),
        Some(&json!({ "ids": [5], "delete-local-data": true }))
    );
}

#[tokio::test]
async fn remove_torrent_keeps_local_data_by_default() {
    let stub = Stub::start(vec![Reply::success(&json!({}))]).await;

    let message = tools::remove_torrent(&client(&stub), args(&json!({ "torrent_id": 5 })))
        .await
        .expect("the tool reports a message");

    assert_eq!(message, "Torrent 5 successfully removed");
    assert_eq!(
        only_request(&stub).arguments(),
        Some(&json!({ "ids": [5], "delete-local-data": false }))
    );
}

#[tokio::test]
async fn start_and_stop_send_the_torrent_id() {
    let stub = Stub::start(vec![Reply::success(&json!({}))]).await;
    let client = client(&stub);

    let started = tools::start_torrent(&client, args(&json!({ "torrent_id": 11 })))
        .await
        .expect("start reports a message");
    assert_eq!(started, "Torrent 11 started successfully");

    let stopped = tools::stop_torrent(&client, args(&json!({ "torrent_id": 11 })))
        .await
        .expect("stop reports a message");
    assert_eq!(stopped, "Torrent 11 stopped successfully");

    let requests = stub.requests();
    assert_eq!(requests[0].method(), Some("torrent-start"));
    assert_eq!(requests[0].arguments(), Some(&json!({ "ids": [11] })));
    assert_eq!(requests[1].method(), Some("torrent-stop"));
    assert_eq!(requests[1].arguments(), Some(&json!({ "ids": [11] })));
}

#[tokio::test]
async fn get_torrent_info_renders_the_report() {
    let stub = Stub::start(vec![Reply::success(&json!({
        "torrents": [{
            "id": 3,
            "name": "Ubuntu 24.04",
            "status": 4,
            "totalSize": 1_073_741_824,
            "percentDone": 0.5,
            "rateDownload": 2048,
            "rateUpload": 512,
            "uploadRatio": 0.42,
            "eta": 3600,
            "peersConnected": 12,
            "downloadDir": "/downloads",
            "error": 0,
            "errorString": ""
        }]
    }))])
    .await;

    let message = tools::get_torrent_info(&client(&stub), args(&json!({ "torrent_id": 3 })))
        .await
        .expect("the tool reports a message");

    assert_eq!(
        message,
        "Torrent Information:\n\
         Name: Ubuntu 24.04\n\
         ID: 3\n\
         Status: Downloading\n\
         Size: 1024.00 MB\n\
         Progress: 50.0%\n\
         Download Rate: 2.0 KB/s\n\
         Upload Rate: 0.5 KB/s\n\
         Ratio: 0.42\n\
         ETA: 3600 seconds\n\
         Peers: 12\n\
         Download Dir: /downloads\n"
    );

    let arguments = only_request(&stub)
        .arguments()
        .cloned()
        .expect("the request carried arguments");
    assert_eq!(arguments["ids"], json!([3]));
    assert!(
        arguments["fields"]
            .as_array()
            .expect("fields is an array")
            .iter()
            .any(|field| field == "files"),
        "the detailed field set includes `files`"
    );
}

#[tokio::test]
async fn get_torrent_info_reports_an_unknown_id() {
    let stub = Stub::start(vec![Reply::success(&json!({ "torrents": [] }))]).await;

    let message = tools::get_torrent_info(&client(&stub), args(&json!({ "torrent_id": 9 })))
        .await
        .expect("the tool reports a message");

    assert_eq!(message, "Torrent 9 not found");
}

#[tokio::test]
async fn set_torrent_priority_maps_the_level_to_bandwidth_priority() {
    let stub = Stub::start(vec![Reply::success(&json!({}))]).await;
    let client = client(&stub);

    let high = tools::set_torrent_priority(
        &client,
        args(&json!({ "torrent_id": 5, "priority": "high" })),
    )
    .await
    .expect("the tool reports a message");
    assert_eq!(high, "Torrent 5 priority set to high");

    let low = tools::set_torrent_priority(
        &client,
        args(&json!({ "torrent_id": 6, "priority": "low" })),
    )
    .await
    .expect("the tool reports a message");
    assert_eq!(low, "Torrent 6 priority set to low");

    let requests = stub.requests();
    assert_eq!(requests[0].method(), Some("torrent-set"));
    assert_eq!(
        requests[0].arguments(),
        Some(&json!({ "ids": [5], "bandwidthPriority": 1 }))
    );
    assert_eq!(
        requests[1].arguments(),
        Some(&json!({ "ids": [6], "bandwidthPriority": -1 }))
    );
}

#[tokio::test]
async fn set_speed_limits_enables_each_limit_independently() {
    let stub = Stub::start(vec![Reply::success(&json!({}))]).await;

    let message = tools::set_speed_limits(
        &client(&stub),
        args(&json!({ "download_limit": 500, "upload_limit": 0 })),
    )
    .await
    .expect("the tool reports a message");

    assert_eq!(message, "Speed limits updated successfully");
    let request = only_request(&stub);
    assert_eq!(request.method(), Some("session-set"));
    assert_eq!(
        request.arguments(),
        Some(&json!({
            "speed-limit-down-enabled": true,
            "speed-limit-down": 500,
            "speed-limit-up-enabled": false,
            "speed-limit-up": 0
        }))
    );
}

#[tokio::test]
async fn set_speed_limits_with_no_limits_sends_an_empty_argument_map() {
    let stub = Stub::start(vec![Reply::success(&json!({}))]).await;

    tools::set_speed_limits(&client(&stub), args(&json!({})))
        .await
        .expect("the tool reports a message");

    assert_eq!(only_request(&stub).arguments(), Some(&json!({})));
}

#[tokio::test]
async fn search_torrents_filters_by_name_and_status() {
    let stub = Stub::start(vec![Reply::success(&json!({
        "torrents": [
            { "id": 1, "name": "Ubuntu 24.04", "status": 4, "percentDone": 0.25 },
            { "id": 2, "name": "ubuntu server", "status": 6, "percentDone": 1.0 },
            { "id": 3, "name": "Debian 12", "status": 4, "percentDone": 0.1 },
            { "id": 4, "name": "UBUNTU images", "status": 0, "percentDone": 0.75 }
        ]
    }))])
    .await;

    let all = tools::search_torrents(&client(&stub), args(&json!({ "query": "ubuntu" })))
        .await
        .expect("the tool reports a message");
    assert_eq!(
        all,
        "Found 3 matching torrents:\n\
         ID: 1 | Ubuntu 24.04 | Status: Downloading | Progress: 25.0%\n\
         ID: 2 | ubuntu server | Status: Seeding | Progress: 100.0%\n\
         ID: 4 | UBUNTU images | Status: Stopped | Progress: 75.0%"
    );

    let seeding = tools::search_torrents(
        &client(&stub),
        args(&json!({ "query": "ubuntu", "status_filter": "seeding" })),
    )
    .await
    .expect("the tool reports a message");
    assert_eq!(
        seeding,
        "Found 1 matching torrents:\nID: 2 | ubuntu server | Status: Seeding | Progress: 100.0%"
    );

    let none = tools::search_torrents(&client(&stub), args(&json!({ "query": "gentoo" })))
        .await
        .expect("the tool reports a message");
    assert_eq!(none, "No torrents found matching the search criteria");
}

#[tokio::test]
async fn search_torrents_treats_a_missing_filter_as_all() {
    let stub = Stub::start(vec![Reply::success(&json!({
        "torrents": [{ "id": 1, "name": "match", "status": 0, "percentDone": 0.0 }]
    }))])
    .await;

    let found = tools::search_torrents(&client(&stub), args(&json!({ "query": "match" })))
        .await
        .expect("the tool reports a message");

    assert!(found.starts_with("Found 1 matching torrents:"));
}

#[tokio::test]
async fn get_session_stats_renders_the_report() {
    let stub = Stub::start(vec![Reply::success(&json!({
        "activeTorrentCount": 2,
        "pausedTorrentCount": 1,
        "torrentCount": 5,
        "current-stats": {
            "downloadSpeed": 1536,
            "uploadSpeed": 512,
            "downloadedBytes": 3_145_728,
            "uploadedBytes": 524_288,
            "filesAdded": 3
        },
        "cumulative-stats": {
            "downloadedBytes": 2_147_483_648_i64,
            "uploadedBytes": 1_073_741_824,
            "filesAdded": 10,
            "sessionCount": 4,
            "secondsActive": 7200
        }
    }))])
    .await;

    let message = tools::get_session_stats(&client(&stub))
        .await
        .expect("the tool reports a message");

    assert_eq!(
        message,
        "Transmission Session Statistics:\n\
         \n\
         Current Session:\n\
         - Download Speed: 1.5 KB/s\n\
         - Upload Speed: 0.5 KB/s\n\
         - Downloaded: 3.00 MB\n\
         - Uploaded: 0.50 MB\n\
         - Files Added: 3\n\
         - Active Torrents: 2\n\
         - Paused Torrents: 1\n\
         - Total Torrents: 5\n\
         \n\
         Cumulative:\n\
         - Downloaded: 2.00 GB\n\
         - Uploaded: 1.00 GB\n\
         - Files Added: 10\n\
         - Sessions: 4\n\
         - Uptime: 2.0 hours\n"
    );
    assert_eq!(only_request(&stub).method(), Some("session-stats"));
}
