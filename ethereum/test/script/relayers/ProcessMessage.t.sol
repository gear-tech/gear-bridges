// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Test} from "forge-std/Test.sol";
import {ProcessMessageScript} from "script/relayers/ProcessMessage.s.sol";
import {VaraMessage} from "src/interfaces/IMessageQueue.sol";
import {Base} from "test/Base.sol";

contract ProcessMessageScriptTest is Test, Base {
    function test_Relaying() public {
        deployBridgeFromConstants();

        VaraMessage memory message =
            VaraMessage({nonce: 1, source: bytes32(uint256(2)), destination: address(messageHandlerMock), payload: ""});
        bytes32 messageHash =
            keccak256(abi.encodePacked(message.nonce, message.source, message.destination, message.payload));
        bytes32 merkleRoot = keccak256(abi.encodePacked(bytes32(0), messageHash));

        // forge-lint: disable-start(unsafe-cheatcode)
        vm.setEnv("PRIVATE_KEY", "1");
        vm.setEnv("MESSAGE_QUEUE", vm.toString(address(messageQueue)));
        vm.setEnv("BLOCK_NUMBER", "1");
        vm.setEnv("TOTAL_LEAVES", "1");
        vm.setEnv("LEAF_INDEX", "0");
        vm.setEnv("NONCE", "1");
        vm.setEnv("SOURCE", vm.toString(message.source));
        vm.setEnv("DESTINATION", vm.toString(message.destination));
        vm.setEnv("PAYLOAD", "0x");
        vm.setEnv("MERKLE_PROOF", vm.toString(bytes32(0)));
        // forge-lint: disable-end(unsafe-cheatcode)

        vm.prank(address(this));
        messageQueue.submitMerkleRoot(1, merkleRoot, "");
        vm.warp(block.timestamp + 5 minutes);

        ProcessMessageScript relayingScript = new ProcessMessageScript();
        relayingScript.run();

        assertTrue(messageQueue.isProcessed(message.nonce));
    }
}
