use anyhow::{anyhow, Result};
use clap::Args;
use url::Url;

#[derive(Args, Clone, Debug)]
pub struct GearConnectionArgs {
    /// Gear RPC endpoint URL
    #[arg(
        long = "gear-endpoint",
        alias = "gear-url",
        env = "GEAR_ENDPOINT",
        default_value = "ws://127.0.0.1:9944"
    )]
    pub endpoint: Url,

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
        Ok(self.endpoint.to_string())
    }

    pub fn get_host_port(&self) -> Result<(String, u16)> {
        let url = &self.endpoint;
        let host = url.host_str().ok_or_else(|| anyhow!("No host in URL"))?;
        let scheme = url.scheme();
        let address = format!("{scheme}://{host}");

        let port = url
            .port()
            .or_else(|| match url.scheme() {
                "wss" | "https" => Some(443),
                "ws" | "http" => Some(80),
                _ => None,
            })
            .unwrap_or(9944);
        Ok((address, port))
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
