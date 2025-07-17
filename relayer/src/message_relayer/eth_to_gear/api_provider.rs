use anyhow::Context;
use gear_rpc_client::GearApi;
use gsdk::Api;
use std::time::Duration;
use tokio::sync::{
    mpsc,
    mpsc::{UnboundedReceiver, UnboundedSender},
    oneshot,
};

/// Max reconnection attempts before failing. Default: 10.
const MAX_RECONNECT_ATTEMPTS: usize = 10;
/// Timeout between reconnects. Default: 1m30s.
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(90);

struct ApiConnectionRequest {
    session: u64,
    receiver: oneshot::Sender<ApiConnectionResponse>,
}

struct ApiConnectionResponse {
    session: u64,
    api: Api,
}

/// A connection to the [`ApiProvider`] which can be used to request an API connection.
///
/// Each connection has a session number which is used to identify the connection. When
/// a connection is cloned the session number is reset to `None`.
pub struct ApiProviderConnection {
    sender: UnboundedSender<ApiConnectionRequest>,
    session: u64,
    api: Api,
}

impl ApiProviderConnection {
    /// Explicit reconnect reqest to the [`ApiProvider`]. This will
    /// update current connection to include latest session number
    /// and API connection.
    ///
    /// When this function errors it indicates that the provider
    /// has failed and no further connections can be made.
    pub async fn reconnect(&mut self) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        let request = ApiConnectionRequest {
            session: self.session,
            receiver: tx,
        };
        self.sender
            .send(request)
            .context("failed to send API connection request")?;

        let response = rx
            .await
            .context("failed to receive API connection response")?;
        self.session = response.session;
        self.api = response.api;

        Ok(())
    }

    /// Request a new API connection from the [`ApiProvider`]. This will return a new
    /// [`GearApi`](gclient::GearApi) instance with specified suri.
    pub fn gclient_client(&mut self, suri: &str) -> anyhow::Result<gclient::GearApi> {
        gclient::GearApi::from(self.api.clone())
            .with(suri)
            .map_err(|e| anyhow::anyhow!("failed to set suri: {}", e))
    }

    /// Request a new API connection from the [`ApiProvider`].
    pub fn gclient(&mut self) -> gclient::GearApi {
        gclient::GearApi::from(self.api.clone())
    }

    pub fn client(&self) -> GearApi {
        GearApi::from(self.api.clone())
    }
}

impl Clone for ApiProviderConnection {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            session: self.session,
            api: self.api.clone(),
        }
    }
}

/// A service which provides API connections to services which request it.
pub struct ApiProvider {
    session: u64,
    domain: String,
    port: u16,
    retries: u8,
    api: Api,

    receiver: UnboundedReceiver<ApiConnectionRequest>,
    sender: UnboundedSender<ApiConnectionRequest>,
}

impl ApiProvider {
    pub async fn new(domain: String, port: u16, retries: u8) -> anyhow::Result<Self> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let uri: &str = &format!("{domain}:{port}");
        let api = Api::builder()
            .retries(retries)
            .build(uri)
            .await
            .context("failed to connect to API")?;

        Ok(Self {
            session: 0,
            domain,
            port,
            retries,
            api,
            receiver,
            sender,
        })
    }

    pub fn connection(&self) -> ApiProviderConnection {
        ApiProviderConnection {
            sender: self.sender.clone(),
            session: self.session,
            api: self.api.clone(),
        }
    }

    async fn reconnect(&mut self) -> bool {
        let uri: &str = &format!("{}:{}", self.domain, self.port);

        for attempt in 0..MAX_RECONNECT_ATTEMPTS {
            match Api::builder().retries(self.retries).build(uri).await {
                Ok(api) => {
                    self.api = api;
                    return true;
                }
                Err(err) => {
                    log::error!(
                        "Failed to create API connection (attempt {attempt}/{MAX_RECONNECT_ATTEMPTS}): {err}"
                    );

                    tokio::time::sleep(RECONNECT_TIMEOUT).await;
                }
            }
        }

        log::error!("All {MAX_RECONNECT_ATTEMPTS} attempts to connect to API failed. Giving up.");
        false
    }

    pub fn spawn(mut self) {
        tokio::spawn(async move {
            while let Some(request) = self.receiver.recv().await {
                log::info!(
                    "Current session #{}, request session #{}",
                    self.session,
                    request.session
                );

                if request.session < self.session {
                    let response = ApiConnectionResponse {
                        session: self.session,
                        api: self.api.clone(),
                    };
                    match request.receiver.send(response) {
                        Ok(_) => {
                            log::info!("Session reuses API connection");
                        }
                        Err(_) => {
                            log::error!("Failed to send response for the session");
                        }
                    }
                    continue;
                }

                // TODO: Implement a backoff strategy for the connection
                let rem = self.session % 10;
                let sleep_time = if rem < 3 {
                    1
                } else if rem < 5 {
                    10
                } else {
                    30
                };

                tokio::time::sleep(Duration::from_secs(sleep_time)).await;
                if !self.reconnect().await {
                    return;
                }
                self.session += 1;
                log::info!(
                    "Established new API connection with session number {}",
                    self.session
                );

                let response = ApiConnectionResponse {
                    session: self.session,
                    api: self.api.clone(),
                };
                match request.receiver.send(response) {
                    Ok(_) => {
                        log::info!("Session got new API connection");
                    }
                    Err(_) => {
                        log::error!("Failed to send response for the session");
                    }
                }
            }
        });
    }
}
