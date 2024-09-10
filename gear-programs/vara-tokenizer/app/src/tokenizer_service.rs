use crate::admin_service;
use sails_rs::{calls::*, gstd::msg, prelude::*};
use vft_client::traits::Vft;

static mut STORAGE: Option<TokenizerConfig> = None;

#[derive(Debug)]
pub(crate) struct TokenizerConfig {
    pub vft_address: ActorId,
}

pub(crate) fn init(vft_address: ActorId) {
    unsafe {
        STORAGE = Some(TokenizerConfig { vft_address });
    };
}

pub(crate) fn storage_mut() -> &'static mut TokenizerConfig {
    unsafe { STORAGE.as_mut().expect("program is not initialized") }
}

pub(crate) fn storage() -> &'static TokenizerConfig {
    unsafe { STORAGE.as_ref().expect("program is not initialized") }
}

#[derive(Debug, Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum TokenizerEvent {
    Minted { to: ActorId, value: u128 },
    Burned { from: ActorId, value: u128 },
}

#[derive(Debug, Clone)]
pub struct TokenizerService<V> {
    vft_client: V,
}

#[sails_rs::service(events = TokenizerEvent)]
impl<V> TokenizerService<V>
where
    V: Vft,
{
    pub fn new(vft_client: V) -> Self {
        Self { vft_client }
    }

    pub async fn mint_from_value(&mut self) -> u128 {
        let value = msg::value();
        if value == 0 {
            return value;
        }

        let source = msg::source();
        let vft_program_id = storage().vft_address;

        let success = self
            .vft_client
            .mint(source, value.into())
            .send_recv(vft_program_id)
            .await
            .expect("Failed to send message to vft program");
        if !success {
            panic!("mint failed");
        }
        self.notify_on(TokenizerEvent::Minted { to: source, value })
            .expect("Failed to send event");
        value
    }

    pub async fn burn_and_return_value(&mut self, value: u128) -> u128 {
        if value == 0 {
            return value;
        }

        let source = msg::source();
        let vft_program_id = storage().vft_address;

        let success = self
            .vft_client
            .burn(source, value.into())
            .send_recv(vft_program_id)
            .await
            .expect("Failed to send message to vft program");
        if !success {
            panic!("burn failed");
        }
        self.notify_on(TokenizerEvent::Burned {
            from: source,
            value,
        })
        .expect("Failed to send event");

        // TODO reply with value `program::send_reply_with_value` when `sails` allows it
        msg::send_bytes_with_gas(source, vec![], 0, value)
            .expect("Failed to send message with retuned value");
        value
    }

    pub fn vft_address(&self) -> ActorId {
        storage().vft_address
    }

    pub fn update_vft_address(&mut self, new_vft_address: ActorId) {
        admin_service::ensure_is_admin();
        storage_mut().vft_address = new_vft_address;
    }
}
