//! Tool argument schemas and the service function behind each MCP tool.
//!
//! The `#[tool]` methods in [`crate::server`] are thin adapters over these
//! functions so the behaviour is testable without a transport.
//!
//! Argument fields are `Option<T>` and carry `#[schemars(required)]` on the
//! arguments that are mandatory. That combination is deliberate: the generated
//! schema advertises a `required` list, while serde still deserializes a missing
//! field to `None` so the service function can report
//! `Error executing <tool>: '<name>'` as ordinary text instead of letting the
//! call fail at the protocol layer.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::client::TransmissionClient;
use crate::error::Error;
use crate::format;

/// Fields fetched for a single-torrent lookup.
const TORRENT_INFO_FIELDS: [&str; 21] = [
    "id",
    "name",
    "status",
    "totalSize",
    "percentDone",
    "rateDownload",
    "rateUpload",
    "uploadRatio",
    "eta",
    "peersConnected",
    "downloadDir",
    "error",
    "errorString",
    "addedDate",
    "doneDate",
    "trackerStats",
    "files",
    "fileStats",
    "pieces",
    "pieceCount",
    "pieceSize",
];

/// Fields fetched for the `transmission://torrents` resource.
pub const TORRENT_LIST_FIELDS: [&str; 16] = [
    "id",
    "name",
    "status",
    "totalSize",
    "percentDone",
    "rateDownload",
    "rateUpload",
    "uploadRatio",
    "eta",
    "peersConnected",
    "downloadDir",
    "error",
    "errorString",
    "addedDate",
    "doneDate",
    "trackerStats",
];

/// Fields fetched by `search_torrents`, a smaller set than the full listing.
const SEARCH_FIELDS: [&str; 7] = [
    "id",
    "name",
    "status",
    "totalSize",
    "percentDone",
    "rateDownload",
    "rateUpload",
];

/// Arguments of `add_torrent`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddTorrentArgs {
    /// Torrent URL, magnet link, or base64-encoded .torrent file
    #[schemars(required)]
    pub url: Option<String>,
    /// Download directory (optional)
    pub download_dir: Option<String>,
    /// Start paused (optional, default false)
    #[serde(default)]
    pub paused: bool,
}

/// Arguments of `remove_torrent`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveTorrentArgs {
    /// Torrent ID to remove
    #[schemars(required)]
    pub torrent_id: Option<i64>,
    /// Also delete local data (default false)
    #[serde(default)]
    pub delete_local_data: bool,
}

/// Arguments of `start_torrent`, `stop_torrent` and `get_torrent_info`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TorrentIdArgs {
    /// Torrent ID
    #[schemars(required)]
    pub torrent_id: Option<i64>,
}

/// Download priority levels accepted by `set_torrent_priority`.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// Favoured over normal torrents.
    High,
    /// The default priority.
    Normal,
    /// Only runs when nothing else wants bandwidth.
    Low,
}

/// A `{"type": "string", "enum": [...]}` schema.
///
/// schemars writes a unit-variant enum as a `oneOf` of `const` branches, which
/// is not the shape these tools have always advertised — the schema is part of
/// the wire contract, so it is written out by hand. Each enum keeps the value
/// list next to the variants it mirrors.
fn string_enum(values: &[&str]) -> Schema {
    let mut schema = serde_json::Map::new();
    schema.insert("type".to_owned(), Value::String("string".to_owned()));
    schema.insert("enum".to_owned(), json!(values));
    schema.into()
}

impl JsonSchema for Priority {
    fn schema_name() -> Cow<'static, str> {
        "Priority".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        string_enum(&Priority::NAMES)
    }

    fn inline_schema() -> bool {
        true
    }
}

impl Priority {
    /// The values the tool schema advertises, in declaration order.
    const NAMES: [&'static str; 3] = ["high", "normal", "low"];

    /// The `bandwidthPriority` value Transmission expects.
    const fn bandwidth(self) -> i64 {
        match self {
            Self::High => 1,
            Self::Normal => 0,
            Self::Low => -1,
        }
    }

    /// The word the caller used, for the confirmation message.
    const fn label(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Normal => "normal",
            Self::Low => "low",
        }
    }
}

