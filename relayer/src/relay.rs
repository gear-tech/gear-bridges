use crate::{EthereumArgs, RelayArgs};

use ethereum_client::Contracts as EthApi;
use gear_rpc_client::GearApi;

pub async fn relay(args: RelayArgs) -> anyhow::Result<()> {
    let gear_api = GearApi::new(&args.vara_endpoint.vara_endpoint)
        .await
        .unwrap();

    let eth_api = {
        let EthereumArgs {
            eth_endpoint,
            fee_payer,
            relayer_address,
            mq_address,
        } = args.ethereum_args;

        EthApi::new(
            &eth_endpoint,
            &mq_address,
            &relayer_address,
            fee_payer.as_deref(),
        )
        .unwrap_or_else(|err| panic!("Error while creating ethereum client: {}", err))
    };

    // sender: ALICE
    // receiver: 0x000...00011
    // paylod: 0x11
    // nonce: 1
    // 0xbcf2aa76c36358f3913a1d701a2e9f9622d214348613f0059139b93e58edc6c2
    let block = gear_api.block_number_to_hash(715).await.unwrap();
    let message =
        hex::decode("bcf2aa76c36358f3913a1d701a2e9f9622d214348613f0059139b93e58edc6c2").unwrap();
    let message: [u8; 32] = message.try_into().unwrap();
    let message_hash = primitive_types::H256::from(message);

    let proof = gear_api
        .fetch_message_inclusion_merkle_proof(block, message_hash)
        .await
        .unwrap();

    // ALICE
    let sender =
        hex::decode("d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d").unwrap();
    let sender: [u8; 32] = sender.try_into().unwrap();

    eth_api
        .provide_content_message(
            block.0,
            proof.num_leaves as u32,
            proof.leaf_index as u32,
            1u128,
            sender,
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1],
            &[0x11][..],
            proof.proof,
        )
        .await?;

    Ok(())
}
