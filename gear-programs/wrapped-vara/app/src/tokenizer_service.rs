use crate::vft_funcs;
use sails_rs::{gstd::msg, prelude::*};

#[derive(Debug, Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum TokenizerEvent {
    Minted { to: ActorId, value: u128 },
    Burned { from: ActorId, value: u128 },
}

#[derive(Clone, Debug, Default)]
pub struct TokenizerService(());

#[sails_rs::service(events = TokenizerEvent)]
impl TokenizerService {
    pub fn new() -> Self {
        Self(())
    }

    pub async fn mint(&mut self) -> CommandReply<u128> {
        let value = msg::value();
        if value == 0 {
            return CommandReply::new(value);
        }

        let to = msg::source();
        if let Err(_err) = vft_funcs::mint(to, value.into()) {
            CommandReply::new(0).with_value(value)
        } else {
            self.notify_on(TokenizerEvent::Minted { to, value })
                .expect("Failed to send `Minted` event");
            CommandReply::new(value)
        }
    }

    pub async fn burn(&mut self, value: u128) -> CommandReply<u128> {
        if value == 0 {
            return CommandReply::new(value);
        }

        let from = msg::source();
        vft_funcs::burn(from, value.into()).expect("Failed to burn value");

        self.notify_on(TokenizerEvent::Burned { from, value })
            .expect("Failed to send `Burned` event");

        CommandReply::new(value).with_value(value)
    }
}
