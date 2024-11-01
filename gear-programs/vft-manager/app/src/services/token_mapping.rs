use collections::HashMap;
use sails_rs::prelude::*;

use super::error::Error;

#[derive(Debug, Default)]
pub struct TokenMap {
    vara_to_eth: HashMap<ActorId, H160>,
    eth_to_vara: HashMap<H160, ActorId>,
}

impl TokenMap {
    pub fn insert(&mut self, vara_token_id: ActorId, eth_token_id: H160) -> Result<(), Error> {
        let already_present = self
            .vara_to_eth
            .insert(vara_token_id, eth_token_id)
            .is_some();
        if already_present {
            return Err(Error::TokenMappingError);
        }

        let already_present = self
            .eth_to_vara
            .insert(eth_token_id, vara_token_id)
            .is_some();
        if already_present {
            return Err(Error::TokenMappingError);
        }

        Ok(())
    }

    pub fn remove(&mut self, vara_token_id: ActorId) -> Result<H160, Error> {
        let eth_token_id = self
            .vara_to_eth
            .remove(&vara_token_id)
            .ok_or(Error::TokenMappingError)?;

        let _ = self
            .eth_to_vara
            .remove(&eth_token_id)
            .ok_or(Error::TokenMappingError)?;

        Ok(eth_token_id)
    }

    pub fn get_eth_token_id(&self, vara_token_id: &ActorId) -> Result<H160, Error> {
        self.vara_to_eth
            .get(vara_token_id)
            .cloned()
            .ok_or(Error::NoCorrespondingEthAddress)
    }

    pub fn get_vara_token_id(&self, eth_token_id: &H160) -> Result<ActorId, Error> {
        self.eth_to_vara
            .get(eth_token_id)
            .cloned()
            .ok_or(Error::NoCorrespondingVaraAddress)
    }

    pub fn read_state(&self) -> Vec<(ActorId, H160)> {
        self.vara_to_eth.clone().into_iter().collect()
    }
}
