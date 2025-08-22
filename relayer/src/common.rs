use std::{sync::Arc, time::Duration};

use alloy::transports::{RpcError, TransportErrorKind};
use prover::proving::GenesisConfig;

use crate::{
    proof_storage::ProofStorage,
    prover_interface::{self, FinalProof},
};
use parity_scale_codec::Encode;
use ethereum_client::{EthApi, TxHash};
use gear_rpc_client::GearApi;

pub(crate) type SyncStepCount = usize;

pub(crate) async fn sync_authority_set_id(
    gear_api: &GearApi,
    proof_storage: &Arc<dyn ProofStorage>,
    genesis_config: GenesisConfig,
    latest_authority_set_id: u64,
    latest_proven_authority_set_id: Option<u64>,
) -> anyhow::Result<SyncStepCount> {
    log::trace!("pub(crate) async fn sync_authority_set_id( genesis_config = {genesis_config:?}");

    let latest_proven = genesis_config.authority_set_id + 1;
    // let Some(latest_proven) = latest_proven_authority_set_id else {
        log::trace!("pub(crate) async fn sync_authority_set_id( latest_authority_set_id = {latest_authority_set_id}");

        if latest_authority_set_id <= genesis_config.authority_set_id {
            log::warn!(
                "Network haven't reached authority set id #(GENESIS + 1). \
                Current authority set id: {}, genesis: {}",
                latest_authority_set_id,
                genesis_config.authority_set_id,
            );
            return Ok(0);
        }

        // let proof = prover_interface::prove_genesis(gear_api, genesis_config).await?;

        // log::trace!("Init storage with proofs");
        // proof_storage
        //     .init(proof, genesis_config.authority_set_id)
        //     .await
        //     .unwrap();
        let (block, current_epoch_block_finality) = gear_api
            .fetch_finality_proof_for_session(genesis_config.authority_set_id)
            .await?;

        let next_validator_set_inclusion_proof = gear_api
            .fetch_next_session_keys_inclusion_proof(block)
            .await?;

        log::info!("genesis block = {}, current_epoch_block_finality = {}, next_validator_set_inclusion_proof = {}",
            hex::encode(block.encode()),
            hex::encode(current_epoch_block_finality.encode()),
            hex::encode(next_validator_set_inclusion_proof.encode()),
        );
        log::info!("==================================================================================");

    //     return Ok(1);
    // };

    log::trace!("pub(crate) async fn sync_authority_set_id( latest_proven = {latest_proven}");

    if latest_proven < genesis_config.authority_set_id + 1 {
        panic!(
            "Invalid state of proof storage detected: \
            latest proven authority set id = {} \
            but genesis = {}. \
            Clean proof storage state and restart the relayer.",
            latest_proven, genesis_config.authority_set_id,
        );
    }

    if latest_proven < latest_authority_set_id {
        // let mut proof = proof_storage
        //     .get_proof_for_authority_set_id(latest_proven)
        //     .await?;

        for set_id in latest_proven..latest_authority_set_id {
            log::info!(
                "Proving authority set change {} -> {}",
                set_id,
                set_id + 1
            );
            let (block, current_epoch_block_finality) = gear_api
                .fetch_finality_proof_for_session(set_id)
                .await?;

            let next_validator_set_inclusion_proof = gear_api
                .fetch_next_session_keys_inclusion_proof(block)
                .await?;

            log::info!("genesis block = {}, current_epoch_block_finality = {}, next_validator_set_inclusion_proof = {}",
                hex::encode(block.encode()),
                hex::encode(current_epoch_block_finality.encode()),
                hex::encode(next_validator_set_inclusion_proof.encode()),
            );
            log::info!("==================================================================================");
            // proof = prover_interface::prove_validator_set_change(gear_api, proof, set_id).await?;
            // proof_storage
            //     .update(proof.proof.clone(), set_id + 1)
            //     .await?;
        }

        let step_count = latest_authority_set_id - latest_proven;
        return Ok(step_count as usize);
    }

    if latest_proven == latest_authority_set_id {
        return Ok(0);
    }

    panic!(
        "Invalid state of proof storage detected: \
        latest proven authority set id = {latest_proven} \
        but latest authority set id on VARA = {latest_authority_set_id}. \
        Clean proof storage state and restart the relayer."
    )
}

pub(crate) async fn submit_merkle_root_to_ethereum(
    eth_api: &EthApi,
    proof: FinalProof,
) -> Result<TxHash, ethereum_client::Error> {
    log::info!(
        "Submitting merkle root {} at block #{} to ethereum",
        hex::encode(proof.merkle_root),
        proof.block_number
    );

    let tx_hash = eth_api
        .provide_merkle_root(proof.block_number, proof.merkle_root, proof.proof)
        .await?;

    Ok(tx_hash)
}

pub(crate) fn is_rpc_transport_error_recoverable(err: &RpcError<TransportErrorKind>) -> bool {
    match err {
        RpcError::Transport(transport) => match transport {
            TransportErrorKind::MissingBatchResponse(_) => true,
            TransportErrorKind::BackendGone => true,
            TransportErrorKind::PubsubUnavailable => false,
            TransportErrorKind::HttpError(_) => false,
            TransportErrorKind::Custom(_) => false,
            _ => false,
        },
        _ => false,
    }
}

pub(crate) fn is_transport_error_recoverable(err: &anyhow::Error) -> bool {
    if let Some(ethereum_client::Error::ErrorInHTTPTransport(err)) =
        err.downcast_ref::<ethereum_client::Error>()
    {
        return is_rpc_transport_error_recoverable(err);
    }

    // raw provider calls return `RpcError`.
    if let Some(err) = err.downcast_ref::<RpcError<TransportErrorKind>>() {
        return is_rpc_transport_error_recoverable(err);
    }

    // sails calls return gclient error which can contain subxt error with rpc transport error
    if let Some(gclient::Error::Subxt(err)) = err.downcast_ref::<gclient::Error>() {
        if err.is_disconnected_will_reconnect() {
            return true;
        }
        if let subxt::Error::Rpc(rpc) = &**err {
            match rpc {
                subxt::error::RpcError::SubscriptionDropped => return true,
                subxt::error::RpcError::DisconnectedWillReconnect(_) => return true,
                _ => (),
            }
        }
    }

    false
}

pub struct BlockRange {
    pub from: u64,
    pub to: u64,
}

/// Creates BlockRange that does not exceed the maximum allowed range (i.e. to avoid
/// an error 'server returned an error response: error code -32602: query exceeds max block range 100000').
pub fn create_range(from: Option<u64>, latest: u64) -> BlockRange {
    let Some(from) = from else {
        return BlockRange {
            from: latest,
            to: latest,
        };
    };

    let block_to_max = from + 99_999;
    let block_to = if block_to_max > latest {
        latest
    } else {
        block_to_max
    };

    BlockRange { from, to: block_to }
}

pub const MAX_RETRIES: u32 = 10;
pub const BASE_RETRY_DELAY: Duration = Duration::from_secs(10);
