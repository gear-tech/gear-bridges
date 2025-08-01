// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test} from "forge-std/Test.sol";
import {Base} from "./Base.sol";
import {
    GovernanceConstants,
    IGovernance,
    ChangeGovernanceMessage,
    PauseProxyMessage,
    GovernancePacker
} from "src/interfaces/IGovernance.sol";
import {VaraMessage, IMessageQueue, Hasher} from "src/interfaces/IMessageQueue.sol";

contract GovernanceAdminTest is Test, Base {
    using Hasher for VaraMessage;

    using GovernancePacker for ChangeGovernanceMessage;
    using GovernancePacker for PauseProxyMessage;

    function setUp() public {
        deployBridgeFromConstants();
    }

    function test_HandleMessageWithInvalidSender() public {
        bytes32 source = bytes32(uint256(0x11));
        bytes memory payload = "";

        vm.expectRevert(abi.encodeWithSelector(IGovernance.InvalidSender.selector));
        governanceAdmin.handleMessage(source, payload);
    }

    function test_HandleMessageWithInvalidSource() public {
        VaraMessage memory message = VaraMessage({
            nonce: 0x11,
            source: bytes32(uint256(0x22)),
            destination: address(governanceAdmin),
            payload: ""
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IGovernance.InvalidSource.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_HandleMessageWithEmptyPayload() public {
        VaraMessage memory message = VaraMessage({
            nonce: 0x11,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: ""
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IGovernance.InvalidPayload.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_HandleMessageWithInvalidDiscriminant() public {
        VaraMessage memory message = VaraMessage({
            nonce: 0x11,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: abi.encodePacked(uint8(GovernanceConstants.UPGRADE_PROXY + 1))
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IGovernance.InvalidPayload.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_HandleMessageWithChangeGovernance() public {
        bytes32 previousGovernance = governanceAdmin.governance();
        bytes32 newGovernance = bytes32(uint256(0x22));
        assertEq(ChangeGovernanceMessage({newGovernance: newGovernance}).pack().length, 33);
        VaraMessage memory message = VaraMessage({
            nonce: 0x11,
            source: previousGovernance,
            destination: address(governanceAdmin),
            payload: ChangeGovernanceMessage({newGovernance: newGovernance}).pack()
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(governanceAdmin));
        emit IGovernance.GovernanceChanged(previousGovernance, newGovernance);

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), true);
        assertEq(governanceAdmin.governance(), newGovernance);
    }

    function test_HandleMessageWithChangeGovernanceAndNotEnoughPayload() public {
        VaraMessage memory message = VaraMessage({
            nonce: 0x11,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: abi.encodePacked(uint8(GovernanceConstants.CHANGE_GOVERNANCE))
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IGovernance.InvalidPayload.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_HandleMessageWithUpgradeProxyAndNotEnoughPayload() public {
        VaraMessage memory message = VaraMessage({
            nonce: 0x11,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: abi.encodePacked(uint8(GovernanceConstants.UPGRADE_PROXY))
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IGovernance.InvalidPayload.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_HandleMessageWithPauseAndInvalidProxy() public {
        address invalidProxy = address(0x22);
        VaraMessage memory message = VaraMessage({
            nonce: 0x11,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: PauseProxyMessage({proxy: invalidProxy}).pack()
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IGovernance.InvalidPayload.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_HandleMessageWithPauseAndInvalidMessageSize() public {
        VaraMessage memory message = VaraMessage({
            nonce: 0x11,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: bytes.concat(PauseProxyMessage({proxy: address(messageQueue)}).pack(), "ff")
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IGovernance.InvalidPayload.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_HandleMessageWithUpgradeProxyAndInvalidMessageSize() public {
        VaraMessage memory message = VaraMessage({
            nonce: 0x11,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: abi.encodePacked(uint8(GovernanceConstants.UPGRADE_PROXY), address(messageQueue))
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IGovernance.InvalidPayload.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }
}
