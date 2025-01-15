use super::service::ethereum_event_client;
use parity_scale_codec::{Decode, Encode};
use sails_rs::prelude::String;
use scale_info::TypeInfo;

/// Errors returned by the Historical Proxy service.
#[derive(Debug, Decode, Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum ProxyError {
    /// Endpoint for requested slot not found.
    NoEndpointForSlot(u64),
    /// Failed to send message.
    SendFailure(String),
    /// Failed to receive reply.
    ReplyFailure(String),
    /// Failed to decode reply.
    DecodeFailure(String),
    /// `ethereum-event-client` returned error.
    EthereumEventClient(ethereum_event_client::Error),
}
