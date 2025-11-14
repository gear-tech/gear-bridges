use actix::{Message, ResponseFuture};
use anyhow::Context;
use checkpoint_light_client_client::{traits::ServiceCheckpointFor as _, ServiceCheckpointFor};
use eth_events_electra_client::traits::EthereumEventClient;
use gear_rpc_client::GearApi;
use gsdk::Api;
use historical_proxy_client::{traits::HistoricalProxy as _, HistoricalProxy};
use primitive_types::H256;
use sails_rs::{
    calls::{Action, Call, Query},
    gclient::calls::GClientRemoting,
    ActorId,
};
use std::time::Duration;
use tokio::sync::{
    mpsc,
    mpsc::{UnboundedReceiver, UnboundedSender},
    oneshot,
};

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
    domain: String,
    port: u16,
    max_reconnect_attempts: u8,
    api: Api,

    receiver: UnboundedReceiver<ApiConnectionRequest>,
    sender: UnboundedSender<ApiConnectionRequest>,
}

impl ApiProvider {
    pub async fn new(
        domain: String,
        port: u16,
        max_reconnect_attempts: u8,
    ) -> anyhow::Result<Self> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let uri: &str = &format!("{domain}:{port}");
        let api = Api::builder()
            .build(uri)
            .await
            .context("failed to connect to API")?;

        Ok(Self {
            session: 0,
            domain,
            port,
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
        let uri: &str = &format!("{}:{}", self.domain, self.port);

        for attempt in 0..self.max_reconnect_attempts {
            match Api::builder().build(uri).await {
                Ok(api) => {
                    self.api = api;
                    return true;
                }
                Err(err) => {
                    log::error!(
                        "Failed to create API connection (attempt {attempt}/{max_reconnect_attempts}): {err}",
                        max_reconnect_attempts = self.max_reconnect_attempts
                    );

                    tokio::time::sleep(RECONNECT_TIMEOUT).await;
                }
            }
        }

        log::error!(
            "All {max_reconnect_attempts} attempts to connect to API failed. Giving up.",
            max_reconnect_attempts = self.max_reconnect_attempts
        );
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

/// An Actix actor that provides Gear API connectivity. Instead
/// of actors using Gear API directly, they should send requests
/// to this actor to perform Gear API operations on their behalf.
///
/// When there is Sails operation to be performed, client should
/// send a message to this actor and await the response.
///
// TODO(Adel): proper error handling once we decide on the
// usage of Actix.
pub struct GearApiActor {
    api: Api,
}

impl GearApiActor {
    pub fn new(api: Api) -> Self {
        Self { api }
    }
}

impl actix::Actor for GearApiActor {
    type Context = actix::Context<Self>;
}

#[derive(Message)]
#[rtype(result = "anyhow::Result<u64>")]
pub struct GetGasLimit;

impl actix::Handler<GetGasLimit> for GearApiActor {
    type Result = anyhow::Result<u64>;

    fn handle(&mut self, _msg: GetGasLimit, _ctx: &mut Self::Context) -> Self::Result {
        self.api
            .gas_limit()
            .context("failed to get gas limit from Gear node")
    }
}

/// Construct a remoting instance connected to the Gear node with the given SURI.
#[derive(Message)]
#[rtype(result = "anyhow::Result<GClientRemoting>")]
pub struct GetRemoting {
    pub suri: String,
}

impl actix::Handler<GetRemoting> for GearApiActor {
    type Result = anyhow::Result<GClientRemoting>;

    fn handle(&mut self, msg: GetRemoting, _ctx: &mut Self::Context) -> Self::Result {
        let api = self.api.clone();
        let suri = msg.suri;
        let gclient = gclient::GearApi::from(api)
            .with(&suri)
            .context("failed to create gclient with provided suri");

        gclient.map(|api| GClientRemoting::new(api))
    }
}

/// Fetch checkpoint at specified slot.
///
/// This makes use of Historical Proxy & eth-events services.
#[derive(Message)]
#[rtype(result = "anyhow::Result<(u64, H256)>")]
pub struct GetCheckpointSlot {
    pub slot: u64,
    pub suri: String,
    pub historical_proxy_id: ActorId,
}

impl actix::Handler<GetCheckpointSlot> for GearApiActor {
    type Result = ResponseFuture<anyhow::Result<(u64, H256)>>;

    fn handle(&mut self, msg: GetCheckpointSlot, _ctx: &mut Self::Context) -> Self::Result {
        let GetCheckpointSlot {
            suri,
            historical_proxy_id,
            slot,
        } = msg;

        let api = self.api.clone();

        // we don't access actor ctx: can just return a future
        Box::pin(async move {
            let api = gclient::GearApi::from(api)
                .with(&suri)
                .context("failed to create gclient with provided suri")?;
            let gas_limit = api.block_gas_limit()?;

            let remoting = GClientRemoting::new(api);
            let historical_proxy = HistoricalProxy::new(remoting.clone());
            let eth_events = eth_events_electra_client::EthereumEventClient::new(remoting.clone());
            let service_checkpoint = ServiceCheckpointFor::new(remoting);

            let endpoint = historical_proxy
                .endpoint_for(slot)
                .recv(historical_proxy_id)
                .await
                .context("failed to query historical proxy")?
                .map_err(|e| {
                    anyhow::anyhow!("Proxy failed to get endpoint for slot #{slot}: {e:?}",)
                })?;

            let checkpoint_endpoint = eth_events
                .checkpoint_light_client_address()
                .recv(endpoint)
                .await
                .context("failed to get checkpoint light client address")?;

            let (checkpoint_slot, checkpoint) = service_checkpoint
                .get(slot)
                .with_gas_limit(gas_limit)
                .recv(checkpoint_endpoint)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to receive checkpoint: {e:?}"))?
                .map_err(|e| anyhow::anyhow!("Checkpoint error: {e:?}"))?;

            Ok((checkpoint_slot, checkpoint))
        })
    }
}

#[derive(Message)]
#[rtype(result = "anyhow::Result<(Vec<u8>, Vec<u8>)>")]
pub struct ProxyRedirect {
    pub suri: String,
    pub historical_proxy_address: ActorId,
    pub slot: u64,
    pub proofs: Vec<u8>,
    pub receiver_address: ActorId,
    pub receiver_route: Vec<u8>,
}

impl actix::Handler<ProxyRedirect> for GearApiActor {
    type Result = ResponseFuture<anyhow::Result<(Vec<u8>, Vec<u8>)>>;

    fn handle(&mut self, msg: ProxyRedirect, _ctx: &mut Self::Context) -> Self::Result {
        let ProxyRedirect {
            suri,
            historical_proxy_address,
            slot,
            proofs,
            receiver_address,
            receiver_route,
        } = msg;

        let api = self.api.clone();

        Box::pin(async move {
            let api = gclient::GearApi::from(api)
                .with(&suri)
                .context("failed to create gclient with provided suri")?;
            let gas_limit_block = api.block_gas_limit()?;
            let gas_limit = gas_limit_block / 100 * 95;

            let remoting = GClientRemoting::new(api);
            let mut historical_proxy = HistoricalProxy::new(remoting.clone());

            let (receipt_rlp, reply) = historical_proxy
                .redirect(slot, proofs, receiver_address, receiver_route)
                .with_gas_limit(gas_limit)
                .send_recv(historical_proxy_address)
                .await?
                .map_err(|e| anyhow::anyhow!("Historical proxy error: {e:?}"))?;

            Ok((receipt_rlp, reply))
        })
    }
}
