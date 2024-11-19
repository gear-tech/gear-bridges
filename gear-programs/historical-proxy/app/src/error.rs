use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;

use crate::service::vft;

#[derive(Debug, Clone, Decode, Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum ProxyError {
    NoEndpointForSlot(u64),
    SendFailure,
    ReplyTimeout,
    ReplyHook,
    ReplyFailure,
    DecodeFailure,
    ERC20Relay(erc20_relay_client::Error),
    VftManager(vft::Error),
}
