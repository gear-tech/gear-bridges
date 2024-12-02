use alloy::rpc::types::TransactionReceipt;
use alloy_rlp::Encodable;
use erc20_relay_client::{BlockInclusionProof, EthToVaraEvent};
use ethereum_common::{
    beacon::light::Block,
    utils::{self as eth_utils, BeaconBlockHeaderResponse, BeaconBlockResponse, MerkleProof},
};
use serde::Deserialize;
pub const HOLESKY_RECEIPTS_2_498_456: &[u8; 160_144] =
    include_bytes!("./data/holesky-receipts-2_498_456.json");
pub const HOLESKY_BLOCK_2_498_456: &[u8; 235_397] =
    include_bytes!("./data/holesky-block-2_498_456.json");
pub const HOLESKY_HEADER_2_498_457: &[u8; 670] = include_bytes!("./data/holesky-header-2_498_457.json");
pub const HOLESKY_HEADER_2_498_458: &[u8; 669] = include_bytes!("./data/holesky-header-2_498_458.json");
pub const HOLESKY_HEADER_2_498_459: &[u8; 670] = include_bytes!("./data/holesky-header-2_498_459.json");
pub const HOLESKY_HEADER_2_498_460: &[u8; 670] = include_bytes!("./data/holesky-header-2_498_460.json");
pub const HOLESKY_HEADER_2_498_461: &[u8; 670] = include_bytes!("./data/holesky-header-2_498_461.json");
pub const HOLESKY_HEADER_2_498_462: &[u8; 669] = include_bytes!("./data/holesky-header-2_498_462.json");
pub const HOLESKY_HEADER_2_498_463: &[u8; 670] = include_bytes!("./data/holesky-header-2_498_463.json");
pub const HOLESKY_HEADER_2_498_464: &[u8; 669] = include_bytes!("./data/holesky-header-2_498_464.json");

#[derive(Deserialize)]
pub struct Receipts {
    result: Vec<TransactionReceipt>,
}

pub fn event() -> EthToVaraEvent {
    // tx 0x180cd2328df9c4356adc77e19e33c5aa2d5395f1b52e70d22c25070a04f16691
    let tx_index = 15;

    let receipts: Receipts = serde_json::from_slice(HOLESKY_RECEIPTS_2_498_456.as_ref()).unwrap();
    let receipts = receipts
        .result
        .iter()
        .map(|tx_receipt| {
            let receipt = tx_receipt.as_ref();

            tx_receipt
                .transaction_index
                .map(|i| (i, eth_utils::map_receipt_envelope(receipt)))
        })
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();

    let block: Block = {
        let response: BeaconBlockResponse =
            serde_json::from_slice(HOLESKY_BLOCK_2_498_456.as_ref()).unwrap();

        response.data.message.into()
    };
    let headers = vec![
        {
            let response: BeaconBlockHeaderResponse =
                serde_json::from_slice(HOLESKY_HEADER_2_498_457.as_ref()).unwrap();

            response.data.header.message
        },
        {
            let response: BeaconBlockHeaderResponse =
                serde_json::from_slice(HOLESKY_HEADER_2_498_458.as_ref()).unwrap();

            response.data.header.message
        },
        {
            let response: BeaconBlockHeaderResponse =
                serde_json::from_slice(HOLESKY_HEADER_2_498_459.as_ref()).unwrap();

            response.data.header.message
        },
        {
            let response: BeaconBlockHeaderResponse =
                serde_json::from_slice(HOLESKY_HEADER_2_498_460.as_ref()).unwrap();

            response.data.header.message
        },
        {
            let response: BeaconBlockHeaderResponse =
                serde_json::from_slice(HOLESKY_HEADER_2_498_461.as_ref()).unwrap();

            response.data.header.message
        },
        {
            let response: BeaconBlockHeaderResponse =
                serde_json::from_slice(HOLESKY_HEADER_2_498_462.as_ref()).unwrap();

            response.data.header.message
        },
        {
            let response: BeaconBlockHeaderResponse =
                serde_json::from_slice(HOLESKY_HEADER_2_498_463.as_ref()).unwrap();

            response.data.header.message
        },
        {
            let response: BeaconBlockHeaderResponse =
                serde_json::from_slice(HOLESKY_HEADER_2_498_464.as_ref()).unwrap();

            response.data.header.message
        },
    ];

    let MerkleProof { proof, receipt } =
        eth_utils::generate_merkle_proof(tx_index, &receipts[..]).unwrap();

    let mut receipt_rlp = Vec::with_capacity(Encodable::length(&receipt));
    Encodable::encode(&receipt, &mut receipt_rlp);
    EthToVaraEvent {
        proof_block: BlockInclusionProof {
            block: block.clone(),
            headers: headers.clone(),
        },
        proof: proof.clone(),
        transaction_index: tx_index,
        receipt_rlp,
    }
}
