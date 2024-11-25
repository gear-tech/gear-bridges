use super::error::ProxyError;
use super::*;
pub type Slot = u64;

pub struct ProxyState {
    pub admin: ActorId,
    pub endpoints: EndpointList,
    pub config: Config,
}

#[derive(Clone, Copy, Debug, Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct Config {
    pub reply_timeout: u32,
    pub reply_deposit: u64,
}

pub struct EndpointList(Vec<(Slot, ActorId)>);

impl Default for EndpointList {
    fn default() -> Self {
        Self::new()
    }
}

impl EndpointList {
    pub fn new() -> Self {
        Self(Vec::with_capacity(2))
    }

    pub fn push(&mut self, slot: Slot, actor_id: ActorId) {
        assert!(
            self.0.is_empty() || self.0[self.0.len() - 1].0 < slot,
            "new endpoint should have slot >= current"
        );
        self.0.push((slot, actor_id));
    }

    pub fn endpoints(&self) -> Vec<(Slot, ActorId)> {
        self.0.clone()
    }

    pub fn endpoint_for(&self, slot: Slot) -> Result<ActorId, ProxyError> {
        match self.0.binary_search_by(|(s, _)| s.cmp(&slot)) {
            Ok(i) => Ok(self.0[i].1),
            Err(next) => match self.0.get(next) {
                Some(result) => Ok(result.1),
                None => Err(ProxyError::NoEndpointForSlot(slot)),
            },
        }
    }
}
