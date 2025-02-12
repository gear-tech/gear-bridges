use std::{ops::ControlFlow, sync::mpsc::{channel, Receiver, Sender}, time::Duration};

use futures::executor::block_on;
use gear_rpc_client::GearApi;
use parity_scale_codec::{Decode, Encode};
use primitive_types::H256;
use prometheus::IntGauge;
use tokio::task::JoinHandle;
use utils_prometheus::{impl_metered_service, MeteredService};

use checkpoint_light_client_io::meta::{Order, State, StateRequest};

use crate::message_relayer::common::{EthereumSlotNumber, GSdkArgs, GearBlockNumber};

pub struct CheckpointsExtractor {
    checkpoint_light_client_address: H256,

    latest_checkpoint: Option<EthereumSlotNumber>,

    metrics: Metrics,
    sender: tokio::sync::mpsc::Sender<Request>,
}

impl MeteredService for CheckpointsExtractor {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        latest_checkpoint_slot: IntGauge = IntGauge::new(
            "checkpoint_extractor_latest_checkpoint_slot",
            "Latest slot found in checkpoint light client program state",
        ),
    }
}

impl CheckpointsExtractor {
    pub fn new(checkpoint_light_client_address: H256, sender: tokio::sync::mpsc::Sender<Request>,) -> Self {
        Self {
            checkpoint_light_client_address,
            latest_checkpoint: None,
            metrics: Metrics::new(),
            sender,
        }
    }

    pub fn run(mut self, blocks: Receiver<GearBlockNumber>) -> Receiver<EthereumSlotNumber> {
        let (sender, receiver) = channel();

        tokio::task::spawn_blocking(move || loop {
            let res = block_on(self.run_inner(&sender, &blocks));
            if let Err(err) = res {
                log::error!("Checkpoints extractor failed: {}", err);
            }
        });

        receiver
    }

    async fn run_inner(
        &mut self,
        sender: &Sender<EthereumSlotNumber>,
        blocks: &Receiver<GearBlockNumber>,
    ) -> anyhow::Result<()> {
        loop {
            for block in blocks.try_iter() {
                self.process_block_events(block.0, sender)
                    .await?;
            }
        }
    }

    async fn process_block_events(
        &mut self,
        block: u32,
        sender: &Sender<EthereumSlotNumber>,
    ) -> anyhow::Result<()> {
        let block_hash = {
            let (sender, mut reciever) = tokio::sync::oneshot::channel();
            let request = Request::BlockToHash { block, sender };

            // todo: exit
            self.sender.send(request).await?;

            reciever.await??
        };

        let request = StateRequest {
            order: Order::Reverse,
            index_start: 0,
            count: 1,
        }
        .encode();

        let state = {
            let (sender, mut reciever) = tokio::sync::oneshot::channel();
            let request = Request::ReadState { pid: self.checkpoint_light_client_address, payload: request, at: Some(block_hash), sender };

            // todo: exit
            self.sender.send(request).await?;

            reciever.await??
        };

        let state = hex::decode(&state[2..])?;
        let state = State::decode(&mut &state[..])?;

        assert!(state.checkpoints.len() <= 1);

        let latest_checkpoint = state.checkpoints.first();

        match (latest_checkpoint, self.latest_checkpoint) {
            (None, None) => {}
            (None, Some(_)) => {
                panic!(
                    "Invalid state detected: checkpoint-light-client program contains no checkpoints \
                    but there's one in checkpoints extractor state"
                );
            }
            (Some(checkpoint), None) => {
                self.latest_checkpoint = Some(EthereumSlotNumber(checkpoint.0));

                self.metrics.latest_checkpoint_slot.set(checkpoint.0 as i64);

                log::info!("First checkpoint discovered: {}", checkpoint.0);

                sender.send(EthereumSlotNumber(checkpoint.0))?;
            }
            (Some(latest), Some(stored)) => {
                if latest.0 > stored.0 {
                    self.metrics.latest_checkpoint_slot.set(latest.0 as i64);

                    let latest = EthereumSlotNumber(latest.0);

                    self.latest_checkpoint = Some(latest);

                    log::info!("New checkpoint discovered: {}", latest.0);

                    sender.send(latest)?;
                }
            }
        }

        Ok(())
    }
}

pub enum Request {
    BlockToHash {
        block: u32,
        sender: tokio::sync::oneshot::Sender<anyhow::Result<H256>>,
    },
    ReadState {
        pid: H256,
        payload: Vec<u8>,
        at: Option<H256>,
        sender: tokio::sync::oneshot::Sender<anyhow::Result<String>>,
    },
    LatestFinalizedBlock {
        sender: tokio::sync::oneshot::Sender<anyhow::Result<H256>>,
    },
    BlockHashToNumber {
        hash: H256,
        sender: tokio::sync::oneshot::Sender<anyhow::Result<u32>>,
    },
}

