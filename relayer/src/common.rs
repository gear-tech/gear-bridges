use alloy::transports::{RpcError, TransportErrorKind};
use prover::proving::GenesisConfig;

use crate::{
    proof_storage::ProofStorage,
    prover_interface::{self, FinalProof},
};

use ethereum_client::{EthApi, TxHash};
use gear_rpc_client::GearApi;

pub(crate) type SyncStepCount = usize;

pub(crate) async fn sync_authority_set_id(
    gear_api: &GearApi,
    proof_storage: &mut dyn ProofStorage,
    genesis_config: GenesisConfig,
    latest_authority_set_id: u64,
    latest_proven_authority_set_id: Option<u64>,
) -> anyhow::Result<SyncStepCount> {
    let Some(latest_proven) = latest_proven_authority_set_id else {
        if latest_authority_set_id <= genesis_config.authority_set_id {
            log::warn!(
                "Network haven't reached authority set id #(GENESIS + 1). \
                Current authority set id: {}, genesis: {}",
                latest_authority_set_id,
                genesis_config.authority_set_id,
            );
            return Ok(0);
        }

        let proof = prover_interface::prove_genesis(gear_api, genesis_config).await?;
        proof_storage
            .init(proof, genesis_config.authority_set_id)
            .unwrap();

        return Ok(1);
    };

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
        let mut proof = proof_storage.get_proof_for_authority_set_id(latest_proven)?;

        for set_id in latest_proven..latest_authority_set_id {
            proof = prover_interface::prove_validator_set_change(gear_api, proof, set_id).await?;
            proof_storage.update(proof.proof.clone(), set_id + 1)?;
        }

        let step_count = latest_authority_set_id - latest_proven;
        return Ok(step_count as usize);
    }

    if latest_proven == latest_authority_set_id {
        return Ok(0);
    }

    panic!(
        "Invalid state of proof storage detected: \
        latest proven authority set id = {} \
        but latest authority set id on VARA = {}. \
        Clean proof storage state and restart the relayer.",
        latest_proven, latest_authority_set_id
    )
}

pub(crate) async fn submit_merkle_root_to_ethereum(
    eth_api: &EthApi,
    proof: FinalProof,
) -> anyhow::Result<TxHash> {
    log::info!(
        "Submitting merkle root {} at gear block {} to ethereum",
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
    if let Some(err) = err.downcast_ref::<ethereum_client::Error>() {
        match err {
            ethereum_client::Error::ErrorInHTTPTransport(err) => {
                return is_rpc_transport_error_recoverable(err)
            }
            _ => (),
        }
    }

    // raw provider calls return `RpcError`.
    if let Some(err) = err.downcast_ref::<RpcError<TransportErrorKind>>() {
        return is_rpc_transport_error_recoverable(err);
    }

    // sails calls return gclient error which can contain subxt error with rpc transport error
    if let Some(err) = err.downcast_ref::<gclient::Error>() {
        match err {
            gclient::Error::Subxt(err) => {
                if err.is_disconnected_will_reconnect() {
                    return true;
                }
                match err {
                    subxt::Error::Rpc(rpc) => match rpc {
                        subxt::error::RpcError::SubscriptionDropped => return true,
                        subxt::error::RpcError::DisconnectedWillReconnect(_) => return true,
                        _ => (),
                    },
                    _ => (),
                }
            }

            _ => (),
        }
    }

    false
}
