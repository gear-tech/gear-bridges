// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

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
        deployBridgeDependsOnEnvironment();
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
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
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
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = currentBlockNumber++;
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
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
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
        uint256 blockNumber1 = currentBlockNumber++;
        uint256 blockNumber2 = currentBlockNumber++;
        uint256 blockNumber3 = currentBlockNumber++;
        uint256 blockNumber4 = currentBlockNumber++;

        uint256 blockNumber = blockNumber1;
        bytes32 merkleRoot = bytes32(uint256(0x22)); // valid root, update max block height
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

        blockNumber = blockNumber2;
        merkleRoot = bytes32(uint256(0x33)); // invalid root, suspicious address managed to send it

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

        blockNumber = blockNumber3;
        merkleRoot = bytes32(uint256(0x44)); // invalid root, suspicious address managed to send it

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

        vm.startPrank(deploymentArguments.emergencyStopObservers[0]);

        vm.expectEmit(address(messageQueue));
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

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.ChallengeRoot.selector));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.ChallengeRoot.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);

        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        blockNumber = blockNumber2;
        merkleRoot = bytes32(uint256(0xdeadbeef)); // valid root, calculated by emergency stop admin
        bytes32 previousMerkleRoot = bytes32(uint256(0x33));

        // emergency stop admin managed to submit valid root for first challenged block
        // and enabled emergency stop status
        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.EmergencyStopEnabled();

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.ChallengeRootDisabled();

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.isEmergencyStopped(), true);

        assertEq(messageQueue.getMerkleRoot(blockNumber), bytes32(0));
        assertEq(messageQueue.getMerkleRootTimestamp(previousMerkleRoot), 0);

        blockNumber = blockNumber3;
        merkleRoot = bytes32(uint256(0xfee1dead)); // valid root, calculated by emergency stop admin
        previousMerkleRoot = bytes32(uint256(0x44));

        // emergency stop admin managed to submit valid root for second challenged block
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), bytes32(0));
        assertEq(messageQueue.getMerkleRootTimestamp(previousMerkleRoot), 0);

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

        blockNumber = blockNumber3;
        merkleRoot = messageHash;

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        vm.expectEmit(address(messageQueue));
        emit IERC1967.Upgraded(address(newImplementationMock));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(
            address(uint160(uint256(vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT)))),
            address(newImplementationMock)
        );

        vm.stopPrank();
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

        vm.warp(vm.getBlockTimestamp() + 1);

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.ChallengeRootEnabled(vm.getBlockTimestamp() + messageQueue.CHALLENGE_ROOT_DELAY());

        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

        vm.stopPrank();
    }

    function test_ChallengeRootWithPause() public {
        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22)); // valid root, update max block height
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

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

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageQueue));
        emit PausableUpgradeable.Paused(address(governancePauser));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        vm.startPrank(deploymentArguments.emergencyStopObservers[0]);

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.ChallengeRootEnabled(vm.getBlockTimestamp() + messageQueue.CHALLENGE_ROOT_DELAY());

        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

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
        emit IMessageQueue.ChallengeRootEnabled(vm.getBlockTimestamp() + messageQueue.CHALLENGE_ROOT_DELAY());

        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

        vm.stopPrank();

        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        vm.expectEmit(address(messageQueue));
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
        bytes memory proof = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());
    }

    function test_SubmitMerkleRootWithBlockNumberBeforeGenesis() public {
        uint256 blockNumber = messageQueue.genesisBlock();
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = "";

        if (blockNumber == 0) {
            blockNumber += 42;

            vm.expectEmit(address(messageQueue));
            emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

            messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

            assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
            assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());
            assertEq(messageQueue.genesisBlock(), blockNumber);
            assertEq(messageQueue.maxBlockNumber(), blockNumber);
        }

        blockNumber -= 1;

        vm.expectRevert(
            abi.encodeWithSelector(
                IMessageQueue.BlockNumberBeforeGenesis.selector, blockNumber, messageQueue.genesisBlock()
            )
        );
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);
    }

    function test_SubmitMerkleRootWithBlockNumberTooFar() public {
        uint256 blockNumber = messageQueue.genesisBlock();
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = "";

        if (blockNumber == 0) {
            blockNumber += 42;

            vm.expectEmit(address(messageQueue));
            emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

            messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

            assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
            assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());
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

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = "";

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.InvalidPlonkProof.selector));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);
    }

    function test_SubmitMerkleRootWithEmergencyStop() public {
        uint256 blockNumber = currentBlockNumber++;
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
            nonce: messageNonce++,
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
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = currentBlockNumber++;
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
        uint256 blockNumber = currentBlockNumber++;
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
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = currentBlockNumber++;
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
