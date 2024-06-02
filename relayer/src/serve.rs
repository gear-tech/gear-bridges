use crate::{
    proof_storage::{FileSystemProofStorage, ProofStorage},
    prover_interface::{self, FinalProof}, EthereumArgs, ServeArgs, GENESIS_CONFIG,
};

use ethereum_client::Contracts as EthApi;
use gear_rpc_client::GearApi;
use prover::proving::ExportedProofWithCircuitData;

pub async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    let ServeArgs {
        vara_endpoint,
        ethereum_args,
    } = args;

    let gear_api = GearApi::new(&vara_endpoint.vara_endpoint).await.unwrap();
    let eth_api = {
        let EthereumArgs {
            eth_endpoint,
            fee_payer,
            relayer_address,
            mq_address,
        } = ethereum_args;

        EthApi::new(
            &eth_endpoint,
            &mq_address,
            &relayer_address,
            fee_payer.as_ref().map(|s| s.as_str()),
        )
        .unwrap_or_else(|err| panic!("Error while creating ethereum client: {}", err))
    };

    let mut proof_storage = FileSystemProofStorage::new("./proof_storage".into());

    loop {
        loop {
            let sync_steps = sync_authority_set_id(&gear_api, &mut proof_storage).await?;
            if sync_steps == 0 {
                break;
            }
        }

        let proof = prove_message_sent(&gear_api, &proof_storage).await?;
        submit_proof_to_ethereum(&eth_api, proof).await?;
    }
}

type SyncStepCount = usize;

async fn sync_authority_set_id(
    gear_api: &GearApi,
    proof_storage: &mut dyn ProofStorage,
) -> anyhow::Result<SyncStepCount> {
    let finalized_head = gear_api.latest_finalized_block().await.unwrap();
    let latest_authority_set_id = gear_api.authority_set_id(finalized_head).await.unwrap();

    let latest_proven_authority_set_id = proof_storage.get_latest_proof();
    match latest_proven_authority_set_id {
        None => {
            let proof = prover_interface::prove_genesis(gear_api).await;
            proof_storage
                .init(proof, GENESIS_CONFIG.authority_set_id)
                .unwrap();

            Ok(1)
        }
        Some((mut proof, latest_proven)) if latest_proven < latest_authority_set_id => {
            for set_id in latest_proven..latest_authority_set_id {
                proof = prover_interface::prove_validator_set_change(gear_api, proof, set_id).await;
            }

            let step_count = latest_authority_set_id - latest_proven;
            Ok(step_count as usize)
        }
        Some((_, latest_proven)) if latest_proven == latest_authority_set_id => Ok(0),
        Some((_, latest_proven)) => unreachable!(
            "Invalid state of proof storage detected: latest stored authority set id = {} but latest authority set id on VARA = {}", 
            latest_proven, 
            latest_authority_set_id
        ),
    }
}

async fn prove_message_sent(
    gear_api: &GearApi,
    proof_storage: &dyn ProofStorage,
) -> anyhow::Result<FinalProof> {
    let finalized_head = gear_api.latest_finalized_block().await?;
    
    // TODO: if we are at the start of era we must submit proof for the first block
    // of this era(as it's the latest block in which all the messages from this era are available)

    let authority_set_id = gear_api.signed_by_authority_set_id(finalized_head).await?;
    let inner_proof = proof_storage.get_proof_for_authority_set_id(authority_set_id)?;
    Ok(prover_interface::prove_final(gear_api, inner_proof, authority_set_id).await)
}

async fn submit_proof_to_ethereum(
    eth_api: &EthApi,
    proof: FinalProof,
) -> anyhow::Result<()> {
    todo!()
}
