// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.35;

import {Script} from "forge-std/Script.sol";
import {IMessageQueue} from "src/interfaces/IMessageQueue.sol";

contract SubmitMerkleRootScript is Script {
    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");

        IMessageQueue messageQueue = IMessageQueue(vm.envAddress("MESSAGE_QUEUE"));
        uint256 blockNumber = vm.envUint("BLOCK_NUMBER");
        bytes32 merkleRoot = vm.envBytes32("MERKLE_ROOT");
        bytes memory plonky2Proof = vm.envBytes("PLONKY2_PROOF");

        vm.startBroadcast(privateKey);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, plonky2Proof);

        vm.stopBroadcast();
    }
}
