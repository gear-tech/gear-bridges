use crate::extended_vft_client::traits::Vft;
use sails_rs::{calls::*, gstd::msg, prelude::*};

static mut STORAGE: Option<TokenizerConfig> = None;

#[derive(Debug)]
pub(crate) struct TokenizerConfig {
    pub vft_program_id: ActorId,
}

pub(crate) fn init(vft_program_id: ActorId) {
    unsafe {
        STORAGE = Some(TokenizerConfig { vft_program_id });
    };
}

// pub(crate) fn storage_mut() -> &'static mut TokenizerConfig {
//     unsafe { STORAGE.as_mut().expect("program is not initialized") }
// }

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
        let source = msg::source();
        let value = msg::value();
        let vft_program_id = storage().vft_program_id;

        let success = self
            .vft_client
            .mint(source, value.into())
            .send_recv(vft_program_id)
            .await
            .unwrap();
        if !success {
            panic!("mint failed");
        }
        self.notify_on(TokenizerEvent::Minted { to: source, value })
            .unwrap();
        value
    }

    pub async fn burn_and_return_value(&mut self, value: u128) -> u128 {
        let source = msg::source();
        let vft_program_id = storage().vft_program_id;

        let success = self
            .vft_client
            .burn(source, value.into())
            .send_recv(vft_program_id)
            .await
            .unwrap();
        if !success {
            panic!("burn failed");
        }
        self.notify_on(TokenizerEvent::Burned {
            from: source,
            value,
        })
        .unwrap();
        // TODO program::send_reply_with_value
        value
    }
}
