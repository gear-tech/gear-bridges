use super::service::erc20_relay;
use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;
use sails_rs::prelude::String;

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
    ERC20Relay(erc20_relay::Error),
}
