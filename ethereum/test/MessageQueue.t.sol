// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test} from "forge-std/Test.sol";
import {Base} from "./Base.sol";
import {IAccessControl} from "@openzeppelin/contracts/access/IAccessControl.sol";
import {IERC1967} from "@openzeppelin/contracts/interfaces/IERC1967.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {PausableUpgradeable} from "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import {
    PauseProxyMessage,
    UnpauseProxyMessage,
    UpgradeProxyMessage,
    GovernancePacker
} from "src/interfaces/IGovernance.sol";
import {IMessageHandlerMock} from "src/interfaces/IMessageHandlerMock.sol";
import {VaraMessage, IMessageQueue, Hasher} from "src/interfaces/IMessageQueue.sol";
import {IVerifierMock} from "src/interfaces/IVerifierMock.sol";

contract MessageQueueTest is Test, Base {
    using Hasher for VaraMessage;

    using GovernancePacker for PauseProxyMessage;
    using GovernancePacker for UnpauseProxyMessage;
    using GovernancePacker for UpgradeProxyMessage;

    function setUp() public {
        deployBridgeFromConstants();
    }

    function test_PauseWithGovernanceAdmin() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: 0x11,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageQueue));
        emit PausableUpgradeable.Paused(address(governanceAdmin));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        VaraMessage memory message2 = VaraMessage({
            nonce: 0x12,
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
            nonce: 0x11,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageQueue));
        emit PausableUpgradeable.Paused(address(governancePauser));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        VaraMessage memory message2 = VaraMessage({
            nonce: 0x12,
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
            nonce: 0x11,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageQueue));
        emit PausableUpgradeable.Paused(address(governanceAdmin));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        VaraMessage memory message2 = VaraMessage({
            nonce: 0x12,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        VaraMessage memory message3 = VaraMessage({
            nonce: 0x13,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: UnpauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message3.nonce), false);

        messageHash = message3.hash();

        blockNumber = 0x55;
        merkleRoot = messageHash;

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        vm.expectEmit(address(messageQueue));
        emit PausableUpgradeable.Unpaused(address(governanceAdmin));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message3, proof2);
        assertEq(messageQueue.isProcessed(message3.nonce), true);
    }

    function test_UnpauseWithGovernancePauser() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: 0x11,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageQueue));
        emit PausableUpgradeable.Paused(address(governancePauser));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        VaraMessage memory message2 = VaraMessage({
            nonce: 0x12,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        VaraMessage memory message3 = VaraMessage({
            nonce: 0x13,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: UnpauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message3.nonce), false);

        messageHash = message3.hash();

        blockNumber = 0x55;
        merkleRoot = messageHash;

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());

        vm.expectEmit(address(messageQueue));
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
            nonce: 0x11,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: UpgradeProxyMessage({
                proxy: address(messageQueue),
                newImplementation: address(newImplementationMock),
                data: ""
            }).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageQueue));
        emit IERC1967.Upgraded(address(newImplementationMock));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(
            address(uint160(uint256(vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT)))),
            address(newImplementationMock)
        );

        // just to get coverage for NewImplementationMock contract

        vm.expectEmit(address(messageQueue));
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
        // TODO
    }

    function test_ChallengeRootWithNotEmergencyStopObserver() public {
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.NotEmergencyStopObserver.selector));
        messageQueue.challengeRoot();
    }

    function test_ChallengeRootTwice() public {
        vm.startPrank(deploymentArguments.emergencyStopObservers[0]);

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.ChallengeRootEnabled(vm.getBlockTimestamp() + messageQueue.CHALLENGE_ROOT_DELAY());

        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.ChallengeRoot.selector));
        messageQueue.challengeRoot();

        vm.stopPrank();
    }

    function test_SubmitMerkleRoot() public {
        uint256 blockNumber = 0x11;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());
    }

    function test_SubmitMerkleRootTwice() public {
        uint256 blockNumber = 0x11;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MerkleRootAlreadySet.selector, blockNumber));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);
    }

    function test_SubmitMerkleRootWithInvalidProof() public {
        IVerifierMock(address(verifier)).setValue(false);

        uint256 blockNumber = 0x11;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = "";

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.InvalidPlonkProof.selector));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);
    }

    function test_SubmitMerkleRootWithEmergencyStop() public {
        uint256 blockNumber = 0x11;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

        bytes32 previousMerkleRoot = merkleRoot;
        merkleRoot = bytes32(uint256(0x33));

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.EmergencyStopEnabled();

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.isEmergencyStopped(), true);

        assertEq(messageQueue.getMerkleRoot(blockNumber), bytes32(0));
        assertEq(messageQueue.getMerkleRootTimestamp(previousMerkleRoot), 0);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.EmergencyStop.selector));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        VaraMessage memory message = VaraMessage({
            nonce: 0x11,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();
        merkleRoot = messageHash;

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageHandlerMock));
        emit IMessageHandlerMock.MessageHandled(message.source, message.payload);

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), true);

        vm.stopPrank();
    }

    function test_ProcessMessage() public {
        VaraMessage memory message = VaraMessage({
            nonce: 0x11,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageHandlerMock));
        emit IMessageHandlerMock.MessageHandled(message.source, message.payload);

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), true);
    }

    function test_ProcessMessageWithEmergencyStop() public {
        uint256 blockNumber = 0x11;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

        merkleRoot = bytes32(uint256(0x33));

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.EmergencyStopEnabled();

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.EmergencyStop.selector));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        VaraMessage memory message = VaraMessage({
            nonce: 0x11,
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
            nonce: 0x11,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageHandlerMock));
        emit IMessageHandlerMock.MessageHandled(message.source, message.payload);

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), true);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MessageAlreadyProcessed.selector, message.nonce));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), true);
    }

    function test_ProcessMessageWithMerkleRootNotSet() public {
        VaraMessage memory message = VaraMessage({
            nonce: 0x11,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        uint256 blockNumber = 0x44;
        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MerkleRootNotFound.selector, blockNumber));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_ProcessMessageWithMerkleRootDelayNotPassed() public {
        VaraMessage memory message = VaraMessage({
            nonce: 0x11,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MerkleRootDelayNotPassed.selector, blockNumber));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_ProcessMessageWithInvalidMerkleProof() public {
        VaraMessage memory message = VaraMessage({
            nonce: 0x11,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = 0x44;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
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
}
