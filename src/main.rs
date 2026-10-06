//! Entry point: read the configuration, then serve MCP over stdio.
//!
//! Stdout carries the protocol, so every diagnostic goes to stderr.

use std::process::ExitCode;
use std::sync::Arc;

use rmcp::transport::stdio;
use rmcp::ServiceExt;

use transmission_mcp::client::TransmissionClient;
use transmission_mcp::config::Config;
use transmission_mcp::server::TransmissionMcp;

#[tokio::main]
async fn main() -> ExitCode {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("transmission-mcp: {error}");
            return ExitCode::FAILURE;
        }
    };

    let client = match TransmissionClient::new(&config) {
        Ok(client) => Arc::new(client),
        Err(error) => {
            eprintln!("transmission-mcp: {error}");
            return ExitCode::FAILURE;
        }
    };

    eprintln!(
        "transmission-mcp: rpc endpoint {} (authentication {})",
        client.url(),
        if client.authenticated() {
            "enabled"
        } else {
            "disabled"
        }
    );

    let server = TransmissionMcp::new(client);
    match server.serve(stdio()).await {
        Ok(running) => match running.waiting().await {
            Ok(_) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("transmission-mcp: transport error: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("transmission-mcp: failed to start: {error}");
            ExitCode::FAILURE
        }
    }
}
