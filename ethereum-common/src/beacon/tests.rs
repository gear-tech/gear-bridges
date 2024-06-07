use super::*;
use hex_literal::hex;

const ETHEREUM_9_230_177: &[u8; 133_287] = include_bytes!("./chain-data/ethereum-9_230_177.json");
const SEPOLIA_5_151_035: &[u8; 10_722] = include_bytes!("./chain-data/sepolia-5_151_035.json");
const HOLESKY_1_820_966: &[u8; 130_166] = include_bytes!("./chain-data/holesky-1_820_966.json");

#[test]
fn sepolia_slot_5_151_035() {
    // curl -X 'GET' 'http://{Sepolia Beacon API URL}/eth/v1/beacon/headers/5151035' -H 'accept: application/json'
    // https://sepolia.beaconcha.in/slot/5151035
    // https://sepolia.etherscan.io/block/6040894

    let block_root: Hash256 =
        hex!("4017f0180aa9acb2601ab3ce066081a41661b7f85683901f1cadaec7b6198059").into();
    let beacon_header = BlockHeader {
        slot: 5_151_035,
        proposer_index: 1_959,
        parent_root: hex!("64331200f34a330e14d5673ffa353f1348826d75830697f6b0750ba42bcabc5e")
            .into(),
        state_root: hex!("a605d19e8b3e188fb6df208784d22ce820bbf2f91a32ac92e7d4f99d91ab04bb").into(),
        body_root: hex!("1d63bb6d5e875871045cfb7b4776b18bc4bd3c8372950ee92e3e5458c3a187f2").into(),
    };

    assert_eq!(block_root, beacon_header.tree_hash_root());

    let block_body: BlockBody = serde_json::from_slice(SEPOLIA_5_151_035.as_ref()).unwrap();

    assert_eq!(beacon_header.body_root, block_body.tree_hash_root());

    let block = Block {
        slot: beacon_header.slot,
        proposer_index: beacon_header.proposer_index,
        parent_root: beacon_header.parent_root,
        state_root: beacon_header.state_root,
        body: block_body,
    };

    assert_eq!(block_root, block.tree_hash_root());
}

#[test]
fn ethereum_slot_9_230_177() {
    // curl -X 'GET' 'https://www.lightclientdata.org/eth/v1/beacon/headers/9230177' -H 'accept: application/json'
    // https://beaconcha.in/slot/9230177
    // https://etherscan.io/block/20025266

    let block_root: Hash256 =
        hex!("8a71cd9567f3ef85fc5e0dae0fcc6acd707f87b63e9a3174d4ec80395ab31763").into();
    let beacon_header = BlockHeader {
        slot: 9_230_177,
        proposer_index: 404_728,
        parent_root: hex!("c00156add0e86e806b98c9a6367942d451770febb9a3dfcc79f6364863b749fa")
            .into(),
        state_root: hex!("01556da441fbe34b5c60c5192790358456228733f3ee874c32920a400771d1c1").into(),
        body_root: hex!("d0b99a73709763a46384de415ced4e806cb00ea0b54b61ddc69b5e8ac1bd8b4a").into(),
    };

    assert_eq!(block_root, beacon_header.tree_hash_root());

    let block_body: BlockBody = serde_json::from_slice(ETHEREUM_9_230_177.as_ref()).unwrap();

    assert_eq!(beacon_header.body_root, block_body.tree_hash_root());

    let block = Block {
        slot: beacon_header.slot,
        proposer_index: beacon_header.proposer_index,
        parent_root: beacon_header.parent_root,
        state_root: beacon_header.state_root,
        body: block_body,
    };

    assert_eq!(block_root, block.tree_hash_root());
}

#[test]
fn holesky_slot_1_820_966() {
    // curl -X 'GET' 'http://{Holesky Beacon API URL}/eth/v1/beacon/headers/1820966' -H 'accept: application/json'
    // https://holesky.beaconcha.in/slot/1820966
    // https://holesky.etherscan.io/block/1688645

    let block_root: Hash256 =
        hex!("f26e90bf3731062e91354e205f2b4720ab5340b2925162e9233e9480ca09b8af").into();
    let beacon_header = BlockHeader {
        slot: 1_820_966,
        proposer_index: 1_634_770,
        parent_root: hex!("64da2bfc359d4559c319d0b48bbbdad2a4fcd4b83f384467e0b1f3575ed0a253")
            .into(),
        state_root: hex!("de17bcaecddcb0d688cace7741b0bb1aee83cb843d5f020d9b9050bfa7bc4c3f").into(),
        body_root: hex!("347b3f6235c17a38859280d909a01b249ef206f2884eeeffb210ad54646a4dac").into(),
    };

    assert_eq!(block_root, beacon_header.tree_hash_root());

    let block_body: BlockBody = serde_json::from_slice(HOLESKY_1_820_966.as_ref()).unwrap();

    assert_eq!(beacon_header.body_root, block_body.tree_hash_root());

    let block = Block {
        slot: beacon_header.slot,
        proposer_index: beacon_header.proposer_index,
        parent_root: beacon_header.parent_root,
        state_root: beacon_header.state_root,
        body: block_body,
    };

    assert_eq!(block_root, block.tree_hash_root());
}
