//! The MCP server: thin tool adapters plus the resource handlers.
//!
//! Each `#[tool]` method validates nothing and formats nothing — it hands the
//! arguments to the matching [`crate::tools`] function and wraps the result,
//! which keeps the behaviour testable without a transport.

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, ListResourcesResult, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router, ErrorData, RoleServer, ServerHandler};
use serde_json::{json, Value};

use crate::client::TransmissionClient;
use crate::error::Error;
use crate::tools::{
    self, AddTorrentArgs, RemoveTorrentArgs, SearchTorrentsArgs, SetPriorityArgs,
    SetSpeedLimitsArgs, TorrentIdArgs,
};
use crate::{SERVER_NAME, SERVER_VERSION};

/// URI of the session-information resource.
pub const RESOURCE_SESSION: &str = "transmission://session";
/// URI of the all-torrents resource.
pub const RESOURCE_TORRENTS: &str = "transmission://torrents";
/// URI of the session-statistics resource.
pub const RESOURCE_STATS: &str = "transmission://stats";

/// The Transmission MCP server.
#[derive(Clone)]
pub struct TransmissionMcp {
    client: Arc<TransmissionClient>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl TransmissionMcp {
    /// Build a server over an RPC client.
    #[must_use]
    pub fn new(client: Arc<TransmissionClient>) -> Self {
        Self {
            client,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "add_torrent",
        description = "Add a new torrent by URL or magnet link"
    )]
    async fn add_torrent(
        &self,
        Parameters(args): Parameters<AddTorrentArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(finish(
            "add_torrent",
            tools::add_torrent(&self.client, args).await,
        ))
    }

    #[tool(name = "remove_torrent", description = "Remove a torrent by ID")]
    async fn remove_torrent(
        &self,
        Parameters(args): Parameters<RemoveTorrentArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(finish(
            "remove_torrent",
            tools::remove_torrent(&self.client, args).await,
        ))
    }

    #[tool(name = "start_torrent", description = "Start/resume a torrent by ID")]
    async fn start_torrent(
        &self,
        Parameters(args): Parameters<TorrentIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(finish(
            "start_torrent",
            tools::start_torrent(&self.client, args).await,
        ))
    }

    #[tool(name = "stop_torrent", description = "Stop/pause a torrent by ID")]
    async fn stop_torrent(
        &self,
        Parameters(args): Parameters<TorrentIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(finish(
            "stop_torrent",
            tools::stop_torrent(&self.client, args).await,
        ))
    }

    #[tool(
        name = "get_torrent_info",
        description = "Get detailed information about a specific torrent"
    )]
    async fn get_torrent_info(
        &self,
        Parameters(args): Parameters<TorrentIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(finish(
            "get_torrent_info",
            tools::get_torrent_info(&self.client, args).await,
        ))
    }

    #[tool(
        name = "set_torrent_priority",
        description = "Set download priority for a torrent"
    )]
    async fn set_torrent_priority(
        &self,
        Parameters(args): Parameters<SetPriorityArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(finish(
            "set_torrent_priority",
            tools::set_torrent_priority(&self.client, args).await,
        ))
    }

    #[tool(
        name = "set_speed_limits",
        description = "Set global download/upload speed limits"
    )]
    async fn set_speed_limits(
        &self,
        Parameters(args): Parameters<SetSpeedLimitsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(finish(
            "set_speed_limits",
            tools::set_speed_limits(&self.client, args).await,
        ))
    }

    #[tool(name = "search_torrents", description = "Search torrents by name")]
    async fn search_torrents(
        &self,
        Parameters(args): Parameters<SearchTorrentsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(finish(
            "search_torrents",
            tools::search_torrents(&self.client, args).await,
        ))
    }

    #[tool(
        name = "get_session_stats",
        description = "Get Transmission session statistics"
    )]
    async fn get_session_stats(&self) -> Result<CallToolResult, ErrorData> {
        Ok(finish(
            "get_session_stats",
            tools::get_session_stats(&self.client).await,
        ))
    }

    /// Fetch and render one of the published resources.
    async fn read(&self, uri: &str) -> Result<String, Error> {
        let response = match uri {
            RESOURCE_SESSION => self.client.request("session-get", json!({})).await?,
            RESOURCE_TORRENTS => {
                self.client
                    .request(
                        "torrent-get",
                        json!({ "fields": tools::TORRENT_LIST_FIELDS }),
                    )
                    .await?
            }
            RESOURCE_STATS => self.client.request("session-stats", json!({})).await?,
            other => return Err(Error::Invalid(format!("Unknown resource: {other}"))),
        };
        pretty_arguments(&response)
    }
}

// The transport-side methods of `ServerHandler` are `async` by contract; the
// resource lister has nothing to await, and the macro-generated tool dispatcher
// is likewise synchronous inside an async signature.
#[expect(
    clippy::unused_async_trait_impl,
    reason = "ServerHandler's methods are async"
)]
#[tool_handler(router = self.tool_router)]
impl ServerHandler for TransmissionMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new(SERVER_NAME, SERVER_VERSION))
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(vec![
            Resource::new(RESOURCE_SESSION, "Transmission Session Info")
                .with_description("Current Transmission session information and statistics")
                .with_mime_type("application/json"),
            Resource::new(RESOURCE_TORRENTS, "All Torrents")
                .with_description("List of all torrents in Transmission")
                .with_mime_type("application/json"),
            Resource::new(RESOURCE_STATS, "Transmission Statistics")
                .with_description("Transmission session statistics")
                .with_mime_type("application/json"),
        ]))
    }

    /// Read a resource, reporting a failure as text rather than an error.
    ///
    /// A resource that cannot be fetched returns
    /// `Error reading resource <uri>: <message>` as ordinary content, so a
    /// client that reads a broken resource still sees why.
    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let uri = request.uri;
        let (text, mime_type) = match self.read(&uri).await {
            Ok(body) => (body, "application/json"),
            Err(error) => (
                format!("Error reading resource {uri}: {error}"),
                "text/plain",
            ),
        };
        let contents = ResourceContents::text(text, uri).with_mime_type(mime_type);
        Ok(ReadResourceResult::new(vec![contents]).into())
    }
}

/// Render a service result as the text-only tool result the tools return.
///
/// Both outcomes come back as successful content, with a failure described in
/// the text (`Error executing <tool>: <reason>`) rather than in `isError`; the
/// failure is also logged to stderr.
fn finish(tool: &str, result: Result<String, Error>) -> CallToolResult {
    let text = match result {
        Ok(text) => text,
        Err(error) => {
            eprintln!("Error in tool {tool}: {error}");
            format!("Error executing {tool}: {error}")
        }
    };
    CallToolResult::success(vec![ContentBlock::text(text)])
}

/// Pretty-print the `arguments` member of a response envelope.
///
/// This is what `json.dumps(result["arguments"], indent=2)` produced, including
/// an empty object when the member is absent.
fn pretty_arguments(response: &Value) -> Result<String, Error> {
    let empty = json!({});
    let arguments = response.get("arguments").unwrap_or(&empty);
    serde_json::to_string_pretty(arguments).map_err(|error| Error::Decode(error.to_string()))
}
