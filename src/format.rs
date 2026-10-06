//! Rendering of Transmission RPC payloads into the text a caller reads.
//!
//! The strings produced here are the tools' observable output, so they are
//! treated as a stable contract.

use std::fmt::Write as _;

use serde_json::Value;

/// Human-readable name for a Transmission torrent status code.
///
/// A missing `status` field reads as `Stopped`, the default of `0`; a status
/// that is present but not an integer reads as `Unknown`.
#[must_use]
pub fn status_name(torrent: &Value) -> &'static str {
    match torrent.get("status") {
        None => status_from_code(0),
        Some(value) => value.as_i64().map_or("Unknown", status_from_code),
    }
}

/// Map a raw status code to its name.
fn status_from_code(code: i64) -> &'static str {
    match code {
        0 => "Stopped",
        1 => "Check queued",
        2 => "Checking",
        3 => "Download queued",
        4 => "Downloading",
        5 => "Seed queued",
        6 => "Seeding",
        _ => "Unknown",
    }
}

/// Render a JSON value the way a human-readable message would.
///
/// Strings appear unquoted, `null` prints as `None`, and numbers keep their own
/// formatting — no added quoting or type decoration.
#[must_use]
pub fn json_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".to_owned(),
        other => other.to_string(),
    }
}

/// Read a numeric field, defaulting to `0` when absent.
fn number(object: &Value, key: &str) -> f64 {
    object.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

/// Render a numeric field, defaulting to `0` when absent.
fn integer(object: &Value, key: &str) -> String {
    match object.get(key) {
        None => "0".to_owned(),
        Some(Value::Number(number)) => number
            .as_i64()
            .map_or_else(|| number.to_string(), |value| value.to_string()),
        Some(other) => json_text(other),
    }
}

/// Truthiness for the JSON values Transmission sends: null, zero, an empty
/// string, and an empty collection are all false.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_none_or(|value| value != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
    }
}

/// Time remaining for a torrent, or `Unknown` when Transmission reports `-1`.
fn eta(torrent: &Value) -> String {
    match torrent.get("eta").and_then(Value::as_i64) {
        Some(value) if value != -1 => integer(torrent, "eta"),
        _ => "Unknown".to_owned(),
    }
}

/// The multi-line report returned by `get_torrent_info`.
#[must_use]
pub fn torrent_info(torrent: &Value) -> String {
    let name = torrent.get("name").and_then(Value::as_str).unwrap_or("N/A");
    let id = torrent
        .get("id")
        .map_or_else(|| "N/A".to_owned(), json_text);
    let download_dir = torrent
        .get("downloadDir")
        .and_then(Value::as_str)
        .unwrap_or("N/A");

    let mut out = String::new();
    let _ = writeln!(out, "Torrent Information:");
    let _ = writeln!(out, "Name: {name}");
    let _ = writeln!(out, "ID: {id}");
    let _ = writeln!(out, "Status: {}", status_name(torrent));
    let _ = writeln!(
        out,
        "Size: {:.2} MB",
        number(torrent, "totalSize") / 1024.0 / 1024.0
    );
    let _ = writeln!(
        out,
        "Progress: {:.1}%",
        number(torrent, "percentDone") * 100.0
    );
    let _ = writeln!(
        out,
        "Download Rate: {:.1} KB/s",
        number(torrent, "rateDownload") / 1024.0
    );
    let _ = writeln!(
        out,
        "Upload Rate: {:.1} KB/s",
        number(torrent, "rateUpload") / 1024.0
    );
    let _ = writeln!(out, "Ratio: {:.2}", number(torrent, "uploadRatio"));
    let _ = writeln!(out, "ETA: {} seconds", eta(torrent));
    let _ = writeln!(out, "Peers: {}", integer(torrent, "peersConnected"));
    let _ = writeln!(out, "Download Dir: {download_dir}");

    if torrent.get("error").is_some_and(truthy) {
        let detail = torrent
            .get("errorString")
            .and_then(Value::as_str)
            .unwrap_or("N/A");
        let _ = writeln!(out, "Error: {detail}");
    }
    out
}

/// One line of `search_torrents` output.
#[must_use]
pub fn search_line(torrent: &Value) -> String {
    format!(
        "ID: {} | {} | Status: {} | Progress: {:.1}%",
        torrent
            .get("id")
            .map_or_else(|| "None".to_owned(), json_text),
        torrent
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("None"),
        status_name(torrent),
        number(torrent, "percentDone") * 100.0
    )
}