/// Arguments of `set_torrent_priority`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetPriorityArgs {
    /// Torrent ID
    #[schemars(required)]
    pub torrent_id: Option<i64>,
    /// Priority level
    #[schemars(required)]
    pub priority: Option<Priority>,
}

/// Arguments of `set_speed_limits`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetSpeedLimitsArgs {
    /// Download speed limit in KB/s (0 = unlimited)
    pub download_limit: Option<i64>,
    /// Upload speed limit in KB/s (0 = unlimited)
    pub upload_limit: Option<i64>,
}

/// Status filters accepted by `search_torrents`.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusFilter {
    /// No status filtering.
    #[default]
    All,
    /// Torrents actively downloading (status 4).
    Downloading,
    /// Torrents seeding (status 6).
    Seeding,
    /// Torrents stopped by the user (status 0).
    Paused,
    /// Torrents that finished downloading. Transmission has no separate
    /// "completed" status, so this filter selects seeding (status 6).
    Completed,
}

impl JsonSchema for StatusFilter {
    fn schema_name() -> Cow<'static, str> {
        "StatusFilter".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        string_enum(&StatusFilter::NAMES)
    }

    fn inline_schema() -> bool {
        true
    }
}

impl StatusFilter {
    /// The values the tool schema advertises, in declaration order.
    const NAMES: [&'static str; 5] = ["all", "downloading", "seeding", "paused", "completed"];

    /// The status code this filter selects, or `None` for `All`.
    const fn code(self) -> Option<i64> {
        match self {
            Self::All => None,
            Self::Downloading => Some(4),
            Self::Seeding | Self::Completed => Some(6),
            Self::Paused => Some(0),
        }
    }

    /// Whether a torrent matches this filter.
    #[must_use]
    pub fn matches(self, torrent: &Value) -> bool {
        match self.code() {
            None => true,
            Some(code) => torrent.get("status").and_then(Value::as_i64) == Some(code),
        }
    }
}

/// Arguments of `search_torrents`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchTorrentsArgs {
    /// Search query (torrent name)
    #[schemars(required)]
    pub query: Option<String>,
    /// Filter by status (optional)
    #[serde(default)]
    pub status_filter: StatusFilter,
}

/// `true` when Transmission answered `result: "success"`.
fn succeeded(response: &Value) -> bool {
    response.get("result").and_then(Value::as_str) == Some("success")
}

/// The `result` string of a failed response.
fn failure_reason(response: &Value) -> String {
    response
        .get("result")
        .map_or_else(|| "Unknown error".to_owned(), format::json_text)
}

/// Read a required string field, naming the field on failure.
fn required_str<'a>(object: &'a Value, key: &str) -> Result<&'a str, Error> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Invalid(format!("'{key}'")))
}

/// Read a required field of any type, naming the field on failure.
fn required_value(object: &Value, key: &str) -> Result<String, Error> {
    object
        .get(key)
        .map(format::json_text)
        .ok_or_else(|| Error::Invalid(format!("'{key}'")))
}

/// `arguments` of a response envelope, or an empty object when absent.
fn arguments(response: &Value) -> &Value {
    response.get("arguments").unwrap_or(&Value::Null)
}

/// Add a torrent by URL, magnet link or base64-encoded metainfo.
pub async fn add_torrent(
    client: &TransmissionClient,
    args: AddTorrentArgs,
) -> Result<String, Error> {
    let url = args.url.ok_or_else(|| Error::Invalid("'url'".to_owned()))?;

    let mut rpc = serde_json::Map::new();
    if url.starts_with("magnet:") || url.starts_with("http") {
        rpc.insert("filename".to_owned(), Value::String(url));
    } else {
        rpc.insert("metainfo".to_owned(), Value::String(url));
    }
    if let Some(directory) = args.download_dir.filter(|dir| !dir.is_empty()) {
        rpc.insert("download-dir".to_owned(), Value::String(directory));
    }
    rpc.insert("paused".to_owned(), Value::Bool(args.paused));

    let response = client.request("torrent-add", Value::Object(rpc)).await?;
    if !succeeded(&response) {
        return Ok(format!(
            "Failed to add torrent: {}",
            failure_reason(&response)
        ));
    }

    let arguments = arguments(&response);
    if let Some(added) = arguments.get("torrent-added") {
        return Ok(format!(
            "Successfully added torrent '{}' (ID: {})",
            required_str(added, "name")?,
            required_value(added, "id")?
        ));
    }
    if let Some(duplicate) = arguments.get("torrent-duplicate") {
        return Ok(format!(
            "Torrent already exists: '{}' (ID: {})",
            required_str(duplicate, "name")?,
            required_value(duplicate, "id")?
        ));
    }
    Ok("Torrent added successfully".to_owned())
}

