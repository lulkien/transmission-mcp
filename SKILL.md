---
name: transmission
description: Use when managing torrents via the transmission MCP server. Routing table, procedures and pitfalls for driving its nine tools and three resources.
metadata:
  hermes:
    tags: [mcp, transmission, bittorrent, torrents]
---

# Driving the transmission MCP server

The server exposes a Transmission daemon's RPC API as MCP tools. This guide is
the part the tool schemas cannot carry: which tool to reach for, in what order,
and what the results actually mean.

## Routing

| Intent | Tool | Notes |
| --- | --- | --- |
| What is the daemon doing? | `get_session_stats` | No arguments. Counts, totals, cumulative traffic. |
| List or find torrents | `search_torrents` | `query` is a case-insensitive **substring** of the name. An empty query matches everything. |
| Details on one torrent | `get_torrent_info` | Needs the integer `torrent_id`. |
| Add | `add_torrent` | Magnet link, http(s) URL, or base64 metainfo. |
| Stop / resume | `stop_torrent` / `start_torrent` | Needs `torrent_id`. |
| Delete | `remove_torrent` | `delete_local_data: true` also deletes the files — irreversible. |
| Reorder bandwidth | `set_torrent_priority` | `high` / `normal` / `low`. |
| Throttle | `set_speed_limits` | KB/s; `0` means unlimited. |

Resources, for a whole-picture read without parsing tool text: `transmission://session`,
`transmission://torrents`, `transmission://stats` — raw RPC JSON.

## Procedures

**Act on a specific torrent.** You need its numeric id, and ids are not guessable
from a name.

1. `search_torrents` with a distinctive substring of the name.
2. Take the `ID:` from the matching line. If two results are plausible, say which
   you picked and why rather than picking silently.
3. Call the action tool with that integer id.
4. Done when the tool's text confirms the action against that same id.

**Add a torrent.**

1. `add_torrent` with the magnet link or URL. Pass `paused: true` if the user has
   not said to start it, and `download_dir` when they named a destination.
2. A duplicate is not a failure — the text reads `Torrent already exists: ...`.
   Report the existing id instead of adding again.
3. Done when the text names the new (or existing) torrent id.

**Rate-limit the daemon.**

1. `set_speed_limits` with `download_limit` and/or `upload_limit` in KB/s.
2. Done when the text reads `Speed limits updated successfully`.

**A torrent is not downloading.** Read `get_torrent_info` first, then judge:

- `Status: Stopped` — it is paused; `start_torrent` if that was not intended.
- `Status: Downloading` with `Peers: 0` — a tracker or connectivity problem, not
  a torrent problem. Say so instead of restarting it blindly.
- `ETA: Unknown` plus 0 peers — no estimate is possible; do not report the
  original `-1` as a number of seconds.

## What the results mean

- **A tool failure is still `isError: false`.** This server returns failures as
  ordinary text, so read the message: `Error executing <tool>: <reason>`,
  `Failed to <action>: <rpc result>`, or `Torrent <id> not found`. A check on the
  error flag alone will miss every one of them.
- **`Error executing add_torrent: 'url'` means the argument was missing**, not
  that the daemon rejected anything — no RPC call was made.
- **`status_filter: completed` selects status 6, i.e. seeding.** Transmission has
  no separate "completed" status, so a finished-but-stopped torrent will not show
  up under it; search by name and check `Progress: 100.0%` instead.
- **A `search_torrents` header of `Found N matching torrents:` with N lines** —
  if you report a count, use that number, not your own estimate.
- **Line formats are stable.** In search results, `ID: 7 | name | Status: ... |
  Progress: ...`; in the detailed report, one `Field: value` per line.

## Pitfalls

- **`torrent_id` is an integer.** Passing the torrent's name fails validation
  without reaching the daemon.
- **`add_torrent`'s URL heuristic is positional, not validating.** Anything that
  does not start with `magnet:` or `http` is sent as base64 metainfo, so a local
  file path is sent as if it were torrent data and fails at the daemon.
- **`remove_torrent` with `delete_local_data: true` deletes files.** Confirm with
  the user before using it.
- **`set_speed_limits` sets only what you pass.** Omitting a limit leaves it
  untouched — it does not reset it to unlimited. Pass `0` to remove a limit.
- **Every tool can mutate the daemon.** There is no dry-run or read-only mode.
