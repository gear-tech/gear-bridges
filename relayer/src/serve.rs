use crate::{
    proof_storage::{FileSystemProofStorage, ProofStorage},
    prover_interface::{self, FinalProof}, GENESIS_CONFIG,
};

use ethereum_client::Contracts as EthApi;
use gear_rpc_client::GearApi;

pub async fn serve(gear_api: GearApi, eth_api: EthApi) -> anyhow::Result<()> {
    log::info!("Starting relayer");

    let mut proof_storage = FileSystemProofStorage::new("./proof_storage".into());
    let mut eras = Eras::new(None, gear_api.clone())
        .await
        .unwrap_or_else(|err| panic!("Error while creating era storage: {}", err));

    loop {
        let res = main_loop(&gear_api, &eth_api, &mut proof_storage, &mut eras).await;

        if let Err(err) = res {
            log::error!("{}", err);
        }
    }
}

async fn main_loop(gear_api: &GearApi, eth_api: &EthApi, proof_storage: &mut dyn ProofStorage, eras: &mut Eras) -> anyhow::Result<()> {
    log::info!("Syncing authority set id");
    loop {
        let sync_steps = sync_authority_set_id(&gear_api, proof_storage).await?;
        if sync_steps == 0 {
            break;
        } else {
            log::info!("Synced {} authority set ids", sync_steps);
        }
    }
    log::info!("Authority set id is in sync");

    eras.try_seal(proof_storage).await?;

    log::info!("Proving merkle root presense");
    let proof = prove_message_sent(&gear_api, proof_storage).await?;
    log::info!("Proven merkle root presense");

    log::info!("Submitting proof to ethereum");
    submit_proof_to_ethereum(&eth_api, proof).await?; 
    log::info!("Proof submitted to ethereum");

    Ok(())
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
            let proof = prover_interface::prove_genesis(gear_api).await?;
            proof_storage
                .init(proof, GENESIS_CONFIG.authority_set_id)
                .unwrap();

            Ok(1)
        }
        Some((mut proof, latest_proven)) if latest_proven < latest_authority_set_id => {
            for set_id in latest_proven..latest_authority_set_id {
                proof = prover_interface::prove_validator_set_change(gear_api, proof, set_id).await?;
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

struct Eras {
    last_sealed: u64,
    gear_api: GearApi
}

impl Eras {
    pub async fn new(last_sealed: Option<u64>, gear_api: GearApi) -> anyhow::Result<Self> {
        let last_sealed = if let Some(l) = last_sealed {
            l
        } else {
            let latest = gear_api.latest_finalized_block().await?;
            let set_id = gear_api.authority_set_id(latest).await?;
            set_id.max(2) - 1
        };
        
        Ok(Self { last_sealed, gear_api })
    }

    pub async fn try_seal(&mut self, proof_storage: &dyn ProofStorage) -> anyhow::Result<()> {
        let latest = self.gear_api.latest_finalized_block().await?;
        let current_era = self.gear_api.signed_by_authority_set_id(latest).await?;
    
        while self.last_sealed + 2 <= current_era {
            log::info!("Sealing era #{}", self.last_sealed + 1);
            seal_era(&self.gear_api, self.last_sealed + 1, proof_storage).await?;
            log::info!("Sealed era #{}", self.last_sealed + 1);

            self.last_sealed += 1;
        }

        Ok(())
    }
}

async fn seal_era(gear_api: &GearApi, authority_set_id: u64, proof_storage: &dyn ProofStorage) -> anyhow::Result<()> {
    let block = gear_api.find_era_first_block(authority_set_id + 1).await?;
    let inner_proof = proof_storage.get_proof_for_authority_set_id(authority_set_id)?;
    prover_interface::prove_final(gear_api, inner_proof, block).await?;

    Ok(())
}

async fn prove_message_sent(
    gear_api: &GearApi,
    proof_storage: &dyn ProofStorage,
) -> anyhow::Result<FinalProof> {
    let finalized_head = gear_api.latest_finalized_block().await?;
    
    let authority_set_id = gear_api.signed_by_authority_set_id(finalized_head).await?;
    let inner_proof = proof_storage.get_proof_for_authority_set_id(authority_set_id)?;
    let block = gear_api.search_for_authority_set_block(authority_set_id).await?;
    prover_interface::prove_final(gear_api, inner_proof, block).await
}

async fn submit_proof_to_ethereum(
    eth_api: &EthApi,
    proof: FinalProof,
) -> anyhow::Result<()> {
    log::info!("Submitting merkle root {} at gear block {} to ethereum", hex::encode(proof.merkle_root), proof.block_number);

    eth_api.provide_merkle_root(proof.block_number, proof.merkle_root, &proof.proof[..]).await?;

    Ok(())
}
