use crate::vft_funcs;
use sails_rs::{gstd::msg, prelude::*};
use vft_service::{utils::Result, Service as VftService};

#[derive(Debug, Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum TokenizerEvent {
    Minted { to: ActorId, value: u128 },
    Burned { from: ActorId, value: u128 },
}

#[derive(Clone)]
pub struct TokenizerService {
    vft: VftService,
}

impl Default for TokenizerService {
    fn default() -> Self {
        Self::new()
    }
}

#[sails_rs::service(extends = VftService, events = TokenizerEvent)]
impl TokenizerService {
    pub fn new() -> Self {
        Self {
            vft: VftService::new(),
        }
    }

    pub async fn mint_from_value(&mut self) -> Result<u128> {
        let value = msg::value();
        if value == 0 {
            return Ok(value);
        }

        let to = msg::source();
        if let Err(err) = vft_funcs::mint(to, value.into()) {
            // TODO reply with value `program::send_reply_with_value` when `sails` allows it
            // see https://github.com/gear-tech/sails/issues/475
            msg::send_bytes_with_gas(to, vec![], 0, value).expect("Failed to send value to user");
            Err(err)
        } else {
            self.notify_on(TokenizerEvent::Minted { to, value })
                .expect("Failed to send `Minted` event");
            Ok(value)
        }
    }

    pub async fn burn_and_return_value(&mut self, value: u128) -> Result<u128> {
        if value == 0 {
            return Ok(value);
        }

        let from = msg::source();
        vft_funcs::burn(from, value.into())?;

        self.notify_on(TokenizerEvent::Burned { from, value })
            .expect("Failed to send `Burned` event");

        // TODO reply with value `program::send_reply_with_value` when `sails` allows it
        // see https://github.com/gear-tech/sails/issues/475
        msg::send_bytes_with_gas(from, vec![], 0, value).expect("Failed to send value to user");
        Ok(value)
    }
}

impl AsRef<VftService> for TokenizerService {
    fn as_ref(&self) -> &VftService {
        &self.vft
    }
}
