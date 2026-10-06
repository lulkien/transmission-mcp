//! Server configuration, read from the environment.
//!
//! `TRANSMISSION_HOST`, `TRANSMISSION_PORT`, `TRANSMISSION_USERNAME` and
//! `TRANSMISSION_PASSWORD`. Empty credentials mean "no authentication", the
//! same as an unset variable.

use crate::error::Error;

/// Host used when `TRANSMISSION_HOST` is unset.
pub const DEFAULT_HOST: &str = "localhost";

/// Port used when `TRANSMISSION_PORT` is unset.
pub const DEFAULT_PORT: u16 = 9091;

/// Path of the RPC endpoint on the Transmission web server.
pub const RPC_PATH: &str = "/transmission/rpc";

/// Connection settings for one Transmission RPC endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Hostname or IP of the Transmission daemon.
    pub host: String,
    /// TCP port of the Transmission web/RPC server.
    pub port: u16,
    /// RPC username, if authentication is enabled.
    pub username: Option<String>,
    /// RPC password, if authentication is enabled.
    pub password: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: DEFAULT_HOST.to_owned(),
            port: DEFAULT_PORT,
            username: None,
            password: None,
        }
    }
}

impl Config {
    /// Read the configuration from the process environment.
    ///
    /// A `TRANSMISSION_PORT` that is not a valid port is an error rather than a
    /// silent fallback, which would point the server at the wrong daemon.
    pub fn from_env() -> Result<Self, Error> {
        let host = std::env::var("TRANSMISSION_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_owned());
        let raw_port =
            std::env::var("TRANSMISSION_PORT").unwrap_or_else(|_| DEFAULT_PORT.to_string());
        let port = raw_port.parse::<u16>().map_err(|_| {
            Error::Invalid(format!("TRANSMISSION_PORT is not a valid port: {raw_port}"))
        })?;

        Ok(Self {
            host,
            port,
            username: std::env::var("TRANSMISSION_USERNAME").ok(),
            password: std::env::var("TRANSMISSION_PASSWORD").ok(),
        })
    }

    /// The full URL of the RPC endpoint.
    #[must_use]
    pub fn rpc_url(&self) -> String {
        format!("http://{}:{}{RPC_PATH}", self.host, self.port)
    }

    /// Credentials for HTTP basic auth, or `None` when auth is disabled.
    ///
    /// An empty username or password means authentication is off.
    #[must_use]
    pub fn basic_auth(&self) -> Option<(&str, &str)> {
        let username = self.username.as_deref().filter(|value| !value.is_empty())?;
        let password = self.password.as_deref().filter(|value| !value.is_empty())?;
        Some((username, password))
    }
}
