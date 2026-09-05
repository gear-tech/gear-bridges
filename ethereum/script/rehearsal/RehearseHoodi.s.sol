// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.35;

import {Script, console} from "forge-std/Script.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {VerifierMock} from "src/mocks/VerifierMock.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {IGovernance} from "src/interfaces/IGovernance.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";

/// @notice Step-0 Hoodi rehearsal for the security upgrade (DEPLOYMENT-PLAN.md R1):
/// exercises the new 7-argument initialize (reinitializer(7)), the appended storage
/// layout, the genesis floor and the re-initialization guard on a live testnet
/// proxy, using a mock verifier (the real verifier needs T0 gnark regeneration).
/// Broadcasts ONE deployment batch; all probes are read-only or expected reverts.
contract RehearseHoodiScript is Script {
    function run() public {
        uint256 genesisTip = vm.envOr("REHEARSAL_GENESIS_TIP", uint256(35947462));
        uint256 watermark = vm.envOr("REHEARSAL_WATERMARK", uint256(1000));

        vm.startBroadcast();

        // 1. Mock verifier (accepts everything; the real one lands via T0).
        VerifierMock verifier = new VerifierMock(true);
        console.log("VerifierMock:", address(verifier));

        // 2. New (fixed) MessageQueue implementation.
        MessageQueue impl = new MessageQueue();
        console.log("MessageQueue impl:", address(impl));

        // 3. Fresh proxy: initialize with the 7-arg signature (reinitializer(7)).
        bytes memory initData = abi.encodeCall(
            MessageQueue.initialize,
            (
                IGovernance(msg.sender), // governanceAdmin (rehearsal: EOA occupies the seat)
                IGovernance(msg.sender), // governancePauser
                msg.sender, // emergencyStopAdmin = broadcaster EOA
                _observers(), // emergencyStopObservers
                IVerifier(address(verifier)),
                genesisTip,
                watermark
            )
        );
        ERC1967Proxy proxy = new ERC1967Proxy(address(impl), initData);
        MessageQueue mq = MessageQueue(address(proxy));
        console.log("MessageQueue proxy:", address(proxy));

        // 4. Floor readbacks — the H-2 core assertion.
        require(mq.genesisBlock() == genesisTip, "genesisBlock mismatch");
        require(mq.maxBlockNumber() == genesisTip, "maxBlockNumber mismatch");
        require(mq.nonceWatermark() == watermark, "nonceWatermark mismatch");
        console.log("floors pinned: genesis=%d max=%d watermark=%d", genesisTip, genesisTip, watermark);

        vm.stopBroadcast();

        // 5. Re-initialization guard (read-only eth_call against the live proxy).
        bytes memory reinitData = abi.encodeCall(
            MessageQueue.initialize,
            (
                IGovernance(msg.sender),
                IGovernance(msg.sender),
                msg.sender,
                _observers(),
                IVerifier(address(verifier)),
                genesisTip,
                watermark
            )
        );
        (bool ok, bytes memory ret) = address(proxy).call(reinitData);
        require(!ok, "REHEARSAL FAIL: re-initialization succeeded");
        require(
            bytes4(ret) == bytes4(keccak256("InvalidInitialization()")),
            "unexpected re-init revert"
        );
        console.log("re-init correctly rejected: InvalidInitialization");

        // 6. Genesis floor probe: submission below genesis must revert with
        //    BlockNumberBeforeGenesis BEFORE any proof is touched (dummy proof bytes).
        bytes4 expected = bytes4(keccak256("BlockNumberBeforeGenesis(uint256,uint256)"));
        (ok, ret) = address(mq).call(
            abi.encodeCall(
                MessageQueue.submitMerkleRoot,
                (genesisTip - 1, bytes32(uint256(0xabc)), hex"deadbeef")
            )
        );
        require(!ok, "REHEARSAL FAIL: pre-genesis root accepted");
        require(bytes4(ret) == expected, "unexpected pre-genesis revert selector");
        console.log("genesis floor probe passed: BlockNumberBeforeGenesis");

        console.log("REHEARSAL OK");
    }

    function _observers() internal view returns (address[] memory a) {
        a = new address[](1);
        a[0] = msg.sender;
    }
}
