use checkpoint_light_client_client::{traits::ServiceState, Order};
use gclient::GearApi;
use primitive_types::H256;
use sails_rs::{calls::Query, gclient::calls::GClientRemoting};

use super::common::EthereumSlotNumber;

pub mod all_token_transfers;
pub mod api_provider;
pub mod manual;
pub mod paid_token_transfers;

pub mod message_sender;
pub mod proof_composer;
pub mod storage;
pub mod tx_manager;

pub async fn get_latest_checkpoint(
    checkpoint_light_client_address: H256,
    gear_api: GearApi,
) -> Option<EthereumSlotNumber> {
    let remoting = GClientRemoting::new(gear_api);
    checkpoint_light_client_client::ServiceState::new(remoting)
        .get(Order::Reverse, 0, 1)
        .recv(checkpoint_light_client_address.0.into())
        .await
        .ok()
        .map(|state| {
            state
                .checkpoints
                .last()
                .map(|(checkpoint, _)| EthereumSlotNumber(*checkpoint))
        })
        .unwrap_or(None)
}
