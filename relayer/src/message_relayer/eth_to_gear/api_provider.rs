use actix::{Message, ResponseFuture};
use alloy_primitives::FixedBytes;
use anyhow::Context;
use checkpoint_light_client_client::{traits::ServiceCheckpointFor as _, ServiceCheckpointFor};
use eth_events_electra_client::{traits::EthereumEventClient, EthToVaraEvent};
use gear_rpc_client::GearApi;
use gsdk::Api;
use historical_proxy_client::{traits::HistoricalProxy as _, HistoricalProxy};
use primitive_types::H256;
use sails_rs::{
    calls::{Action, Query},
    gclient::calls::GClientRemoting,
    ActorId,
};
use std::time::Duration;
use tokio::sync::{
    mpsc,
    mpsc::{UnboundedReceiver, UnboundedSender},
    oneshot,
};
use uuid::Uuid;

use crate::message_relayer::eth_to_gear::message_sender::MessageStatus;

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
pub struct GearApiActor {
    api: Api,
    suri: String,
    historical_proxy_id: ActorId,
}

impl GearApiActor {
    pub fn new(api: Api, suri: String, historical_proxy_id: ActorId) -> Self {
        Self {
            api,
            suri,
            historical_proxy_id,
        }
    }
}

/// Fetch checkpoint at specified slot.
///
/// This makes use of Historical Proxy & eth-events services.
#[derive(Message)]
#[rtype(result = "anyhow::Result<(u64, H256)>")]
pub struct GetCheckpointSlot {
    pub slot: u64,
}

impl actix::Actor for GearApiActor {
    type Context = actix::Context<Self>;
}

impl actix::Handler<GetCheckpointSlot> for GearApiActor {
    type Result = ResponseFuture<anyhow::Result<(u64, H256)>>;

    fn handle(&mut self, msg: GetCheckpointSlot, _ctx: &mut Self::Context) -> Self::Result {
        let api = self.api.clone();
        let suri = self.suri.clone();
        let historical_proxy_id = self.historical_proxy_id;
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
                .endpoint_for(msg.slot)
                .recv(historical_proxy_id)
                .await
                .context("failed to query historical proxy")?
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Proxy failed to get endpoint for slot #{slot}: {e:?}",
                        slot = msg.slot
                    )
                })?;

            let checkpoint_endpoint = eth_events
                .checkpoint_light_client_address()
                .recv(endpoint)
                .await
                .context("failed to get checkpoint light client address")?;

            let (checkpoint_slot, checkpoint) = service_checkpoint
                .get(msg.slot)
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
#[rtype(result = "(Uuid, MessageStatus)")]
pub struct RelayGearMessage {
    pub suri: String,
    pub payload: EthToVaraEvent,
    pub tx_hash: FixedBytes<32>,
    pub tx_uuid: Uuid,
}

/*
impl actix::Handler<RelayGearMessage> for GearApiActor {
    type Result = ResponseFuture<(Uuid, MessageStatus)>;

    fn handle(&mut self, msg: RelayGearMessage, ctx: &mut Self::Context) -> Self::Result {
        let api = self.api.clone();
        Box::pin(async move {
            let api = gclient::GearApi::from(api)
                .with(msg.suri)
                .context("failed to create gclient with provided suri")?;
            let gas_limit_block = api.block_gas_limit()?;
            let gas_limit = gas_limit_block / 100 * 95;
            let remoting = GClientRemoting::new(api);

            let mut proxy_service = HistoricalProxy::new(remoting.clone());

            let (_, receiver_reply) = proxy_service
                .redirect(
                    payload.proof_block.block.slot,
                    payload.encode(),
                    self.receiver_address
                )
        })
    }
}
*/

pub struct SailsQuery<Q, R, A>
where
    Q: Query<Output = R, Args = A>,
{
    pub query: Q,
    pub target: ActorId,
}

impl<Q, R: 'static, A> actix::Message for SailsQuery<Q, R, A>
where
    Q: Query<Output = R, Args = A>,
{
    type Result = sails_rs::Result<R, sails_rs::errors::Error>;
}

impl<Q, R: 'static, A> actix::Handler<SailsQuery<Q, R, A>> for GearApiActor
where
    Q: Query<Output = R, Args = A> + 'static,
{
    type Result = ResponseFuture<sails_rs::Result<R, sails_rs::errors::Error>>;

    fn handle(&mut self, msg: SailsQuery<Q, R, A>, _ctx: &mut Self::Context) -> Self::Result {
        Box::pin(async move {
            let query = msg.query;
            let result = query.recv(msg.target).await;
            result
        })
    }
}
