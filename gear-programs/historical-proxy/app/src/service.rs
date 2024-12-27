// Incorporate code generated based on the IDL file

#[allow(dead_code)]
#[allow(clippy::module_inception)]
pub(crate) mod ethereum_event_client {
    include!(concat!(env!("OUT_DIR"), "/ethereum_event_client.rs"));
}

use sails_rs::{calls::ActionIo, gstd};

use cell::RefCell;
use sails_rs::{gstd::ExecContext, prelude::*};

use crate::{
    error::ProxyError,
    state::{ProxyState, Slot},
};

/// Events enmitted by the Historical Proxy service.
#[derive(Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
enum Event {
    /// Tx receipt is checked to be valid and successfully sent to the
    /// underlying program.
    Relayed {
        /// Ethereum slot containing target transaction.
        slot: u64,
        /// Ethereum block number which contains target transaction.
        block_number: u64,
        /// Index of the target transaction in the `block_number`.
        transaction_index: u32,
    },
}

/// Historical Proxy service.
///
/// `etereum-event-client` programs can become outdated with Ethereum updates, so
/// every `ethereum-event-client` ever deployed is valid for some Ethereum slot interval.
///
/// When Ethereum updates in a way incompatible with `ethereum-event-cleint`(or if we need to
/// update `ethereum-event-client` for some other reason) we need to deploy a new version of
/// `ethereum-event-client` and still have access to the old one(in order to process
/// historical transactions).
///
/// This service provides such an access. For every `ethereum-event-client` ever deployed
/// it maps Ethereum slot from which this `ethereum-event-client` is valid from.
///
/// When user makes request to the Historical Proxy service he will specify Ethereum slot number
/// where the target transaction was sent. Historical Proxy will decide which `ethereum-event-client`
/// is responsible of processing transactions for this slot and will redirect user request to it.
/// If `ethereum-event-client` returned success its reply will be redirected to the program
/// that user have specified in his request. For more info see `redirect` implementation.
pub struct HistoricalProxyService<'a, ExecContext> {
    state: &'a RefCell<ProxyState>,
    exec_context: ExecContext,
}

#[sails_rs::service(events = Event)]
impl<'a, T> HistoricalProxyService<'a, T>
where
    T: ExecContext,
{
    pub fn new(state: &'a RefCell<ProxyState>, exec_context: T) -> Self {
        Self {
            state,
            exec_context,
        }
    }

    /// Get current service admin.
    pub fn admin(&self) -> ActorId {
        self.state.borrow().admin
    }

    /// Get endpoint for the specified `slot`.
    pub fn endpoint_for(&self, slot: Slot) -> Result<ActorId, ProxyError> {
        self.state.borrow().endpoints.endpoint_for(slot)
    }

    /// Add new endpoint to the map. Endpoint will be effective for all the
    /// requests with slots starting from `slot`.
    ///
    /// This function can be called only by an admin.
    pub fn add_endpoint(&mut self, slot: Slot, endpoint: ActorId) -> Result<(), ProxyError> {
        let source = self.exec_context.actor_id();

        let mut state = self.state.borrow_mut();
        if source != state.admin {
            return Err(ProxyError::NotAdmin);
        }

        state.endpoints.push(slot, endpoint);
        Ok(())
    }

    /// Get endpoint map stored in this service.
    pub fn endpoints(&self) -> Vec<(Slot, ActorId)> {
        self.state.borrow().endpoints.endpoints()
    }

    /// Redirect message to `ethereum-event-client` program which is valid for `slot`.
    /// If message is relayed successfully then reply is sent to `client` address
    /// to `client_route` route.
    ///
    /// # Parameters
    ///
    /// - `slot`: slot for which message is relayed.
    /// - `proofs`: SCALE encoded `EthToVaraEvent`.
    /// - `client`: client address to send receipt to on success.
    /// - `client_route`: route to send receipt to on success.
    ///
    /// # Returns
    ///
    /// - `(Vec<u8>, Vec<u8>)`: on success where first vector is receipt and second vector is reply from calling `client_route`.
    /// - `ProxyError`: if redirect failed
    #[allow(clippy::await_holding_refcell_ref)]
    pub async fn redirect(
        &mut self,
        slot: Slot,
        proofs: Vec<u8>,
        client: ActorId,
        client_route: Vec<u8>,
    ) -> Result<(Vec<u8>, Vec<u8>), ProxyError> {
        let state = self.state.borrow();
        let endpoint = state.endpoints.endpoint_for(slot)?;
        drop(state);
        // 1) check if proofs are correct and receive data for further processing
        let check_proofs = {
            let mut payload =
                ethereum_event_client::ethereum_event_client::io::CheckProofs::ROUTE.to_vec();
            payload.extend_from_slice(&proofs);
            payload
        };

        let ethereum_event_client::CheckedProofs {
            receipt_rlp,
            transaction_index,
            block_number,
        } = ethereum_event_client::ethereum_event_client::io::CheckProofs::decode_reply(
            gstd::msg::send_bytes_for_reply(endpoint, check_proofs, 0, 0)
                .map_err(|e| {
                    ProxyError::SendFailure(format!(
                        "failed to send message to ethereum-event-client: {:?}",
                        e
                    ))
                })?
                .await
                .map_err(|e| {
                    ProxyError::ReplyFailure(format!(
                        "failed to receive reply from ethereum-event-client: {:?}",
                        e
                    ))
                })?,
        )
        .map_err(|e| {
            ProxyError::DecodeFailure(format!(
                "failed to decode reply from ethereum-event-client: {:?}",
                e
            ))
        })?
        .map_err(ProxyError::EthereumEventClient)?;
        // 2) Invoke client with a receipt. Uses route and address suplied by the user.
        let submit_receipt = {
            let params = (slot, transaction_index, receipt_rlp.clone());
            let mut payload = Vec::with_capacity(params.encoded_size() + client_route.len());
            payload.extend_from_slice(&client_route);
            params.encode_to(&mut payload);
            payload
        };

        let reply = gstd::msg::send_bytes_for_reply(client, submit_receipt, 0, 0)
            .map_err(|e| {
                ProxyError::SendFailure(format!("failed to send message to client: {:?}", e))
            })?
            .await
            .map_err(|e| {
                ProxyError::ReplyFailure(format!("failed to receive reply from client: {:?}", e))
            })?;

        let _ = self.notify_on(Event::Relayed {
            slot,
            block_number,
            transaction_index: transaction_index as u32,
        });

        Ok((receipt_rlp, reply))
    }
}
