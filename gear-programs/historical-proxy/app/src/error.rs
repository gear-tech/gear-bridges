use super::service::ethereum_event_client;
use parity_scale_codec::{Decode, Encode};
use sails_rs::prelude::String;
use scale_info::TypeInfo;

#[derive(Debug, Decode, Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum ProxyError {
    NoEndpointForSlot(u64),
    SendFailure(String),
    ReplyTimeout(String),
    ReplyFailure(String),
    DecodeFailure(String),
    NotAdmin,
    EthereumEventClient(ethereum_event_client::Error),
}
