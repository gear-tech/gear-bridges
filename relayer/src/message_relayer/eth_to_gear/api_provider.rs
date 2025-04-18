use anyhow::Context;
use gear_rpc_client::GearApi;
use gsdk::Api;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{mpsc, oneshot};

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
    session: Option<u64>,
}

impl ApiProviderConnection {
    fn new(sender: UnboundedSender<ApiConnectionRequest>) -> Self {
        Self {
            sender,
            session: None,
        }
    }

    /// Request a new API connection from the [`ApiProvider`]. This will return a new
    /// [`GearApi`] instance which can be used to interact with the API.
    pub async fn request_connection(&mut self) -> anyhow::Result<GearApi> {
        let (tx, rx) = oneshot::channel();
        // set session to 0 if it's None, this will allow us to reuse any existing connection
        // if there is one.
        let session = self.session.unwrap_or(0);
        let request = ApiConnectionRequest {
            session,
            receiver: tx,
        };
        self.sender
            .send(request)
            .context("failed to send API connection request")?;
        let response = rx
            .await
            .context("failed to receive API connection response")?;
        self.session = Some(response.session);
        Ok(GearApi::from(response.api))
    }

    /// Returns the session number of the connection.
    ///
    /// Returns `None` if the connection has not been established yet.
    pub fn session(&self) -> Option<u64> {
        self.session
    }
}

impl Clone for ApiProviderConnection {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            session: None,
        }
    }
}

/// A service which provides API connections to services which request it.
pub struct ApiProvider {
    session: u64,
    doman: String,
    port: u16,
    retries: u8,
    api: Api,

    receiver: UnboundedReceiver<ApiConnectionRequest>,
}

impl ApiProvider {
    pub async fn new(
        domain: String,
        port: u16,
        retries: u8,
    ) -> anyhow::Result<(Self, ApiProviderConnection)> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let uri: &str = &format!("{domain}:{port}");
        let api = Api::builder()
            .retries(retries)
            .build(uri)
            .await
            .context("failed to connect to API")?;
        let conn = ApiProviderConnection::new(sender.clone());
        Ok((
            Self {
                session: 0,
                doman: domain,
                port,
                retries,
                api,
                receiver,
            },
            conn,
        ))
    }

    pub async fn spawn(mut self) {
        tokio::spawn(async move {
            while let Ok(request) = self.receiver.try_recv() {
                if request.session < self.session {
                    let response = ApiConnectionResponse {
                        session: request.session,
                        api: self.api.clone(),
                    };
                    match request.receiver.send(response) {
                        Ok(_) => {
                            log::info!("Session #{} reuses API connection", request.session);
                        }
                        Err(_) => {
                            log::error!("Failed to send response for session #{}", request.session);
                        }
                    }
                    continue;
                }
                let uri: &str = &format!("{}:{}", self.doman, self.port);
                self.api = match Api::builder().retries(self.retries).build(uri).await {
                    Ok(api) => api,
                    Err(err) => {
                        log::error!("Failed to create API connection: {}", err);
                        return Err(err);
                    }
                };
                self.session += 1;
                log::info!(
                    "established new API connection with session number {}",
                    self.session
                );
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

                let response = ApiConnectionResponse {
                    session: self.session,
                    api: self.api.clone(),
                };
                match request.receiver.send(response) {
                    Ok(_) => {
                        log::info!("Session #{} got new API connection", self.session);
                    }
                    Err(_) => {
                        log::error!("Failed to send response for session #{}", self.session);
                    }
                }
            }

            Ok(())
        });
    }
}