/// Remove a torrent, optionally deleting its data.
pub async fn remove_torrent(
    client: &TransmissionClient,
    args: RemoveTorrentArgs,
) -> Result<String, Error> {
    let id = args
        .torrent_id
        .ok_or_else(|| Error::Invalid("'torrent_id'".to_owned()))?;
    let delete_local_data = args.delete_local_data;

    let response = client
        .request(
            "torrent-remove",
            json!({ "ids": [id], "delete-local-data": delete_local_data }),
        )
        .await?;

    if succeeded(&response) {
        let action = if delete_local_data {
            "removed and local data deleted"
        } else {
            "removed"
        };
        Ok(format!("Torrent {id} successfully {action}"))
    } else {
        Ok(format!(
            "Failed to remove torrent: {}",
            failure_reason(&response)
        ))
    }
}

/// Start (or resume) one torrent.
pub async fn start_torrent(
    client: &TransmissionClient,
    args: TorrentIdArgs,
) -> Result<String, Error> {
    let id = args
        .torrent_id
        .ok_or_else(|| Error::Invalid("'torrent_id'".to_owned()))?;

    let response = client
        .request("torrent-start", json!({ "ids": [id] }))
        .await?;

    if succeeded(&response) {
        Ok(format!("Torrent {id} started successfully"))
    } else {
        Ok(format!(
            "Failed to start torrent: {}",
            failure_reason(&response)
        ))
    }
}

/// Stop (pause) one torrent.
pub async fn stop_torrent(
    client: &TransmissionClient,
    args: TorrentIdArgs,
) -> Result<String, Error> {
    let id = args
        .torrent_id
        .ok_or_else(|| Error::Invalid("'torrent_id'".to_owned()))?;

    let response = client
        .request("torrent-stop", json!({ "ids": [id] }))
        .await?;

    if succeeded(&response) {
        Ok(format!("Torrent {id} stopped successfully"))
    } else {
        Ok(format!(
            "Failed to stop torrent: {}",
            failure_reason(&response)
        ))
    }
}

/// Detailed report for one torrent.
pub async fn get_torrent_info(
    client: &TransmissionClient,
    args: TorrentIdArgs,
) -> Result<String, Error> {
    let id = args
        .torrent_id
        .ok_or_else(|| Error::Invalid("'torrent_id'".to_owned()))?;

    let response = client
        .request(
            "torrent-get",
            json!({ "ids": [id], "fields": TORRENT_INFO_FIELDS }),
        )
        .await?;

    if !succeeded(&response) {
        return Ok(format!(
            "Failed to get torrent info: {}",
            failure_reason(&response)
        ));
    }

    let torrent = arguments(&response)
        .get("torrents")
        .and_then(Value::as_array)
        .and_then(|torrents| torrents.first());

    match torrent {
        Some(torrent) => Ok(format::torrent_info(torrent)),
        None => Ok(format!("Torrent {id} not found")),
    }
}

/// Set the bandwidth priority of one torrent.
pub async fn set_torrent_priority(
    client: &TransmissionClient,
    args: SetPriorityArgs,
) -> Result<String, Error> {
    let id = args
        .torrent_id
        .ok_or_else(|| Error::Invalid("'torrent_id'".to_owned()))?;
    let priority = args
        .priority
        .ok_or_else(|| Error::Invalid("'priority'".to_owned()))?;

    let response = client
        .request(
            "torrent-set",
            json!({ "ids": [id], "bandwidthPriority": priority.bandwidth() }),
        )
        .await?;

    if succeeded(&response) {
        Ok(format!("Torrent {id} priority set to {}", priority.label()))
    } else {
        Ok(format!(
            "Failed to set priority: {}",
            failure_reason(&response)
        ))
    }
}

