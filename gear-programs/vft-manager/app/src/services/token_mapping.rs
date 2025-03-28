use collections::HashMap;
use sails_rs::prelude::*;

use super::{error::Error, TokenSupply};

/// Mapping between `VFT` and `ERC20` tokens.
#[derive(Debug, Default)]
pub struct TokenMap {
    /// Mapping from `VFT` token addresses to `ERC20` token addresses and the [TokenSupply] type.
    vara_to_eth: HashMap<ActorId, (H160, TokenSupply)>,
    /// Mapping from `ERC20` token addresses to `VFT` token addresses.
    eth_to_vara: HashMap<H160, ActorId>,
}

impl TokenMap {
    /// Insert token pair into the map.
    ///
    /// Will return error if either `vara_token_id` or `eth_token_id` is already present in the map.
    pub fn insert(&mut self, vara_token_id: ActorId, eth_token_id: H160, supply: TokenSupply) {
        if self
            .vara_to_eth
            .insert(vara_token_id, (eth_token_id, supply))
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
    }

    /// Remove token pair from map.
    ///
    /// Will return error if `vara_token_id` don't correspond to the already existing mapping.
    pub fn remove(&mut self, vara_token_id: ActorId) -> H160 {
        let (eth_token_id, _suply) = self
            .vara_to_eth
            .remove(&vara_token_id)
            .expect("Mapping not found");

        let _ = self
            .eth_to_vara
            .remove(&eth_token_id)
            .expect("Mapping not found");

        eth_token_id
    }

    /// Get `ERC20` token address by `VFT` token address.
    ///
    /// Will return error if mapping isn't found.
    pub fn get_eth_token_id(&self, vara_token_id: &ActorId) -> Result<H160, Error> {
        self.vara_to_eth
            .get(vara_token_id)
            .cloned()
            .map(|(eth_token_id, _supply)| eth_token_id)
            .ok_or(Error::NoCorrespondingEthAddress)
    }

    /// Get `VFT` token address by `ERC20` token address.
    ///
    /// Will return error if mapping isn't found.
    pub fn get_vara_token_id(&self, eth_token_id: &H160) -> Result<ActorId, Error> {
        self.eth_to_vara
            .get(eth_token_id)
            .cloned()
            .ok_or(Error::NoCorrespondingVaraAddress)
    }

    /// Get token pair [TokenSupply] type by `VFT` token address.
    ///
    /// Will return error if mapping isn't found.
    pub fn get_supply_type(&self, vara_token_id: &ActorId) -> Result<TokenSupply, Error> {
        self.vara_to_eth
            .get(vara_token_id)
            .cloned()
            .map(|(_eth_token_id, supply)| supply)
            .ok_or(Error::NoCorrespondingVaraAddress)
    }

    /// Read state of the token mapping. Will return all entries present in the mapping.
    pub fn read_state(&self) -> Vec<(ActorId, H160, TokenSupply)> {
        self.vara_to_eth
            .clone()
            .into_iter()
            .map(|(vara_token, (eth_token, supply))| {
                (vara_token, eth_token, supply)
            })
            .collect()
    }
}
