use gear_rpc_client::GearApi;
use prover::BlockJustification;

#[tokio::main]
async fn main() {
    let api = GearApi::new().await;
    let block = api.latest_finalized_block().await;
    let justification = api.fetch_justification(block).await;

    let justification = BlockJustification {
        pre_commits: justification.pre_commits.into_iter().take(3).collect(),
        msg: justification.msg,
    };
    justification.prove();
}
