# Transmission MCP Server

A Model Context Protocol server that exposes a Transmission daemon through its
RPC API: torrent management, configuration and monitoring as MCP tools, plus
read-only session, torrent and statistics resources.

Built on [`rmcp`](https://crates.io/crates/rmcp) as a single binary speaking MCP
over stdio.

## Building

```bash
cargo build --release           # -> target/release/transmission-mcp
cargo install --path . --locked # -> ~/.cargo/bin/transmission-mcp
```

Rust 1.88 or newer.

## Configuration

All configuration comes from the environment, so a stdio MCP client can set it in
its server entry. There is no config file.

| Variable | Default | Meaning |
| --- | --- | --- |
| `TRANSMISSION_HOST` | `localhost` | Hostname or IP of the Transmission daemon |
| `TRANSMISSION_PORT` | `9091` | Port of the Transmission web/RPC server |
| `TRANSMISSION_USERNAME` | *(unset)* | RPC username, when authentication is enabled |
| `TRANSMISSION_PASSWORD` | *(unset)* | RPC password, when authentication is enabled |

An empty username or password means authentication is off. The endpoint is always
`http://<host>:<port>/transmission/rpc`. The server performs the `409 Conflict`
CSRF handshake itself, caching the session id and refreshing it when the daemon
restarts.

There is nothing else to configure — no config file, no launcher script. An MCP
client supplies the variables in its own server entry:

```json
"transmission": {
  "command": "/path/to/transmission-mcp",
  "args": [],
  "env": {
    "TRANSMISSION_HOST": "localhost",
    "TRANSMISSION_PORT": "9091",
    "TRANSMISSION_USERNAME": "",
    "TRANSMISSION_PASSWORD": ""
  }
}
```

To run it by hand, set the same variables inline: `TRANSMISSION_HOST=box
TRANSMISSION_PORT=9091 ./target/release/transmission-mcp`. Diagnostics go to
stderr — stdout carries the protocol.

## Tools

| Tool | Arguments | Effect |
| --- | --- | --- |
| `add_torrent` | `url` (required), `download_dir`, `paused` | Add by magnet link or http(s) URL; anything else is treated as base64 `.torrent` metainfo |
| `remove_torrent` | `torrent_id` (required), `delete_local_data` | Remove a torrent, optionally with its files |
| `start_torrent` | `torrent_id` (required) | Resume |
| `stop_torrent` | `torrent_id` (required) | Pause |
| `get_torrent_info` | `torrent_id` (required) | Detailed report for one torrent |
| `set_torrent_priority` | `torrent_id`, `priority` (`high`/`normal`/`low`) | Set bandwidth priority |
| `set_speed_limits` | `download_limit`, `upload_limit` (KB/s, 0 = unlimited) | Set global limits |
| `search_torrents` | `query` (required), `status_filter` (`all`/`downloading`/`seeding`/`paused`/`completed`) | Search by name |
| `get_session_stats` | *(none)* | Session statistics |

## Resources

| URI | Content |
| --- | --- |
| `transmission://session` | `session-get` arguments, pretty-printed JSON |
| `transmission://torrents` | `torrent-get` for every torrent |
| `transmission://stats` | `session-stats` arguments |

A resource that cannot be fetched returns its error as resource text rather than
failing the read, so a client still sees why.

## Registering with a client

Use an **absolute path** to the binary: a stdio MCP subprocess inherits a filtered
environment, so a relative path may not resolve. The variables go in the client's
`env` block, and `mcp_config.json` is a ready-made example.

```bash
cargo install --path . --locked                     # -> ~/.cargo/bin/transmission-mcp
hermes mcp add transmission --command "$HOME/.cargo/bin/transmission-mcp"
hermes mcp list
hermes mcp test transmission
```

`hermes mcp add` takes `env` from the environment at add time, so export the
variables first if the daemon is not on `localhost:9091`. To attach or change
them afterwards, edit the server entry in `~/.hermes/config.yaml`:

```yaml
transmission:
  command: /home/you/.cargo/bin/transmission-mcp
  env:
    TRANSMISSION_HOST: localhost
    TRANSMISSION_PORT: '9091'
    TRANSMISSION_USERNAME: ''
    TRANSMISSION_PASSWORD: ''
```

Tools become callable at the next client session, not in the one that registers
the server.

## Behaviour notes

- **A tool failure is text, not an error.** A call that cannot complete still
  returns successful content whose message reads `Error executing <tool>:
  <reason>`, `Failed to <action>: <rpc result>` or `Torrent <id> not found`.
  `isError` is never set, so branch on the text, not the flag.
- **Optional numeric arguments are advertised as nullable.** `download_limit` is
  `["integer", "null"]`, the generator's way of expressing "optional". Passing
  `null` is treated as omitting the argument.
- **Schemas carry `"$schema"` and `"format": "int64"`** — valid JSON Schema
  keywords emitted alongside the properties, enums, defaults and `required`
  lists.
- **An unrecognised `priority` or `status_filter` fails validation** rather than
  silently falling back to a default.
- **`search_torrents`' `completed` filter selects status 6 (seeding).**
  Transmission has no separate "completed" status, so a finished-and-stopped
  torrent does not match it.
- **`set_speed_limits` sets only what you pass.** Omit a limit to leave it
  untouched; pass `0` to remove it.
- **A torrent whose `status` is not an integer reads as `Unknown`.**
- **Network failures report the cause chain** (`Network error: … : connection
  refused`), because a bare client error reads the same for DNS, TLS and
  connection failures.

## Testing

```bash
cargo test --all-targets    # unit + client + service + protocol tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

The integration tests spawn the real binary over stdio and point it at a scripted
RPC stub, so the tool surface, the RPC payloads and the exact output text are all
asserted without needing a daemon. No test requires network access.

## Security

- Transmission's RPC is plain HTTP. Keep the daemon on a trusted network (a
  VPN or a private overlay such as Tailscale), or tunnel it.
- Credentials are read from the environment and sent as HTTP basic auth; they
  are never logged. Put them in the client's `env` block, not in a file in the
  repository.
- Every tool can mutate the daemon. There is no read-only mode.
