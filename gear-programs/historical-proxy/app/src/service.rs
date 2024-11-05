use ptr::addr_of_mut;
use sails_rs::{gstd, prelude::*};

struct ProxyData {
    checkpoint_clients: Vec<(ActorId, u64)>,
}

static mut PROXY_DATA: Option<ProxyData> = None;

fn proxy_data_mut() -> &'static mut ProxyData {
    unsafe {
        match addr_of_mut!(PROXY_DATA).as_mut().unwrap() {
            Some(proxy) => return proxy,
            None => {
                panic!("ProxyData not initialized")
            }
        }
    }
}
pub struct HistoricalProxyService(());

#[sails_rs::service]
impl HistoricalProxyService {
    pub fn new() -> Self {
        Self(())
    }

    pub fn add_checkpoint(&mut self, slot: u64, actor_id: ActorId) {
        let data = proxy_data_mut();

        data.checkpoint_clients.push((actor_id, slot));
        data.checkpoint_clients.sort_by(|a, b| a.1.cmp(&b.1))
    }

    pub fn checkpoint_by_slot(&self, slot: u64) -> Option<ActorId> {
        let data = proxy_data_mut();
        match data
            .checkpoint_clients
            .binary_search_by(|(_, slot_current)| slot_current.cmp(&slot))
        {
            Ok(index) => Some(data.checkpoint_clients[index].0),
            Err(index_next) => match data.checkpoint_clients.get(index_next) {
                Some((actor_id, _)) => {
                    return Some(*actor_id);
                }
                None => None,
            },
        }
    }

    pub async fn proxy(&mut self, slot: u64, payload: Vec<u8>) -> Vec<u8> {
        let actor = self.checkpoint_by_slot(slot).expect("no checkpoint found");

        let reply = gstd::msg::send_bytes_for_reply(actor, payload, 0, 0)
            .unwrap()
            .await;
        reply.expect("todo: handle errors")
    }
}
