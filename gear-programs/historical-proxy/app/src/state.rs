use super::error::ProxyError;
use super::{ActorId, Vec};

pub type Slot = u64;

/// State of the Historical Proxy service.
pub struct ProxyState {
    pub admin: ActorId,
    pub endpoints: EndpointList,
}

/// Mapping between endpoints and Ethereum slots they're active from.
///
/// ### Invariant
///
/// Endpoints are stored in ascending order, sorted by slot number.
#[derive(Default)]
pub struct EndpointList(Vec<(Slot, ActorId)>);

impl EndpointList {
    pub fn new() -> Self {
        Self(Vec::with_capacity(2))
    }

    /// Add new endpoint that will be active starting from `slot`(inclusive).
    ///
    /// Panics if provided `slot` <= greatest already existing slot.
    pub fn push(&mut self, slot: Slot, actor_id: ActorId) {
        assert!(
            self.0.is_empty() || self.0[self.0.len() - 1].0 < slot,
            "new endpoint should have slot >= current"
        );
        self.0.push((slot, actor_id));
    }

    /// Get list of currently active endpoints. Returns `Vec<(Slot, ActorId)>`
    /// where `ActorId` means endpoint address and `Slot` means Ethereum slot
    /// this endpoint is active from(inclusive).
    pub fn endpoints(&self) -> Vec<(Slot, ActorId)> {
        self.0.clone()
    }

    /// Get endpoint for the specified slot. Will return error if endpoint is not found.
    pub fn endpoint_for(&self, slot: Slot) -> Result<ActorId, ProxyError> {
        match self.0.binary_search_by(|(s, _)| s.cmp(&slot)) {
            Ok(i) => Ok(self.0[i].1),
            Err(next) if next != 0 => match self.0.get(next - 1) {
                Some(result) => Ok(result.1),
                None => Err(ProxyError::NoEndpointForSlot(slot)),
            },
            Err(_) => Err(ProxyError::NoEndpointForSlot(slot)),
        }
    }
}
