use crate::{
    proof_storage::{FileSystemProofStorage, ProofStorage},
    prover_interface::{self, FinalProof}, EthereumArgs, ServeArgs, GENESIS_CONFIG,
};

use ethereum_client::Contracts as EthApi;
use gear_rpc_client::GearApi;

pub async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    log::info!("Starting relayer...");
    
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
            fee_payer.as_deref(),
        )
        .unwrap_or_else(|err| panic!("Error while creating ethereum client: {}", err))
    };

    let mut proof_storage = FileSystemProofStorage::new("./proof_storage".into());

    loop {
        let res: anyhow::Result<()> = {
            log::info!("Syncing authority set id...");
            loop {
                let sync_steps = sync_authority_set_id(&gear_api, &mut proof_storage).await?;
                if sync_steps == 0 {
                    break;
                } else {
                    log::info!("Synced {} authority set ids", sync_steps);
                }
            }
            log::info!("Authority set id is in sync");

            log::info!("Proving...");
            let proof = prove_message_sent(&gear_api, &proof_storage).await?;
            log::info!("Proven");

            log::info!("Submitting proof to ethereum...");
            submit_proof_to_ethereum(&eth_api, proof).await?; 
            log::info!("Proof submitted to ethereum");

            Ok(())
        };

        if let Err(err) = res {
            log::error!("{}", err);
        }
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
                proof_storage.update(proof.proof.clone())?;
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
    eth_api.provide_merkle_root(proof.block_number, proof.merkle_root, &proof.proof[..]).await?;

    Ok(())
}
