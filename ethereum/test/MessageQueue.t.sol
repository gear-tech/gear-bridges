// Copyright (C) Gear Technologies Inc.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {PausableUpgradeable} from "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import {IAccessControl} from "@openzeppelin/contracts/access/IAccessControl.sol";
import {IERC1967} from "@openzeppelin/contracts/interfaces/IERC1967.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {Test} from "forge-std/Test.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {IMessageHandlerMock} from "src/interfaces/IMessageHandlerMock.sol";
import {IMessageQueue, VaraMessage} from "src/interfaces/IMessageQueue.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";
import {Hasher} from "src/libraries/Hasher.sol";
import {
    GovernancePacker,
    PauseProxyMessage,
    UnpauseProxyMessage,
    UpgradeProxyMessage
} from "src/libraries/packing/GovernancePacker.sol";
import {Base, ForkBeefyFixture} from "test/Base.sol";

contract MessageQueueTest is Test, Base {
    using Hasher for VaraMessage;

    using GovernancePacker for PauseProxyMessage;
    using GovernancePacker for UnpauseProxyMessage;
    using GovernancePacker for UpgradeProxyMessage;

    function setUp() public {
        deployBridgeFromConstants();
    }

    function _mappingSlot(bytes32 key, uint256 slot) private pure returns (bytes32) {
        return keccak256(abi.encode(key, slot));
    }

    function _seedLegacyRoot(uint256 blockNumber, bytes32 root, uint256 timestamp) private {
        vm.store(address(messageQueue), _mappingSlot(bytes32(blockNumber), 11), root);
        vm.store(address(messageQueue), _mappingSlot(root, 12), bytes32(timestamp));
    }

    function _userMessage(uint256 nonce) private view returns (VaraMessage memory) {
        return VaraMessage(nonce, bytes32(uint256(0x22)), address(messageHandlerMock), hex"33");
    }

    function test_LegacyLayoutHistoryAndTombstoneSurviveUpgrade() public {
        VaraMessage memory message = _userMessage(837);
        bytes32 root = message.hash();
        uint256 legacyBlock = 100;
        uint256 legacyTime = block.timestamp - 3600;
        _seedLegacyRoot(legacyBlock, root, legacyTime);
        vm.store(address(messageQueue), bytes32(uint256(8)), bytes32(legacyBlock));
        vm.store(address(messageQueue), bytes32(uint256(9)), bytes32(legacyBlock));
        vm.store(address(messageQueue), _mappingSlot(bytes32(uint256(836)), 13), bytes32(uint256(1)));
        vm.store(address(messageQueue), bytes32(uint256(14)), bytes32(legacyBlock + 1));
        bytes memory proof = _rootProof(legacyBlock + 1, root);
        messageQueue.submitMerkleRoot(legacyBlock + 1, root, proof);

        assertEq(
            address(uint160(uint256(vm.load(address(messageQueue), bytes32(uint256(0)))))), address(governanceAdmin)
        );
        assertEq(
            address(uint160(uint256(vm.load(address(messageQueue), bytes32(uint256(1)))))), address(governancePauser)
        );
        assertEq(
            address(uint160(uint256(vm.load(address(messageQueue), bytes32(uint256(2)))))),
            deploymentArguments.emergencyStopAdmin
        );
        assertEq(
            uint256(vm.load(address(messageQueue), bytes32(uint256(3)))),
            deploymentArguments.emergencyStopObservers.length
        );
        assertEq(
            uint256(
                vm.load(
                    address(messageQueue),
                    _mappingSlot(bytes32(uint256(uint160(deploymentArguments.emergencyStopObservers[0]))), 4)
                )
            ),
            1
        );
        assertEq(address(uint160(uint256(vm.load(address(messageQueue), bytes32(uint256(5)))))), address(verifier));
        assertEq(vm.load(address(messageQueue), bytes32(uint256(6))), bytes32(0));
        assertEq(vm.load(address(messageQueue), bytes32(uint256(7))), bytes32(0));
        assertEq(uint256(vm.load(address(messageQueue), bytes32(uint256(8)))), legacyBlock);
        assertEq(uint256(vm.load(address(messageQueue), bytes32(uint256(9)))), legacyBlock + 1);
        assertEq(vm.load(address(messageQueue), bytes32(uint256(10))), bytes32(0));
        assertEq(vm.load(address(messageQueue), _mappingSlot(bytes32(legacyBlock), 11)), root);
        assertEq(uint256(vm.load(address(messageQueue), _mappingSlot(root, 12))), legacyTime);
        assertEq(uint256(vm.load(address(messageQueue), _mappingSlot(bytes32(uint256(836)), 13))), 1);
        assertEq(messageQueue.beefyRootMinimum(), legacyBlock + 1);
        assertEq(uint256(vm.load(address(messageQueue), _mappingSlot(bytes32(legacyBlock + 1), 15))), block.timestamp);

        vm.prank(address(governanceAdmin));
        messageQueue.pause();
        bytes32[] memory slots = new bytes32[](16);
        for (uint256 i; i < slots.length; i++) {
            slots[i] = vm.load(address(messageQueue), bytes32(i));
        }
        MessageQueue replacement = new MessageQueue();
        vm.prank(address(governanceAdmin));
        messageQueue.upgradeToAndCall(address(replacement), "");
        for (uint256 i; i < slots.length; i++) {
            assertEq(vm.load(address(messageQueue), bytes32(i)), slots[i]);
        }
        assertTrue(messageQueue.paused());
        assertTrue(messageQueue.hasRole(messageQueue.DEFAULT_ADMIN_ROLE(), address(governanceAdmin)));
        assertEq(messageQueue.getMerkleRootTimestamp(root), legacyTime);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(legacyBlock), legacyTime);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(legacyBlock + 1), block.timestamp);
        assertTrue(messageQueue.isProcessed(836));
        vm.prank(address(governanceAdmin));
        messageQueue.unpause();
        messageQueue.processMessage(legacyBlock, 1, 0, message, new bytes32[](0));
        assertTrue(messageQueue.isProcessed(837));
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MessageAlreadyProcessed.selector, uint256(837)));
        messageQueue.processMessage(legacyBlock, 1, 0, message, new bytes32[](0));
        message.nonce = 836;
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MessageAlreadyProcessed.selector, uint256(836)));
        messageQueue.processMessage(legacyBlock, 1, 0, message, new bytes32[](0));
    }

    function test_LegacyFallbackIsUnavailableAtOrAboveFloor() public {
        VaraMessage memory message = _userMessage(900);
        bytes32 root = message.hash();
        uint256 timestamp = block.timestamp - 3600;
        vm.store(address(messageQueue), bytes32(uint256(14)), bytes32(0));
        _seedLegacyRoot(100, root, timestamp);
        _seedLegacyRoot(101, root, timestamp);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(101), timestamp);
        vm.store(address(messageQueue), bytes32(uint256(14)), bytes32(uint256(101)));
        assertEq(messageQueue.getMerkleRootTimestampForBlock(100), timestamp);
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MerkleRootTimestampNotFound.selector, uint256(101)));
        messageQueue.getMerkleRootTimestampForBlock(101);
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MerkleRootTimestampNotFound.selector, uint256(101)));
        messageQueue.processMessage(101, 1, 0, message, new bytes32[](0));
        assertFalse(messageQueue.isProcessed(message.nonce));
        assertEq(messageQueue.getMerkleRootTimestamp(root), timestamp);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(102), 0);
    }

    function test_RegisteredRootWithZeroTimestampFailsClosed() public {
        VaraMessage memory message = _userMessage(901);
        _seedLegacyRoot(100, message.hash(), 0);
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MerkleRootTimestampNotFound.selector, uint256(100)));
        messageQueue.getMerkleRootTimestampForBlock(100);
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MerkleRootTimestampNotFound.selector, uint256(100)));
        messageQueue.processMessage(100, 1, 0, message, new bytes32[](0));
        assertFalse(messageQueue.isProcessed(message.nonce));
    }

    function test_FutureAndOverflowingTimestampsFailWithoutArithmeticPanic() public {
        VaraMessage memory message = _userMessage(902);
        _seedLegacyRoot(100, message.hash(), type(uint256).max);
        vm.store(address(messageQueue), bytes32(uint256(14)), bytes32(uint256(101)));
        vm.expectRevert(IMessageQueue.MerkleRootDelayNotPassed.selector);
        messageQueue.processMessage(100, 1, 0, message, new bytes32[](0));
        vm.store(address(messageQueue), _mappingSlot(bytes32(uint256(100)), 15), bytes32(block.timestamp + 1));
        vm.expectRevert(IMessageQueue.MerkleRootDelayNotPassed.selector);
        messageQueue.processMessage(100, 1, 0, message, new bytes32[](0));
        vm.warp(type(uint256).max);
        vm.store(address(messageQueue), _mappingSlot(bytes32(uint256(100)), 15), bytes32(type(uint256).max - 300));
        messageQueue.processMessage(100, 1, 0, message, new bytes32[](0));
        assertTrue(messageQueue.isProcessed(message.nonce));
    }

    function test_NewRegistrationsAndConflictPreserveLegacyRootHistory() public {
        bytes32 root = bytes32(uint256(22));
        uint256 legacyTime = block.timestamp - 3600;
        _seedLegacyRoot(100, root, legacyTime);
        vm.store(address(messageQueue), bytes32(uint256(14)), bytes32(uint256(101)));
        bytes memory proof = _rootProof(101, root);
        messageQueue.submitMerkleRoot(101, root, proof);
        vm.warp(block.timestamp + 200);
        uint256 secondTime = block.timestamp;
        proof = _rootProof(102, root);
        messageQueue.submitMerkleRoot(102, root, proof);
        proof = _rootProof(101, bytes32(uint256(33)));
        messageQueue.submitMerkleRoot(101, bytes32(uint256(33)), proof);
        assertEq(messageQueue.getMerkleRootTimestamp(root), legacyTime);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(100), legacyTime);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(101), 0);
        assertEq(vm.load(address(messageQueue), _mappingSlot(bytes32(uint256(101)), 15)), bytes32(0));
        assertEq(messageQueue.getMerkleRootTimestampForBlock(102), secondTime);
        assertTrue(messageQueue.isEmergencyStopped());
    }

    function test_EmptyProgressHasNoEconomicOrMaturityState() public {
        bytes memory proof = _rootProof(100, bytes32(uint256(22)));
        messageQueue.submitMerkleRoot(100, bytes32(uint256(22)), proof);
        vm.store(address(messageQueue), bytes32(uint256(14)), bytes32(uint256(100)));
        proof = _rootProof(101, bytes32(0));
        messageQueue.submitEmptyQueueProgress(101, proof);
        assertEq(messageQueue.genesisBlock(), 100);
        assertEq(messageQueue.maxBlockNumber(), 101);
        assertEq(messageQueue.getMerkleRoot(101), bytes32(0));
        assertEq(messageQueue.getMerkleRootTimestampForBlock(101), 0);
        assertEq(vm.load(address(messageQueue), _mappingSlot(bytes32(uint256(101)), 15)), bytes32(0));
        assertEq(messageQueue.getMerkleRootTimestamp(bytes32(uint256(22))), 0);
        assertFalse(messageQueue.isProcessed(101));
        proof = _rootProof(101, bytes32(uint256(33)));
        messageQueue.submitMerkleRoot(101, bytes32(uint256(33)), proof);
        assertEq(messageQueue.getMerkleRoot(101), bytes32(uint256(33)));
        assertEq(messageQueue.getMerkleRootTimestampForBlock(101), block.timestamp);
        vm.expectRevert(IMessageQueue.EmptyQueueProgressNotForward.selector);
        messageQueue.submitEmptyQueueProgress(101, hex"1234");
    }

    function test_EmptyProgressRequiresInitializationAndValidProof() public {
        vm.expectRevert(IMessageQueue.EmptyQueueNotInitialized.selector);
        messageQueue.submitEmptyQueueProgress(100, "");
        vm.expectRevert(IMessageQueue.InvalidSourceBlock.selector);
        messageQueue.submitMerkleRoot(0, bytes32(uint256(22)), "");
        bytes memory proof = _rootProof(100, bytes32(uint256(22)));
        messageQueue.submitMerkleRoot(100, bytes32(uint256(22)), proof);
        vm.expectRevert(IMessageQueue.InvalidEmptyQueueProgressProof.selector);
        messageQueue.submitEmptyQueueProgress(101, "");
        assertEq(messageQueue.maxBlockNumber(), 100);
        assertEq(messageQueue.getMerkleRoot(101), bytes32(0));
    }

    function test_EmergencyPublisherCannotBypassFloorForwardOrBounds() public {
        bytes memory proof = _rootProof(100, bytes32(uint256(22)));
        messageQueue.submitMerkleRoot(100, bytes32(uint256(22)), proof);
        proof = _rootProof(100, bytes32(uint256(33)));
        messageQueue.submitMerkleRoot(100, bytes32(uint256(33)), proof);
        vm.store(address(messageQueue), bytes32(uint256(14)), bytes32(uint256(102)));
        vm.prank(deploymentArguments.emergencyStopObservers[0]);
        messageQueue.challengeRoot();
        vm.startPrank(deploymentArguments.emergencyStopAdmin);
        vm.expectRevert(
            abi.encodeWithSelector(IMessageQueue.BlockNumberBelowMinimum.selector, uint256(101), uint256(102))
        );
        messageQueue.submitMerkleRoot(101, bytes32(uint256(33)), "");
        vm.expectRevert(
            abi.encodeWithSelector(IMessageQueue.BlockNumberBelowMinimum.selector, uint256(101), uint256(102))
        );
        messageQueue.submitEmptyQueueProgress(101, "");
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.BlockNumberOverflow.selector, uint256(1) << 32));
        messageQueue.submitMerkleRoot(uint256(1) << 32, bytes32(uint256(33)), "");
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.BlockNumberOverflow.selector, uint256(1) << 32));
        messageQueue.submitEmptyQueueProgress(uint256(1) << 32, "");
        uint256 maxAllowed = 100 + messageQueue.MAX_BLOCK_DISTANCE();
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.BlockNumberTooFar.selector, maxAllowed + 1, maxAllowed));
        messageQueue.submitMerkleRoot(maxAllowed + 1, bytes32(uint256(33)), "");
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.BlockNumberTooFar.selector, maxAllowed + 1, maxAllowed));
        messageQueue.submitEmptyQueueProgress(maxAllowed + 1, "");
        vm.stopPrank();
        proof = _rootProof(102, bytes32(uint256(33)));
        vm.prank(deploymentArguments.emergencyStopAdmin);
        messageQueue.submitMerkleRoot(102, bytes32(uint256(33)), proof);
        proof = _rootProof(104, bytes32(0));
        vm.prank(deploymentArguments.emergencyStopAdmin);
        messageQueue.submitEmptyQueueProgress(104, proof);
        proof = _rootProof(103, bytes32(uint256(44)));
        vm.prank(deploymentArguments.emergencyStopAdmin);
        messageQueue.submitMerkleRoot(103, bytes32(uint256(44)), proof);
        assertEq(messageQueue.getMerkleRoot(103), bytes32(uint256(44)));
        vm.prank(deploymentArguments.emergencyStopAdmin);
        vm.expectRevert(IMessageQueue.EmptyQueueProgressNotForward.selector);
        messageQueue.submitEmptyQueueProgress(103, "");
        assertEq(messageQueue.maxBlockNumber(), 104);
        assertEq(messageQueue.getMerkleRoot(104), bytes32(0));
    }

    function test_RepeatedRootHasIndependentBlockMaturity() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        uint256 first = ++currentBlockNumber;
        uint256 second = ++currentBlockNumber;
        bytes32 root = message.hash();
        uint256 firstTime = vm.getBlockTimestamp();
        bytes memory proof = _rootProof(first, root);
        messageQueue.submitMerkleRoot(first, root, proof);
        vm.warp(firstTime + 200);
        proof = _rootProof(second, root);
        messageQueue.submitMerkleRoot(second, root, proof);
        vm.warp(firstTime + 300);
        vm.expectRevert(IMessageQueue.MerkleRootDelayNotPassed.selector);
        messageQueue.processMessage(second, 1, 0, message, new bytes32[](0));
        uint256 saved = vm.snapshotState();
        messageQueue.processMessage(first, 1, 0, message, new bytes32[](0));
        assertTrue(messageQueue.isProcessed(message.nonce));
        assertTrue(vm.revertToState(saved));
        vm.warp(firstTime + 500);
        messageQueue.processMessage(second, 1, 0, message, new bytes32[](0));
        assertTrue(messageQueue.isProcessed(message.nonce));
    }

    function test_ConflictingBlockDoesNotDeleteSharedRootTimestamp() public {
        uint256 first = ++currentBlockNumber;
        uint256 second = ++currentBlockNumber;
        bytes32 root = bytes32(uint256(0x22));
        bytes memory proof = _rootProof(first, root);
        messageQueue.submitMerkleRoot(first, root, proof);
        vm.warp(vm.getBlockTimestamp() + 200);
        uint256 secondTime = vm.getBlockTimestamp();
        proof = _rootProof(second, root);
        messageQueue.submitMerkleRoot(second, root, proof);
        proof = _rootProof(first, bytes32(uint256(0x33)));
        messageQueue.submitMerkleRoot(first, bytes32(uint256(0x33)), proof);
        assertEq(messageQueue.getMerkleRoot(first), bytes32(0));
        assertEq(messageQueue.getMerkleRoot(second), root);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(second), secondTime);
        assertTrue(messageQueue.isEmergencyStopped());
    }

    function test_SubmitMerkleRootWithAuthenticatedBeefyProof() public {
        ForkBeefyFixture realFixture = new ForkBeefyFixture(messageQueue, 1, false);
        verifier = IVerifier(address(realFixture.verifier()));
        vm.store(address(messageQueue), bytes32(uint256(5)), bytes32(uint256(uint160(address(verifier)))));
        uint256 sourceBlock = 100;
        bytes32 root = bytes32(uint256(0x22));
        bytes memory proof = realFixture.rootProof(uint32(sourceBlock), root);

        messageQueue.submitMerkleRoot(sourceBlock, root, proof);

        assertEq(messageQueue.getMerkleRoot(sourceBlock), root);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(sourceBlock), vm.getBlockTimestamp());
        assertEq(messageQueue.genesisBlock(), sourceBlock);
        assertEq(messageQueue.maxBlockNumber(), sourceBlock);
    }

    function test_SubmitMerkleRootRejectsAliasedBlockNumber() public {
        ForkBeefyFixture realFixture = new ForkBeefyFixture(messageQueue, 1, false);
        verifier = IVerifier(address(realFixture.verifier()));
        vm.store(address(messageQueue), bytes32(uint256(5)), bytes32(uint256(uint160(address(verifier)))));
        uint256 canonicalBlock = 100;
        bytes32 root = bytes32(uint256(0x22));
        bytes memory proof = realFixture.rootProof(uint32(canonicalBlock), root);
        uint256 aliasBlock = canonicalBlock | (1 << 32);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.BlockNumberOverflow.selector, aliasBlock));
        messageQueue.submitMerkleRoot(aliasBlock, root, proof);

        assertEq(messageQueue.genesisBlock(), 0);
        assertEq(messageQueue.maxBlockNumber(), 0);
        assertEq(messageQueue.getMerkleRoot(aliasBlock), bytes32(0));
        assertEq(messageQueue.getMerkleRootTimestampForBlock(aliasBlock), 0);
        messageQueue.submitMerkleRoot(canonicalBlock, root, proof);
        assertEq(messageQueue.getMerkleRoot(canonicalBlock), root);
    }

    function test_PauseWithGovernanceAdmin() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = _rootProof(blockNumber, merkleRoot);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit PausableUpgradeable.Paused(address(governanceAdmin));

        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        VaraMessage memory message2 = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(messageQueue.isProcessed(message2.nonce), false);
    }

    function test_PauseWithGovernancePauser() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = _rootProof(blockNumber, merkleRoot);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit PausableUpgradeable.Paused(address(governancePauser));

        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        VaraMessage memory message2 = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(messageQueue.isProcessed(message2.nonce), false);
    }

    function test_PauseUnauthorized() public {
        vm.expectRevert(
            abi.encodeWithSelector(
                IAccessControl.AccessControlUnauthorizedAccount.selector, address(this), messageQueue.PAUSER_ROLE()
            )
        );
        messageQueue.pause();
    }

    function test_UnpauseWithGovernanceAdmin() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = _rootProof(blockNumber, merkleRoot);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit PausableUpgradeable.Paused(address(governanceAdmin));

        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        VaraMessage memory message2 = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        VaraMessage memory message3 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: UnpauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message3.nonce), false);

        messageHash = message3.hash();

        blockNumber = currentBlockNumber++;
        merkleRoot = messageHash;
        proof1 = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit PausableUpgradeable.Unpaused(address(governanceAdmin));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message3, proof2);
        assertEq(messageQueue.isProcessed(message3.nonce), true);
    }

    function test_UnpauseWithGovernancePauser() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = _rootProof(blockNumber, merkleRoot);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit PausableUpgradeable.Paused(address(governancePauser));

        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        VaraMessage memory message2 = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        VaraMessage memory message3 = VaraMessage({
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: UnpauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message3.nonce), false);

        messageHash = message3.hash();

        blockNumber = currentBlockNumber++;
        merkleRoot = messageHash;
        proof1 = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit PausableUpgradeable.Unpaused(address(governancePauser));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message3, proof2);
        assertEq(messageQueue.isProcessed(message3.nonce), true);
    }

    function test_UnpauseUnauthorized() public {
        vm.expectRevert(
            abi.encodeWithSelector(
                IAccessControl.AccessControlUnauthorizedAccount.selector, address(this), messageQueue.PAUSER_ROLE()
            )
        );
        messageQueue.unpause();
    }

    function test_UpgradeToAndCallWithGovernanceAdmin() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: UpgradeProxyMessage({
                proxy: address(messageQueue), newImplementation: address(newImplementationMock), data: ""
            }).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IERC1967.Upgraded(address(newImplementationMock));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(
            address(uint160(uint256(vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT)))),
            address(newImplementationMock)
        );

        // just to get coverage for NewImplementationMock contract

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IERC1967.Upgraded(address(newImplementationMock));

        messageQueue.upgradeToAndCall(address(newImplementationMock), "");
        assertEq(
            address(uint160(uint256(vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT)))),
            address(newImplementationMock)
        );
    }

    function test_UpgradeToAndCallUnauthorized() public {
        vm.startPrank(address(governancePauser));

        vm.expectRevert(
            abi.encodeWithSelector(
                IAccessControl.AccessControlUnauthorizedAccount.selector,
                address(governancePauser),
                messageQueue.DEFAULT_ADMIN_ROLE()
            )
        );
        messageQueue.upgradeToAndCall(address(0), "");

        vm.stopPrank();
    }

    function test_ChallengeRoot() public {
        uint256 blockNumber1 = currentBlockNumber++;
        uint256 blockNumber2 = currentBlockNumber++;
        uint256 blockNumber3 = currentBlockNumber++;
        uint256 blockNumber4 = currentBlockNumber++;

        uint256 blockNumber = blockNumber1;
        bytes32 merkleRoot = bytes32(uint256(0x22)); // valid root, update max block height
        bytes memory proof1 = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(blockNumber), vm.getBlockTimestamp());

        blockNumber = blockNumber2;
        merkleRoot = bytes32(uint256(0x33)); // invalid root, suspicious address managed to send it
        proof1 = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(blockNumber), vm.getBlockTimestamp());

        blockNumber = blockNumber3;
        merkleRoot = bytes32(uint256(0x44)); // invalid root, suspicious address managed to send it
        proof1 = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(blockNumber), vm.getBlockTimestamp());

        vm.startPrank(deploymentArguments.emergencyStopObservers[0]);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.ChallengeRootEnabled(vm.getBlockTimestamp() + messageQueue.CHALLENGE_ROOT_DELAY());

        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

        vm.stopPrank();

        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        blockNumber = blockNumber4;
        merkleRoot = messageHash; // valid root, just to check that no one can submit any root now

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.ChallengeRoot.selector));
        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.ChallengeRoot.selector));
        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);

        blockNumber = blockNumber2;
        merkleRoot = bytes32(uint256(0xdeadbeef)); // valid root, calculated by emergency stop admin
        proof1 = _rootProof(blockNumber, merkleRoot);

        // emergency stop admin managed to submit valid root for first challenged block
        // and enabled emergency stop status
        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.EmergencyStopEnabled();

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.ChallengeRootDisabled();

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.prank(deploymentArguments.emergencyStopAdmin);
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.isEmergencyStopped(), true);

        assertEq(messageQueue.getMerkleRoot(blockNumber), bytes32(0));
        assertEq(messageQueue.getMerkleRootTimestampForBlock(blockNumber), 0);

        blockNumber = blockNumber3;
        merkleRoot = bytes32(uint256(0xfee1dead)); // valid root, calculated by emergency stop admin
        proof1 = _rootProof(blockNumber, merkleRoot);

        // emergency stop admin managed to submit valid root for second challenged block
        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.prank(deploymentArguments.emergencyStopAdmin);
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), bytes32(0));
        assertEq(messageQueue.getMerkleRootTimestampForBlock(blockNumber), 0);

        // when all bad roots are removed
        // governance can send message to update MessageQueue and remove emergency stop status
        // emergencyStopAdmin in this case will only accept roots from our RPC node
        VaraMessage memory message2 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: UpgradeProxyMessage({
                proxy: address(messageQueue), newImplementation: address(newImplementationMock), data: ""
            }).pack()
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        messageHash = message2.hash();

        blockNumber = blockNumber3 + 1;
        merkleRoot = messageHash;
        proof1 = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        vm.prank(deploymentArguments.emergencyStopAdmin);
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IERC1967.Upgraded(address(newImplementationMock));

        vm.prank(deploymentArguments.emergencyStopAdmin);
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(
            address(uint160(uint256(vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT)))),
            address(newImplementationMock)
        );
    }

    function test_ChallengeRootWithNotEmergencyStopObserver() public {
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.NotEmergencyStopObserver.selector));
        messageQueue.challengeRoot();
    }

    function test_ChallengeRootTwice() public {
        vm.startPrank(deploymentArguments.emergencyStopObservers[0]);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.ChallengeRootEnabled(vm.getBlockTimestamp() + messageQueue.CHALLENGE_ROOT_DELAY());

        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

        vm.warp(vm.getBlockTimestamp() + 1);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.ChallengeRootEnabled(vm.getBlockTimestamp() + messageQueue.CHALLENGE_ROOT_DELAY());

        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

        vm.stopPrank();
    }

    function test_ChallengeRootWithPause() public {
        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22)); // valid root, update max block height
        bytes memory proof1 = _rootProof(blockNumber, merkleRoot);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(blockNumber), vm.getBlockTimestamp());

        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        blockNumber = currentBlockNumber++;
        merkleRoot = messageHash; // invalid root, suspicious address managed to send pause
        proof1 = _rootProof(blockNumber, merkleRoot);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(blockNumber), vm.getBlockTimestamp());

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit PausableUpgradeable.Paused(address(governancePauser));

        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.startPrank(deploymentArguments.emergencyStopObservers[0]);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.ChallengeRootEnabled(vm.getBlockTimestamp() + messageQueue.CHALLENGE_ROOT_DELAY());

        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.stopPrank();

        blockNumber = currentBlockNumber++;
        merkleRoot = bytes32(uint256(0x33)); // valid root, just to check that no one can submit any root now

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.ChallengeRoot.selector));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.ChallengeRoot.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
    }

    function test_DisableChallengeRoot() public {
        vm.startPrank(deploymentArguments.emergencyStopObservers[0]);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.ChallengeRootEnabled(vm.getBlockTimestamp() + messageQueue.CHALLENGE_ROOT_DELAY());

        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

        vm.stopPrank();

        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.ChallengeRootDisabled();

        messageQueue.disableChallengeRoot();

        vm.stopPrank();
    }

    function test_DisableChallengeRootWithNotEmergencyStopAdmin() public {
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.NotEmergencyStopAdmin.selector));
        messageQueue.disableChallengeRoot();
    }

    function test_DisableChallengeRootWithChallengeRootNotEnabled() public {
        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.ChallengeRootNotEnabled.selector));
        messageQueue.disableChallengeRoot();

        vm.stopPrank();
    }

    function test_AllowMessageProcessing() public {
        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(blockNumber), vm.getBlockTimestamp());

        merkleRoot = bytes32(uint256(0x33));
        proof = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.EmergencyStopEnabled();

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.EmergencyStop.selector));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.EmergencyStop.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);

        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MessageProcessingAllowed();

        messageQueue.allowMessageProcessing();

        vm.stopPrank();

        // message is not processed, but anyone can call processMessage now
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MerkleRootNotFound.selector, blockNumber));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_AllowMessageProcessingWithNotEmergencyStopAdmin() public {
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.NotEmergencyStopAdmin.selector));
        messageQueue.allowMessageProcessing();
    }

    function test_AllowMessageProcessingWithNotEmergencyStop() public {
        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.EmergencyStopNotEnabled.selector));
        messageQueue.allowMessageProcessing();

        vm.stopPrank();
    }

    function test_SubmitMerkleRoot() public {
        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(blockNumber), vm.getBlockTimestamp());
    }

    function test_SubmitMerkleRootBackfillsBeforeGenesisAboveFloor() public {
        uint256 firstBlock = 42;
        bytes32 root = bytes32(uint256(0x22));
        bytes memory proof = _rootProof(firstBlock, root);
        messageQueue.submitMerkleRoot(firstBlock, root, proof);

        proof = _rootProof(firstBlock - 1, root);
        messageQueue.submitMerkleRoot(firstBlock - 1, root, proof);

        assertEq(messageQueue.getMerkleRoot(firstBlock - 1), root);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(firstBlock - 1), vm.getBlockTimestamp());
        assertEq(messageQueue.genesisBlock(), firstBlock);
        assertEq(messageQueue.maxBlockNumber(), firstBlock);
    }

    function test_SubmitMerkleRootRejectsBelowBeefyFloor() public {
        vm.store(address(messageQueue), bytes32(uint256(14)), bytes32(uint256(42)));
        bytes32 root = bytes32(uint256(0x22));
        bytes memory proof = _rootProof(42, root);

        vm.expectRevert(
            abi.encodeWithSelector(IMessageQueue.BlockNumberBelowMinimum.selector, uint256(41), uint256(42))
        );
        messageQueue.submitMerkleRoot(41, root, "");
        messageQueue.submitMerkleRoot(42, root, proof);
        assertEq(messageQueue.getMerkleRoot(42), root);
    }

    function test_PublicationRequiresBeefyFloor() public {
        vm.store(address(messageQueue), bytes32(uint256(14)), bytes32(0));

        vm.expectRevert(IMessageQueue.InvalidBeefyVerifier.selector);
        messageQueue.submitMerkleRoot(100, bytes32(uint256(0x22)), "");
        vm.expectRevert(IMessageQueue.InvalidBeefyVerifier.selector);
        messageQueue.submitEmptyQueueProgress(100, "");
        assertEq(messageQueue.genesisBlock(), 0);
        assertEq(messageQueue.maxBlockNumber(), 0);
    }

    function test_SubmitMerkleRootWithBlockNumberTooFar() public {
        uint256 blockNumber = messageQueue.genesisBlock();
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = "";

        if (blockNumber == 0) {
            blockNumber += 42;
            proof = _rootProof(blockNumber, merkleRoot);

            vm.expectEmit(address(messageQueue));
            // forge-lint: disable-next-item(reentrancy-events)
            emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

            messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

            assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
            assertEq(messageQueue.getMerkleRootTimestampForBlock(blockNumber), vm.getBlockTimestamp());
            assertEq(messageQueue.genesisBlock(), blockNumber);
            assertEq(messageQueue.maxBlockNumber(), blockNumber);
        }

        blockNumber = messageQueue.maxBlockNumber() + messageQueue.MAX_BLOCK_DISTANCE() + 1;

        vm.expectRevert(
            abi.encodeWithSelector(
                IMessageQueue.BlockNumberTooFar.selector,
                blockNumber,
                messageQueue.maxBlockNumber() + messageQueue.MAX_BLOCK_DISTANCE()
            )
        );
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);
    }

    function test_SubmitMerkleRootTwice() public {
        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(blockNumber), vm.getBlockTimestamp());

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MerkleRootAlreadySet.selector, blockNumber));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);
    }

    function test_SubmitMerkleRootWithInvalidMMRSibling() public {
        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = _rootProof(blockNumber, merkleRoot);
        proof[proof.length - 1] ^= bytes1(uint8(1));

        vm.expectRevert(IMessageQueue.InvalidQueueRootProof.selector);
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);
        assertEq(messageQueue.getMerkleRoot(blockNumber), bytes32(0));
    }

    function test_SubmitMerkleRootRejectsWrongProofSourceBlock() public {
        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = _rootProof(blockNumber, merkleRoot);

        vm.expectRevert(IMessageQueue.InvalidQueueRootProof.selector);
        messageQueue.submitMerkleRoot(blockNumber + 1, merkleRoot, proof);
        assertEq(messageQueue.maxBlockNumber(), 0);
    }

    function test_SubmitMerkleRootWithEmergencyStop() public {
        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof1 = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(blockNumber), vm.getBlockTimestamp());

        merkleRoot = bytes32(uint256(0x33));
        proof1 = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.EmergencyStopEnabled();

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.isEmergencyStopped(), true);

        assertEq(messageQueue.getMerkleRoot(blockNumber), bytes32(0));
        assertEq(messageQueue.getMerkleRootTimestampForBlock(blockNumber), 0);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.EmergencyStop.selector));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();
        merkleRoot = messageHash;
        proof1 = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        vm.prank(deploymentArguments.emergencyStopAdmin);
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageHandlerMock));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageHandlerMock.MessageHandled(message.source, message.payload);

        vm.prank(deploymentArguments.emergencyStopAdmin);
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), true);
    }

    function test_ProcessMessage() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageHandlerMock));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageHandlerMock.MessageHandled(message.source, message.payload);

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), true);
    }

    function test_ProcessMessageWithEmergencyStop() public {
        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(blockNumber), vm.getBlockTimestamp());

        merkleRoot = bytes32(uint256(0x33));
        proof = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.EmergencyStopEnabled();

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.EmergencyStop.selector));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.EmergencyStop.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_ProcessMessageWithMessageAlreadyProcessed() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageHandlerMock));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageHandlerMock.MessageHandled(message.source, message.payload);

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), true);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MessageAlreadyProcessed.selector, message.nonce));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), true);
    }

    function test_ProcessMessageWithMerkleRootNotSet() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        uint256 blockNumber = currentBlockNumber++;
        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MerkleRootNotFound.selector, blockNumber));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_ProcessMessageWithMerkleRootDelayNotPassed() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MerkleRootDelayNotPassed.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_ProcessMessageWithInvalidMerkleProof() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = _rootProof(blockNumber, merkleRoot);

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](1);
        proof2[0] = bytes32(uint256(0xbad));

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.InvalidMerkleProof.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_AddEmergencyStopObserver() public {
        address newObserver = address(0x11);
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.NotEmergencyStopAdmin.selector));
        messageQueue.addEmergencyStopObserver(newObserver);

        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.EmergencyStopObserverAdded(newObserver);

        messageQueue.addEmergencyStopObserver(newObserver);

        vm.stopPrank();

        vm.startPrank(newObserver);

        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

        vm.stopPrank();
    }

    function test_RemoveEmergencyStopObserver() public {
        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        address newObserver = address(0x11);
        messageQueue.addEmergencyStopObserver(newObserver);

        vm.stopPrank();

        vm.startPrank(newObserver);

        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

        vm.stopPrank();

        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        messageQueue.disableChallengeRoot();
        assertEq(messageQueue.isChallengingRoot(), false);

        vm.stopPrank();

        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.EmergencyStopObserverRemoved(newObserver);

        messageQueue.removeEmergencyStopObserver(newObserver);

        vm.stopPrank();

        vm.startPrank(newObserver);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.NotEmergencyStopObserver.selector));
        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), false);

        vm.stopPrank();
    }

    function test_RemoveEmergencyStopObserverDuringActiveChallenge() public {
        address observer = deploymentArguments.emergencyStopObservers[0];

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.NotEmergencyStopAdmin.selector));
        messageQueue.removeEmergencyStopObserver(observer);

        vm.startPrank(observer);

        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

        vm.stopPrank();

        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        messageQueue.removeEmergencyStopObserver(observer);
        assertEq(messageQueue.isChallengingRoot(), true);

        vm.stopPrank();

        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        messageQueue.disableChallengeRoot();
        assertEq(messageQueue.isChallengingRoot(), false);

        vm.stopPrank();

        vm.startPrank(observer);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.NotEmergencyStopObserver.selector));
        messageQueue.challengeRoot();

        vm.stopPrank();
    }
}
