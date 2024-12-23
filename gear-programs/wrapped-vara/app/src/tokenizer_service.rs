use crate::vft_funcs;
use sails_rs::{gstd::msg, prelude::*};

/// Events that are emitted by Tokenizer service.
#[derive(Debug, Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum TokenizerEvent {
    /// Native token was converted into `VFT` token for the specified address.
    Minted { to: ActorId, value: u128 },
    /// `VFT` token was converted into native token for the specified address.
    Burned { from: ActorId, value: u128 },
}

/// Tokenizer service.
///
/// This service is responsible for exchanging native tokens to/from
/// the token specified in `vft-service` on the current program.
/// Conversion rate is always 1:1.
/// 
/// Exchanging is performed by locking/unlocking native token
/// on the program address and minting/burning `VFT` token defined
/// in the `vft-service` on the current program.
#[derive(Clone, Debug, Default)]
pub struct TokenizerService(());

#[sails_rs::service(events = TokenizerEvent)]
impl TokenizerService {
    pub fn new() -> Self {
        Self(())
    }

    /// Lock all the value attached to the message and mint corresponding
    /// amount of the `VFT` tokens.
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

    /// Burn `value` of `VFT` token and return it as a value attached to the message.
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
