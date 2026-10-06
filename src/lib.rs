//! A Model Context Protocol server for the Transmission torrent client.
//!
//! Talks to a `transmission-daemon` (or Transmission Desktop) over its RPC API
//! and exposes torrent management, configuration and monitoring as MCP tools,
//! plus read-only session/torrent/statistics resources.
//!
//! The crate is split so the logic is testable without the transport: [`client`]
//! speaks the RPC protocol, [`format`] renders payloads, [`tools`] holds the
//! service functions each tool is a thin adapter over, and [`server`] wires them
//! to the MCP router.

pub mod client;
pub mod config;
pub mod error;
pub mod format;
pub mod server;
pub mod tools;

/// Name reported to MCP clients during `initialize`.
pub const SERVER_NAME: &str = "transmission-mcp";

/// Version reported to MCP clients during `initialize`.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
