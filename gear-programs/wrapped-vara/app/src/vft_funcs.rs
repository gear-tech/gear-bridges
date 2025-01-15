//! Functions to work with `vft-service`.

use sails_rs::prelude::*;
use vft_service::{
    funcs,
    utils::{Error, Result},
    Storage,
};

/// Mint `value` tokens into `to` account.
pub fn mint(to: ActorId, value: U256) -> Result<()> {
    let total_supply = Storage::total_supply();
    let balances = Storage::balances();

    let new_total_supply = total_supply
        .checked_add(value)
        .ok_or(Error::NumericOverflow)?;

    let new_to = funcs::balance_of(balances, to)
        .checked_add(value)
        .ok_or(Error::NumericOverflow)?;

    balances.insert(to, new_to);
    *total_supply = new_total_supply;

    Ok(())
}

/// Burn `value` tokens from `from` address.
pub fn burn(from: ActorId, value: U256) -> Result<()> {
    let total_supply = Storage::total_supply();
    let balances = Storage::balances();

    let new_total_supply = total_supply.checked_sub(value).ok_or(Error::Underflow)?;

    let new_from = funcs::balance_of(balances, from)
        .checked_sub(value)
        .ok_or(Error::InsufficientBalance)?;

    if !new_from.is_zero() {
        balances.insert(from, new_from);
    } else {
        balances.remove(&from);
    }

    *total_supply = new_total_supply;
    Ok(())
}
