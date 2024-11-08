use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;


#[derive(Debug, Clone, Decode, Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum ProxyError {
    NoEndpoint,
}