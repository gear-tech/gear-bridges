use anyhow::Context;
use gear_rpc_client::GearApi;
use gsdk::Api;
use std::time::Duration;
use tokio::sync::{
    mpsc,
    mpsc::{UnboundedReceiver, UnboundedSender},
    oneshot,
};

/// Reconnect backoff starts quickly so transient disconnects do not stall
/// the whole relayer, but still caps at the historical 1m30s interval.
const RECONNECT_BASE_TIMEOUT: Duration = Duration::from_secs(1);
const RECONNECT_MAX_TIMEOUT: Duration = Duration::from_secs(90);
const RECONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(15);

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
    /// Check whether the connection is still alive.
    ///
    /// Internally checks that the underlying sender channel is still open.
    pub fn is_alive(&self) -> bool {
        !self.sender.is_closed()
    }

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
            .map_err(|e| anyhow::anyhow!("failed to set suri: {e}"))
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
    url: String,
    max_reconnect_attempts: u8,
    api: Api,

    receiver: UnboundedReceiver<ApiConnectionRequest>,
    sender: UnboundedSender<ApiConnectionRequest>,
}

impl ApiProvider {
    pub async fn new(url: String, max_reconnect_attempts: u8) -> anyhow::Result<Self> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let api = Api::builder()
            .uri(&url)
            .build()
            .await
            .context("failed to connect to API")?;

        Ok(Self {
            session: 0,
            url,
            max_reconnect_attempts,
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
        let mut attempt: u64 = 1;
        loop {
            let url = self.url.clone();
            let mut connect =
                tokio::spawn(async move { Api::builder().uri(url.as_str()).build().await });

            match tokio::time::timeout(RECONNECT_ATTEMPT_TIMEOUT, &mut connect).await {
                Ok(Ok(Ok(api))) => {
                    self.api = api;
                    return true;
                }
                Ok(Ok(Err(err))) => {
                    log::error!(
                        "Failed to create API connection (runtime reconnect attempt {attempt}, initial max_reconnect_attempts={}): {err}",
                        self.max_reconnect_attempts
                    );

                    tokio::time::sleep(reconnect_delay(attempt)).await;
                }
                Ok(Err(err)) => {
                    log::error!(
                        "API connection task failed (runtime reconnect attempt {attempt}, initial max_reconnect_attempts={}): {err}",
                        self.max_reconnect_attempts
                    );

                    tokio::time::sleep(reconnect_delay(attempt)).await;
                }
                Err(_) => {
                    connect.abort();
                    log::error!(
                        "Timed out creating API connection after {RECONNECT_ATTEMPT_TIMEOUT:?} (runtime reconnect attempt {attempt}, initial max_reconnect_attempts={})",
                        self.max_reconnect_attempts
                    );

                    tokio::time::sleep(reconnect_delay(attempt)).await;
                }
            }
            attempt = attempt.saturating_add(1);
        }
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

fn reconnect_delay(attempt: u64) -> Duration {
    let multiplier = 1u32
        .checked_shl(attempt.saturating_sub(1).min(10) as u32)
        .unwrap_or(1024);

    RECONNECT_BASE_TIMEOUT
        .saturating_mul(multiplier)
        .min(RECONNECT_MAX_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_delay_is_fast_initially_and_capped() {
        assert_eq!(reconnect_delay(1), Duration::from_secs(1));
        assert_eq!(reconnect_delay(2), Duration::from_secs(2));
        assert_eq!(reconnect_delay(10), Duration::from_secs(90));
    }
}
