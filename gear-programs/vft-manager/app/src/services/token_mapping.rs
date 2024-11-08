use collections::HashMap;
use sails_rs::prelude::*;

use super::{error::Error, TokenSupply};

#[derive(Debug, Default)]
pub struct TokenMap {
    vara_to_eth: HashMap<ActorId, H160>,
    eth_to_vara: HashMap<H160, ActorId>,
    supply_mapping: HashMap<ActorId, TokenSupply>,
}

impl TokenMap {
    pub fn insert(&mut self, vara_token_id: ActorId, eth_token_id: H160, supply: TokenSupply) {
        if self
            .vara_to_eth
            .insert(vara_token_id, eth_token_id)
            .is_some()
        {
            panic!("Mapping already present");
        }

        if self
            .eth_to_vara
            .insert(eth_token_id, vara_token_id)
            .is_some()
        {
            panic!("Mapping already present");
        }

        if self.supply_mapping.insert(vara_token_id, supply).is_some() {
            panic!("Mapping already present");
        }
    }

    pub fn remove(&mut self, vara_token_id: ActorId) -> H160 {
        let eth_token_id = self
            .vara_to_eth
            .remove(&vara_token_id)
            .expect("Mapping not found");

        let _ = self
            .eth_to_vara
            .remove(&eth_token_id)
            .expect("Mapping not found");

        let _ = self
            .supply_mapping
            .remove(&vara_token_id)
            .expect("Mapping not found");

        eth_token_id
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

    pub fn get_supply_type(&self, vara_token_id: &ActorId) -> Result<TokenSupply, Error> {
        self.supply_mapping
            .get(vara_token_id)
            .cloned()
            .ok_or(Error::NoCorrespondingVaraAddress)
    }

    pub fn read_state(&self) -> Vec<(ActorId, H160, TokenSupply)> {
        self.vara_to_eth
            .clone()
            .into_iter()
            .map(|(vara_token, eth_token)| {
                let supply = self
                    .get_supply_type(&vara_token)
                    .expect("Should be present due to the TokenMap invariants");
                (vara_token, eth_token, supply)
            })
            .collect()
    }
}
