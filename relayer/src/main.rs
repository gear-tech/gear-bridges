extern crate pretty_env_logger;

use gear_rpc_client::GearApi;
use prover::{message_sent::MessageSent, next_validator_set::NextValidatorSet};

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let api = GearApi::new().await;

    let block = api.latest_finalized_block().await;

    let proof = MessageSent {
        block_finality: api.fetch_finality_proof(block).await,
        inclusion_proof: api.fetch_sent_message_merkle_proof(block).await,
    }
    .prove();

    panic!("verified: {}", proof.verify());

    let now = std::time::Instant::now();

    let proof = NextValidatorSet {
        current_epoch_block_finality: api.fetch_finality_proof(block).await,
        next_validator_set_inclusion_proof: api.fetch_next_authorities_merkle_proof(block).await,
    }
    .prove();

    panic!(
        "verified: {} in {}ms",
        proof.verify(),
        now.elapsed().as_millis()
    );
}
