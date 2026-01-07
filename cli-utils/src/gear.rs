use clap::Args;
use url::Url;
use anyhow::{anyhow, Result};

#[derive(Args, Clone, Debug)]
pub struct GearConnectionArgs {
    /// Gear RPC endpoint URL. Overrides domain/port if set.
    #[arg(long = "gear-endpoint", alias = "gear-url", env = "GEAR_ENDPOINT")]
    pub endpoint: Option<Url>,

    /// Domain of the Gear RPC endpoint (used if endpoint is not set)
    #[arg(
        long = "gear-domain",
        default_value = "ws://127.0.0.1",
        env = "GEAR_DOMAIN"
    )]
    pub domain: String,

    /// Port of the Gear RPC endpoint (used if endpoint is not set)
    #[arg(long = "gear-port", default_value = "9944", env = "GEAR_PORT")]
    pub port: u16,

    /// Reconnect attempts for the Gear RPC.
    #[arg(
        long = "gear-max-reconnect-attempts",
        default_value = "3",
        env = "GEAR_MAX_RECONNECT_ATTEMPTS"
    )]
    pub max_reconnect_attempts: u8,
}

impl GearConnectionArgs {
    pub fn get_endpoint(&self) -> Result<String> {
        if let Some(url) = &self.endpoint {
            return Ok(url.to_string());
        }

        log::warn!("Usage of --gear-domain and --gear-port is deprecated. Please use --gear-endpoint instead.");
        
        let protocol = if self.port == 443 { "wss" } else { "ws" };
        // If domain already contains protocol, don't prepend
        if self.domain.contains("://") {
             Ok(format!("{}:{}", self.domain, self.port))
        } else {
             Ok(format!("{}://{}:{}", protocol, self.domain, self.port))
        }
    }

    pub fn get_host_port(&self) -> Result<(String, u16)> {
        if let Some(url) = &self.endpoint {
            let host = url.host_str().ok_or_else(|| anyhow!("No host in URL"))?;
            let scheme = url.scheme();
            let address = format!("{}://{}", scheme, host);

            let port = url.port().or_else(|| match url.scheme() {
                "wss" | "https" => Some(443),
                "ws" | "http" => Some(80),
                _ => None
            }).unwrap_or(9944);
            Ok((address, port))
        } else {
            log::warn!("Usage of --gear-domain and --gear-port is deprecated. Please use --gear-endpoint instead.");
            // Return domain as is (assuming it might contain scheme) and port
            Ok((self.domain.clone(), self.port))
        }
    }
}

#[derive(Args, Clone, Debug)]
pub struct GearSignerArgs {
    #[clap(flatten)]
    pub connection: GearConnectionArgs,

    /// Substrate URI that identifies a user by a mnemonic phrase or
    /// provides default users from the keyring (e.g., "//Alice", "//Bob",
    /// etc.).
    #[arg(long = "gear-suri", env = "GEAR_SURI")]
    pub suri: String,
}
