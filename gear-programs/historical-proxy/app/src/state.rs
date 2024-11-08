use super::*;
use super::error::ProxyError;
pub type Slot = u64;

pub struct ProxyState {

}

pub struct EndpointList(Vec<(Slot, ActorId)>);

impl EndpointList {
    pub fn push(&mut self, slot: Slot, actor_id: ActorId) {
        assert!(self.0.is_empty() || self.0[self.0.len()-1].0 < slot, "new endpoint should have slot >= current");
        self.0.push((slot, actor_id));
    }

    pub fn endpoints(&self) -> Vec<(Slot, ActorId)> {
        self.0.clone()
    }

    pub fn endpoint_for(&self, slot: Slot) -> Result<(Slot, ActorId), ProxyError> {
        todo!()
    }
}