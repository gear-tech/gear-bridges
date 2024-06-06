// This file is part of Gear.

// Copyright (C) 2024 Gear Technologies Inc.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

use super::*;
use hex_literal::hex;

const ETHEREUM_9_230_177: &[u8; 133_287] = include_bytes!("./ethereum-9_230_177.json");
const SEPOLIA_5_151_035: &[u8; 10_722] = include_bytes!("./sepolia-5_151_035.json");

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
}
