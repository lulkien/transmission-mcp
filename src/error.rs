//! Error taxonomy for the Transmission RPC client.
//!
//! Every variant's [`std::fmt::Display`] is caller-visible: a tool failure is
//! rendered as `Error executing <tool>: <message>`, which makes these strings
//! part of the server's observable contract.

use std::error::Error as StdError;

/// A failure while talking to Transmission, or while validating a tool call.
///
/// # Error kinds
///
/// [`Error::kind`] names the failure class for logging and tests;
/// `Display` is the caller-visible message and is part of the tool's
/// observable output, so it is treated as a stable contract.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The request never produced a response: DNS, connect, TLS or timeout.
    #[error("Network error: {0}")]
    Network(String),

    /// The endpoint answered with a non-success status code.
    #[error("HTTP error {status}: {body}")]
    Http {
        /// The HTTP status code returned.
        status: u16,
        /// The response body, verbatim.
        body: String,
    },

    /// The response body was not JSON.
    #[error("Invalid response: {0}")]
    Decode(String),

    /// The call could not be built from the supplied arguments.
    #[error("{0}")]
    Invalid(String),
}

impl Error {
    /// The failure class, for logging and tests.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Network(_) => "network",
            Self::Http { .. } => "http_status",
            Self::Decode(_) => "decode",
            Self::Invalid(_) => "invalid_argument",
        }
    }

    /// Wrap a `reqwest` failure, folding in its cause chain.
    ///
    /// The top-level `Display` of a `reqwest::Error` reads
    /// `error sending request for url (...)` for a DNS failure, a TLS failure
    /// and a refused proxy alike; only the cause chain tells them apart, so it
    /// is appended to the message.
    #[must_use]
    pub fn from_reqwest(error: &reqwest::Error) -> Self {
        let mut message = error.to_string();
        let mut cause: Option<&(dyn StdError + 'static)> = StdError::source(error);
        while let Some(source) = cause {
            message.push_str(": ");
            message.push_str(&source.to_string());
            cause = source.source();
        }
        Self::Network(message)
    }
}