// fn test() ->  {
//     let (sender, receiver) = tokio::mpsc::channel(10_000);

//     tokio::task::spawn(async move || {
//         let gear_api = GearApi::new(
//             &self.args.vara_domain,
//             self.args.vara_port,
//             self.args.vara_rpc_retries,
//         )
//         .await?;

//         loop {
//             let Some(request) = receiver.recv().await else {
//                 // exit from the task
//                 return;
//             };

//             match request {
//                 Request::BlockToHash { block, sender } => {
//                     let result = gear_api.block_number_to_hash(block).await;
//                     let response = match result {
//                         Err(e) if is_transport(&e) => { /* exit from the inner loop and recreate client */ todo!() }
//                         result => result,
//                     };

//                     // we don't care if the other end is closed
//                     let _ = sender.send(response).await;
//                 }

//                 Request::ReadState { pid, payload, at, sender } => {

//                 }
//             }
//         }
//     })

//     sender
// }

pub fn test222(domain: &str, port: u16, retries: u8) -> (JoinHandle<()>, tokio::sync::mpsc::Sender<Request>) {
    let (sender, mut receiver) = tokio::sync::mpsc::channel(10_000);
    let uri = format!("{domain}:{port}");

    let handle = tokio::task::spawn(async move {
        let mut request_last = None;
        loop {
            match loop_body(&uri, retries, &mut receiver, request_last.take()).await {
                ControlFlow::Break(_) => break,
                ControlFlow::Continue(request) => request_last = request,
            }

            // 2 minutes
            tokio::time::sleep(Duration::from_secs(120)).await;
        }
    });

    (handle, sender)
}

async fn loop_body(uri: &str, retries: u8, receiver: &mut tokio::sync::mpsc::Receiver<Request>, request_last: Option<Request>) -> ControlFlow<(), Option<Request>> {
    let Ok(gsdk_api) = gsdk::Api::builder().retries(retries).build(uri).await else {
        return ControlFlow::Continue(request_last);
    };

    if let Some(request) = request_last {
        match process_request(&gsdk_api, request).await {
            Ok(_) => (),
            Err(Error2::Unknown) => todo!(),
            Err(Error2::Transport(request)) => return ControlFlow::Continue(Some(request)),
        }
    }

    loop_inner(&gsdk_api, receiver).await
}

fn is_transport(_e: &anyhow::Error) -> bool {
    todo!()
}

async fn loop_inner(gsdk_api: &gsdk::Api, receiver: &mut tokio::sync::mpsc::Receiver<Request>) -> ControlFlow<(), Option<Request>> {
    loop {
        let Some(request) = receiver.recv().await else {
            // exit from the task
            return ControlFlow::Break(());
        };

        match process_request(gsdk_api, request).await {
            Ok(_) => (),
            Err(Error2::Unknown) => todo!(),
            Err(Error2::Transport(request)) => return ControlFlow::Continue(Some(request)),
        }
    }
}

pub enum Error2 {
    // transport error occurred. Contains the request being processed
    Transport(Request),
    Unknown,
}

async fn process_request(gsdk_api: &gsdk::Api, request: Request) -> Result<(), Error2> {
    match request {
        Request::BlockToHash { block, sender } => {
            let result = GearApi::from(gsdk_api.clone()).block_number_to_hash(block).await;
            let response = match result {
                Err(e) if is_transport(&e) => return Err(Error2::Transport(Request::BlockToHash { block, sender })),
                result => result,
            };

            // we don't care if the other end is closed
            let _ = sender.send(response);
        }

        Request::ReadState { pid, payload, at, sender } => {
            let result = gsdk_api.read_state(pid, payload.clone(), at).await
                .map_err(Into::into);
            let response = match result {
                Err(e) if is_transport(&e) => return Err(Error2::Transport(Request::ReadState { pid, payload, at, sender })),
                result => result,
            };

            // we don't care if the other end is closed
            let _ = sender.send(response);
        }

        Request::LatestFinalizedBlock { sender } => {
            let result = GearApi::from(gsdk_api.clone()).latest_finalized_block().await;
            let response = match result {
                Err(e) if is_transport(&e) => return Err(Error2::Transport(Request::LatestFinalizedBlock { sender })),
                result => result,
            };

            // we don't care if the other end is closed
            let _ = sender.send(response);
        }

        Request::BlockHashToNumber { hash, sender } => {
            let result = GearApi::from(gsdk_api.clone()).block_hash_to_number(hash).await;
            let response = match result {
                Err(e) if is_transport(&e) => return Err(Error2::Transport(Request::BlockHashToNumber { hash, sender })),
                result => result,
            };

            // we don't care if the other end is closed
            let _ = sender.send(response);
        }
    }

    Ok(())
}
