#![no_std]

// Incorporate code generated based on the IDL file
include!(concat!(env!("OUT_DIR"), "/vft_client.rs"));

/// Allocate configured balance and allowance shards, resuming partial initialization.
pub async fn allocate_shards<R: sails_rs::calls::Remoting + Clone>(
    remoting: R,
    program_id: sails_rs::prelude::ActorId,
    gas_limit: u64,
) -> sails_rs::errors::Result<()> {
    use sails_rs::calls::{Action, Call};
    use traits::VftExtension as _;

    let mut extension = VftExtension::new(remoting);
    while extension
        .allocate_next_balances_shard()
        .with_gas_limit(gas_limit)
        .send_recv(program_id)
        .await?
    {}
    while extension
        .allocate_next_allowances_shard()
        .with_gas_limit(gas_limit)
        .send_recv(program_id)
        .await?
    {}
    Ok(())
}