/// The multi-line report returned by `get_session_stats`.
#[must_use]
pub fn session_stats(stats: &Value) -> String {
    let empty = Value::Null;
    let current = stats.get("current-stats").unwrap_or(&empty);
    let cumulative = stats.get("cumulative-stats").unwrap_or(&empty);

    let mut out = String::new();
    let _ = writeln!(out, "Transmission Session Statistics:\n");
    let _ = writeln!(out, "Current Session:");
    let _ = writeln!(
        out,
        "- Download Speed: {:.1} KB/s",
        number(current, "downloadSpeed") / 1024.0
    );
    let _ = writeln!(
        out,
        "- Upload Speed: {:.1} KB/s",
        number(current, "uploadSpeed") / 1024.0
    );
    let _ = writeln!(
        out,
        "- Downloaded: {:.2} MB",
        number(current, "downloadedBytes") / 1024.0 / 1024.0
    );
    let _ = writeln!(
        out,
        "- Uploaded: {:.2} MB",
        number(current, "uploadedBytes") / 1024.0 / 1024.0
    );
    let _ = writeln!(out, "- Files Added: {}", integer(current, "filesAdded"));
    let _ = writeln!(
        out,
        "- Active Torrents: {}",
        integer(stats, "activeTorrentCount")
    );
    let _ = writeln!(
        out,
        "- Paused Torrents: {}",
        integer(stats, "pausedTorrentCount")
    );
    let _ = writeln!(out, "- Total Torrents: {}", integer(stats, "torrentCount"));
    let _ = writeln!(out, "\nCumulative:");
    let _ = writeln!(
        out,
        "- Downloaded: {:.2} GB",
        number(cumulative, "downloadedBytes") / 1024.0 / 1024.0 / 1024.0
    );
    let _ = writeln!(
        out,
        "- Uploaded: {:.2} GB",
        number(cumulative, "uploadedBytes") / 1024.0 / 1024.0 / 1024.0
    );
    let _ = writeln!(out, "- Files Added: {}", integer(cumulative, "filesAdded"));
    let _ = writeln!(out, "- Sessions: {}", integer(cumulative, "sessionCount"));
    let _ = writeln!(
        out,
        "- Uptime: {:.1} hours",
        number(cumulative, "secondsActive") / 3600.0
    );
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn names_every_known_status_code() {
        for (code, name) in [
            (0, "Stopped"),
            (1, "Check queued"),
            (2, "Checking"),
            (3, "Download queued"),
            (4, "Downloading"),
            (5, "Seed queued"),
            (6, "Seeding"),
            (99, "Unknown"),
        ] {
            assert_eq!(status_from_code(code), name);
        }
    }

    #[test]
    fn defaults_a_missing_status_to_stopped() {
        assert_eq!(status_name(&json!({})), "Stopped");
        assert_eq!(status_name(&json!({ "status": 4 })), "Downloading");
        assert_eq!(
            status_name(&json!({ "status": null })),
            "Unknown",
            "a status that is present but not a number has no name"
        );
    }

    #[test]
    fn renders_absent_fields_as_the_original_defaults() {
        let report = torrent_info(&json!({ "id": 1 }));

        assert_eq!(
            report,
            "Torrent Information:\n\
             Name: N/A\n\
             ID: 1\n\
             Status: Stopped\n\
             Size: 0.00 MB\n\
             Progress: 0.0%\n\
             Download Rate: 0.0 KB/s\n\
             Upload Rate: 0.0 KB/s\n\
             Ratio: 0.00\n\
             ETA: Unknown seconds\n\
             Peers: 0\n\
             Download Dir: N/A\n"
        );
    }

    #[test]
    fn adds_the_error_line_only_when_an_error_code_is_set() {
        let healthy = torrent_info(&json!({ "error": 0, "errorString": "" }));
        assert!(!healthy.contains("Error:"));

        let broken = torrent_info(&json!({ "error": 3, "errorString": "No data found" }));
        assert!(broken.ends_with("Error: No data found\n"), "got: {broken}");

        let unlabelled = torrent_info(&json!({ "error": 3 }));
        assert!(
            unlabelled.ends_with("Error: N/A\n"),
            "an error without a string still reports one: {unlabelled}"
        );
    }

    #[test]
    fn treats_a_minus_one_eta_as_unknown_but_keeps_a_real_one() {
        assert_eq!(eta(&json!({ "eta": -1 })), "Unknown");
        assert_eq!(eta(&json!({})), "Unknown");
        assert_eq!(eta(&json!({ "eta": 0 })), "0");
        assert_eq!(eta(&json!({ "eta": 3600 })), "3600");
    }

    #[test]
    fn renders_a_search_line() {
        assert_eq!(
            search_line(&json!({ "id": 7, "name": "Debian ISO", "status": 6, "percentDone": 1.0 })),
            "ID: 7 | Debian ISO | Status: Seeding | Progress: 100.0%"
        );
        assert_eq!(
            search_line(&json!({})),
            "ID: None | None | Status: Stopped | Progress: 0.0%"
        );
    }

    #[test]
    fn renders_session_statistics_with_absent_sections() {
        let report = session_stats(&json!({}));

        assert_eq!(
            report,
            "Transmission Session Statistics:\n\
             \n\
             Current Session:\n\
             - Download Speed: 0.0 KB/s\n\
             - Upload Speed: 0.0 KB/s\n\
             - Downloaded: 0.00 MB\n\
             - Uploaded: 0.00 MB\n\
             - Files Added: 0\n\
             - Active Torrents: 0\n\
             - Paused Torrents: 0\n\
             - Total Torrents: 0\n\
             \n\
             Cumulative:\n\
             - Downloaded: 0.00 GB\n\
             - Uploaded: 0.00 GB\n\
             - Files Added: 0\n\
             - Sessions: 0\n\
             - Uptime: 0.0 hours\n"
        );
    }

    #[test]
    fn renders_json_values_without_type_decoration() {
        assert_eq!(json_text(&json!("plain")), "plain");
        assert_eq!(json_text(&json!(null)), "None");
        assert_eq!(json_text(&json!(7)), "7");
        assert_eq!(json_text(&json!(true)), "true");
    }
}
