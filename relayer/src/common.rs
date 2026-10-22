use std::time::Duration;

use alloy::transports::{RpcError, TransportErrorKind};
use gsdk::ext::subxt_rpcs;

use crate::rpc::{self, RetryDecision};

pub(crate) fn is_rpc_transport_error_recoverable(err: &RpcError<TransportErrorKind>) -> bool {
    rpc::classify_alloy_rpc(err) == RetryDecision::Retry
}

pub(crate) fn is_transport_error_recoverable(err: &anyhow::Error) -> bool {
    if rpc::classify_anyhow(err) == RetryDecision::Retry {
        return true;
    }

    if let Some(ethereum_client::Error::ErrorInHTTPTransport(err)) =
        err.downcast_ref::<ethereum_client::Error>()
    {
        return is_rpc_transport_error_recoverable(err);
    }

    // raw provider calls return `RpcError`.
    if let Some(err) = err.downcast_ref::<RpcError<TransportErrorKind>>() {
        return is_rpc_transport_error_recoverable(err);
    }

    // sails calls return gclient error which can contain subxt error with rpc transport error
    if let Some(gclient::Error::GearSDK(gsdk::Error::Subxt(err))) =
        err.downcast_ref::<gclient::Error>()
    {
        if err.is_disconnected_will_reconnect() {
            return true;
        }
        if let subxt::Error::Rpc(rpc) = &**err {
            match rpc {
                subxt::error::RpcError::SubscriptionDropped => return true,
                subxt::error::RpcError::ClientError(
                    subxt_rpcs::Error::DisconnectedWillReconnect(_),
                ) => return true,
                _ => (),
            }
        }
    }

    false
}

pub struct BlockRange {
    pub from: u64,
    pub to: u64,
}

/// Creates BlockRange that does not exceed the maximum allowed range (i.e. to avoid
/// an error 'server returned an error response: error code -32602: query exceeds max block range 100000').
pub fn create_range(from: Option<u64>, latest: u64) -> BlockRange {
    let Some(from) = from else {
        return BlockRange {
            from: latest,
            to: latest,
        };
    };

    let block_to_max = from + 99_999;
    let block_to = if block_to_max > latest {
        latest
    } else {
        block_to_max
    };

    BlockRange { from, to: block_to }
}

pub const MAX_RETRIES: u32 = 10;
pub const BASE_RETRY_DELAY: Duration = Duration::from_secs(10);
