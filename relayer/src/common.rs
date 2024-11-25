use prometheus::{Gauge, IntGauge};
use prover::proving::GenesisConfig;
use utils_prometheus::impl_metered_service;

use crate::{
    proof_storage::ProofStorage,
    prover_interface::{self, FinalProof},
};

use ethereum_client::{EthApi, TxHash};
use gear_rpc_client::GearApi;

impl_metered_service! {
    pub(crate) struct Metrics {
        pub latest_proven_era: IntGauge = IntGauge::new(
            "merkle_root_relayer_latest_proven_era",
            "Latest proven era number",
        ),
        pub latest_observed_gear_era: IntGauge = IntGauge::new(
            "merkle_root_relayer_latest_observed_gear_era",
            "Latest era number observed by relayer",
        ),
        pub fee_payer_balance: Gauge = Gauge::new(
            "merkle_root_relayer_fee_payer_balance",
            "Transaction fee payer balance",
        )
    }
}

pub(crate) type SyncStepCount = usize;

pub(crate) async fn sync_authority_set_id(
    gear_api: &GearApi,
    proof_storage: &mut dyn ProofStorage,
    genesis_config: GenesisConfig,
    metrics: Option<&Metrics>,
) -> anyhow::Result<SyncStepCount> {
    let finalized_head = gear_api.latest_finalized_block().await.unwrap();
    let latest_authority_set_id = gear_api.authority_set_id(finalized_head).await.unwrap();

    if let Some(metrics) = metrics {
        metrics
            .latest_observed_gear_era
            .set(latest_authority_set_id as i64);
    }

    let latest_proven_authority_set_id = proof_storage.get_latest_authority_set_id();

    if let Some(&latest_proven) = latest_proven_authority_set_id.as_ref() {
        if let Some(metrics) = metrics {
            metrics.latest_proven_era.set(latest_proven as i64);
        }
    }

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

pub(crate) async fn submit_proof_to_ethereum(
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