/// Set the global download and/or upload speed limits.
pub async fn set_speed_limits(
    client: &TransmissionClient,
    args: SetSpeedLimitsArgs,
) -> Result<String, Error> {
    let mut rpc = serde_json::Map::new();
    if let Some(limit) = args.download_limit {
        rpc.insert(
            "speed-limit-down-enabled".to_owned(),
            Value::Bool(limit > 0),
        );
        rpc.insert("speed-limit-down".to_owned(), json!(limit));
    }
    if let Some(limit) = args.upload_limit {
        rpc.insert("speed-limit-up-enabled".to_owned(), Value::Bool(limit > 0));
        rpc.insert("speed-limit-up".to_owned(), json!(limit));
    }

    let response = client.request("session-set", Value::Object(rpc)).await?;

    if succeeded(&response) {
        Ok("Speed limits updated successfully".to_owned())
    } else {
        Ok(format!(
            "Failed to set speed limits: {}",
            failure_reason(&response)
        ))
    }
}

/// Search torrents by name, optionally filtered by status.
pub async fn search_torrents(
    client: &TransmissionClient,
    args: SearchTorrentsArgs,
) -> Result<String, Error> {
    let query = args
        .query
        .ok_or_else(|| Error::Invalid("'query'".to_owned()))?;
    let filter = args.status_filter;

    let response = client
        .request("torrent-get", json!({ "fields": SEARCH_FIELDS }))
        .await?;

    if !succeeded(&response) {
        return Ok(format!(
            "Failed to search torrents: {}",
            failure_reason(&response)
        ));
    }

    let empty = Vec::new();
    let torrents = arguments(&response)
        .get("torrents")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let needle = query.to_lowercase();

    let matching: Vec<&Value> = torrents
        .iter()
        .filter(|torrent| {
            torrent
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name.to_lowercase().contains(&needle))
        })
        .filter(|torrent| filter.matches(torrent))
        .collect();

    if matching.is_empty() {
        return Ok("No torrents found matching the search criteria".to_owned());
    }

    let lines: Vec<String> = matching
        .iter()
        .map(|torrent| format::search_line(torrent))
        .collect();
    Ok(format!(
        "Found {} matching torrents:\n{}",
        matching.len(),
        lines.join("\n")
    ))
}

/// Session statistics report.
pub async fn get_session_stats(client: &TransmissionClient) -> Result<String, Error> {
    let response = client.request("session-stats", json!({})).await?;

    if succeeded(&response) {
        Ok(format::session_stats(arguments(&response)))
    } else {
        Ok(format!(
            "Failed to get session stats: {}",
            failure_reason(&response)
        ))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn advertised_enum_values_match_the_variants() {
        // The tool schema is written out by hand (see `string_enum`), so this
        // pins the advertised values to the enum that deserializes them: a
        // rename or reorder on either side breaks here rather than silently
        // changing the wire contract.
        assert_eq!(
            serde_json::to_value(Priority::NAMES).expect("names serialize"),
            serde_json::to_value([Priority::High, Priority::Normal, Priority::Low])
                .expect("variants serialize")
        );
        assert_eq!(
            serde_json::to_value(StatusFilter::NAMES).expect("names serialize"),
            serde_json::to_value([
                StatusFilter::All,
                StatusFilter::Downloading,
                StatusFilter::Seeding,
                StatusFilter::Paused,
                StatusFilter::Completed,
            ])
            .expect("variants serialize")
        );
    }

    #[test]
    fn renders_the_enum_schema_the_tools_advertised() {
        let schema =
            serde_json::to_value(string_enum(&Priority::NAMES)).expect("schema serializes");
        assert_eq!(
            schema,
            json!({ "type": "string", "enum": ["high", "normal", "low"] })
        );
    }

    #[test]
    fn filters_select_torrents_by_status_code() {
        let downloading = json!({ "status": 4 });
        let seeding = json!({ "status": 6 });

        assert!(StatusFilter::All.matches(&downloading));
        assert!(StatusFilter::Downloading.matches(&downloading));
        assert!(!StatusFilter::Downloading.matches(&seeding));
        assert!(StatusFilter::Seeding.matches(&seeding));
        // The original folded "completed" into seeding; that is preserved.
        assert!(StatusFilter::Completed.matches(&seeding));
        assert!(StatusFilter::Paused.matches(&json!({ "status": 0 })));
        assert!(
            !StatusFilter::Paused.matches(&json!({})),
            "a torrent without a status matches no filter but `all`"
        );
    }
}
