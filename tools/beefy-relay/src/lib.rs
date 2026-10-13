pub mod protocol;
pub use protocol::*;
pub mod ethereum;
mod evidence;
#[cfg(any(test, feature = "test-support"))]
pub mod fixtures;
pub mod follower;
pub mod root_publication;
pub mod source;
#[cfg(test)]
mod tests;

use url::{Host, Url};
pub fn local_source_rpc(rpc: &str) -> bool {
    let Ok(url) = Url::parse(rpc) else {
        return false;
    };
    if !matches!(url.scheme(), "ws" | "wss")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    let Some((_, authority)) = rpc.split_once("://") else {
        return false;
    };
    if authority
        .split(['/', '?', '#'])
        .next()
        .is_some_and(|authority| {
            authority.is_empty() || authority.contains('@') || authority.ends_with(':')
        })
    {
        return false;
    }
    match url.host() {
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

#[cfg(test)]
fn message_hash(message: &gear_rpc_client::dto::Message) -> crate::Hash32 {
    let mut preimage = Vec::with_capacity(84 + message.payload.len());
    preimage.extend_from_slice(&message.nonce_be);
    preimage.extend_from_slice(&message.source);
    preimage.extend_from_slice(&message.destination);
    preimage.extend_from_slice(&message.payload);
    keccak256(&preimage)
}

#[cfg(test)]
mod endpoint_tests {
    use super::local_source_rpc;
    #[test]
    fn development_signer_requires_a_parsed_loopback_authority() {
        for rpc in [
            "ws://127.0.0.1:9954",
            "wss://[::1]:9944",
            "ws://localhost:9954/",
        ] {
            assert!(local_source_rpc(rpc), "{rpc}");
        }
        for rpc in [
            "ws://example.com:9954",
            "ws://localhost:9954@remote.example:9954",
            "ws://@localhost:9954",
            "ws://user%40name@localhost:9954",
            "ws://localhost.evil:9954",
            "ws://127.0.0.1:99999",
            "ws://localhost:",
            "ws://127.0.0.1:9954/path",
            "ws://127.0.0.1:9954?token=secret",
            "https://127.0.0.1:9954",
        ] {
            assert!(!local_source_rpc(rpc), "{rpc}");
        }
    }
}
