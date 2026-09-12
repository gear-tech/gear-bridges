// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Test} from "forge-std/Test.sol";
import {SubmitMerkleRootScript} from "script/relayers/SubmitMerkleRoot.s.sol";
import {Base} from "test/Base.sol";

contract SubmitMerkleRootScriptTest is Test, Base {
    function test_Relaying() public {
        deployBridgeFromConstants();

        // forge-lint: disable-start(unsafe-cheatcode)
        vm.setEnv("PRIVATE_KEY", "1");
        vm.setEnv("MESSAGE_QUEUE", vm.toString(address(messageQueue)));
        vm.setEnv("BLOCK_NUMBER", "1");
        vm.setEnv("MERKLE_ROOT", vm.toString(bytes32(uint256(1))));
        vm.setEnv("PLONKY2_PROOF", "0x");
        // forge-lint: disable-end(unsafe-cheatcode)

        SubmitMerkleRootScript relayingScript = new SubmitMerkleRootScript();
        relayingScript.run();

        assertEq(messageQueue.getMerkleRoot(1), bytes32(uint256(1)));
    }
}
