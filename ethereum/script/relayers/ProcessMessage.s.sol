// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.35;

import {Script} from "forge-std/Script.sol";
import {IMessageQueue, VaraMessage} from "src/interfaces/IMessageQueue.sol";

contract ProcessMessageScript is Script {
    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");

        IMessageQueue messageQueue = IMessageQueue(vm.envAddress("MESSAGE_QUEUE"));
        uint256 blockNumber = vm.envUint("BLOCK_NUMBER");
        uint256 totalLeaves = vm.envUint("TOTAL_LEAVES");
        uint256 leafIndex = vm.envUint("LEAF_INDEX");
        VaraMessage memory message = VaraMessage({
            nonce: vm.envUint("NONCE"),
            source: vm.envBytes32("SOURCE"),
            destination: vm.envAddress("DESTINATION"),
            payload: vm.envBytes("PAYLOAD")
        });
        bytes32[] memory proof = vm.envBytes32("MERKLE_PROOF", ",");

        vm.startBroadcast(privateKey);

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof);

        vm.stopBroadcast();
    }
}
