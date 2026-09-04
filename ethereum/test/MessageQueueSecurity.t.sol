// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.35;

import {PausableUpgradeable} from "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import {Initializable} from "@openzeppelin/contracts/proxy/utils/Initializable.sol";
import {Test} from "forge-std/Test.sol";
import {Upgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {GovernancePacker, PauseProxyMessage} from "src/interfaces/IGovernance.sol";
import {IMessageHandlerMock} from "src/interfaces/IMessageHandlerMock.sol";
import {Hasher, IMessageQueue, VaraMessage} from "src/interfaces/IMessageQueue.sol";
import {Base} from "test/Base.sol";

/**
 * @dev Regression tests for SECURITY(H-2) (fresh-deployment replay chain: genesis floor +
 *      processed-nonce watermark seeded via reinitializer) and SECURITY(H-3) (single-observer
 *      veto deadlock: challenge decay + governance-source bypass).
 */
contract MessageQueueSecurityTest is Test, Base {
    using Hasher for VaraMessage;

    using GovernancePacker for PauseProxyMessage;

    function setUp() public {
        deployBridgeDependsOnEnvironment();

        messageNonce = 1;
        currentBlockNumber = 1000;
    }

    /// deploys an additional MessageQueue UUPS proxy with pinned genesis floor and nonce watermark
    function deploySeededMessageQueue(uint256 genesisBlock_, uint256 nonceWatermark_)
        private
        returns (MessageQueue seeded)
    {
        vm.prank(deploymentArguments.deployerAddress, deploymentArguments.deployerAddress);
        seeded = MessageQueue(
            Upgrades.deployUUPSProxy(
                "MessageQueue.sol",
                abi.encodeCall(
                    MessageQueue.initialize,
                    (
                        governanceAdmin,
                        governancePauser,
                        deploymentArguments.emergencyStopAdmin,
                        deploymentArguments.emergencyStopObservers,
                        verifier,
                        genesisBlock_,
                        nonceWatermark_
                    )
                )
            )
        );
    }

    /// stores `message` as a single-leaf merkle tree at `blockNumber` and fast-forwards `messageDelay`
    function submitMessageRoot(MessageQueue queue, uint256 blockNumber, VaraMessage memory message, uint256 messageDelay)
        private
    {
        queue.submitMerkleRoot(blockNumber, message.hash(), "");
        vm.warp(vm.getBlockTimestamp() + messageDelay);
    }

    // SECURITY(H-2): a fresh deployment pins the genesis floor, so a self-proved pre-genesis root
    // cannot frontrun genesis and re-anchor the replay chain.
    function test_GenesisFloorRejectsPreGenesisRoot() public {
        if (!isFork()) {
            uint256 genesisBlock = 5_000_000;
            MessageQueue seeded = deploySeededMessageQueue(genesisBlock, 0);

            assertEq(seeded.genesisBlock(), genesisBlock);
            assertEq(seeded.maxBlockNumber(), genesisBlock);
            assertEq(seeded.nonceWatermark(), 0);

            bytes32 oldEraRoot = bytes32(uint256(0x11));
            vm.expectRevert(
                abi.encodeWithSelector(IMessageQueue.BlockNumberBeforeGenesis.selector, genesisBlock - 1, genesisBlock)
            );
            seeded.submitMerkleRoot(genesisBlock - 1, oldEraRoot, "");

            // the rejected submission must not be able to (re)write the pinned genesis floor
            assertEq(seeded.genesisBlock(), genesisBlock);
            assertEq(seeded.getMerkleRoot(genesisBlock - 1), bytes32(0));

            // submitting exactly at the genesis block works and updates the max block number
            seeded.submitMerkleRoot(genesisBlock, oldEraRoot, "");
            assertEq(seeded.getMerkleRoot(genesisBlock), oldEraRoot);
            assertEq(seeded.maxBlockNumber(), genesisBlock);
        }
    }

    // SECURITY(H-2): processed-nonce floor — messages below the pinned watermark are rejected as
    // replays even when their merkle proof is valid against a legitimately stored root.
    function test_NonceWatermarkRejectsLowNonce() public {
        if (!isFork()) {
            uint256 watermark = 1000;
            MessageQueue seeded = deploySeededMessageQueue(0, watermark);
            assertEq(seeded.nonceWatermark(), watermark);

            VaraMessage memory replayMessage = VaraMessage({
                nonce: watermark - 1,
                source: bytes32(uint256(0x22)),
                destination: address(messageHandlerMock),
                payload: hex"33"
            });
            uint256 replayBlock = currentBlockNumber++;
            submitMessageRoot(seeded, replayBlock, replayMessage, seeded.PROCESS_USER_MESSAGE_DELAY());

            bytes32[] memory emptyProof = new bytes32[](0);
            vm.expectRevert(
                abi.encodeWithSelector(MessageQueue.NonceBelowWatermark.selector, watermark - 1, watermark)
            );
            seeded.processMessage(replayBlock, 1, 0, replayMessage, emptyProof);
            assertEq(seeded.isProcessed(watermark - 1), false);

            // the nonce exactly at the watermark is the first accepted one
            VaraMessage memory freshMessage = VaraMessage({
                nonce: watermark,
                source: bytes32(uint256(0x22)),
                destination: address(messageHandlerMock),
                payload: hex"33"
            });
            uint256 freshBlock = currentBlockNumber++;
            submitMessageRoot(seeded, freshBlock, freshMessage, seeded.PROCESS_USER_MESSAGE_DELAY());

            vm.expectEmit(address(messageHandlerMock));
            emit IMessageHandlerMock.MessageHandled(freshMessage.source, freshMessage.payload);
            seeded.processMessage(freshBlock, 1, 0, freshMessage, emptyProof);
            assertEq(seeded.isProcessed(watermark), true);
        }
    }

    // SECURITY(H-2) monotonic reinitializer pattern: floors are write-once at initialization;
    // nobody — not even the governance admin — can re-seed them after deployment.
    function test_InitializeCannotReseedFloorsAfterDeployment() public {
        if (!isFork()) {
            vm.prank(address(governanceAdmin));
            vm.expectRevert(abi.encodeWithSelector(Initializable.InvalidInitialization.selector));
            messageQueue.initialize(
                governanceAdmin,
                governancePauser,
                deploymentArguments.emergencyStopAdmin,
                deploymentArguments.emergencyStopObservers,
                verifier,
                123_456,
                10
            );
        }
    }

    // SECURITY(H-3): decay math — each re-challenge extends the freeze deadline by exactly one
    // CHALLENGE_ROOT_DELAY instead of resetting a full fresh window from the re-challenge moment;
    // after the chained window fully decays, a new challenge starts a fresh full window.
    function test_ChallengeDecayMath() public {
        if (!isFork()) {
            uint256 delay = messageQueue.CHALLENGE_ROOT_DELAY();
            uint256 t0 = vm.getBlockTimestamp();

            vm.startPrank(deploymentArguments.emergencyStopObservers[0]);

            vm.expectEmit(address(messageQueue));
            emit IMessageQueue.ChallengeRootEnabled(t0 + delay);
            messageQueue.challengeRoot();

            // immediate re-challenge: deadline moves t0+delay -> t0+2*delay (extends by delay),
            // it is NOT reset to (t0+1)+delay
            vm.warp(t0 + 1);
            vm.expectEmit(address(messageQueue));
            emit IMessageQueue.ChallengeRootEnabled(t0 + 2 * delay);
            messageQueue.challengeRoot();

            // mid-window re-challenge extends by exactly one delay again
            vm.warp(t0 + delay);
            vm.expectEmit(address(messageQueue));
            emit IMessageQueue.ChallengeRootEnabled(t0 + 3 * delay);
            messageQueue.challengeRoot();

            vm.stopPrank();

            assertEq(messageQueue.isChallengingRoot(), true);

            vm.warp(t0 + 3 * delay - 1);
            assertEq(messageQueue.isChallengingRoot(), true);

            // the chained freeze self-expires once the decayed window passes, without admin action
            vm.warp(t0 + 3 * delay);
            assertEq(messageQueue.isChallengingRoot(), false);

            // challenging again after full expiry starts a fresh full window
            uint256 t1 = vm.getBlockTimestamp();
            vm.prank(deploymentArguments.emergencyStopObservers[0]);
            vm.expectEmit(address(messageQueue));
            emit IMessageQueue.ChallengeRootEnabled(t1 + delay);
            messageQueue.challengeRoot();
            assertEq(messageQueue.isChallengingRoot(), true);
        }
    }

    // SECURITY(H-3): governance-source bypass — while a challenge is active, normal messages stay
    // frozen but governance-admin-source messages (and messages relayed by the GovernanceAdmin
    // contract itself) still process, keeping the governance escape hatch alive.
    function test_GovernanceBypassWhileChallenging() public {
        if (!isFork()) {
            VaraMessage memory userMessage = VaraMessage({
                nonce: messageNonce++,
                source: bytes32(uint256(0x22)),
                destination: address(messageHandlerMock),
                payload: hex"33"
            });
            uint256 userBlock = currentBlockNumber++;
            submitMessageRoot(messageQueue, userBlock, userMessage, messageQueue.PROCESS_USER_MESSAGE_DELAY());

            VaraMessage memory relayedMessage = VaraMessage({
                nonce: messageNonce++,
                source: bytes32(uint256(0x44)),
                destination: address(messageHandlerMock),
                payload: hex"55"
            });
            uint256 relayedBlock = currentBlockNumber++;
            submitMessageRoot(messageQueue, relayedBlock, relayedMessage, messageQueue.PROCESS_USER_MESSAGE_DELAY());

            VaraMessage memory adminMessage = VaraMessage({
                nonce: messageNonce++,
                source: governanceAdmin.governance(),
                destination: address(governanceAdmin),
                payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
            });
            uint256 adminBlock = currentBlockNumber++;
            submitMessageRoot(messageQueue, adminBlock, adminMessage, messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

            vm.prank(deploymentArguments.emergencyStopObservers[0]);
            messageQueue.challengeRoot();
            assertEq(messageQueue.isChallengingRoot(), true);

            bytes32[] memory emptyProof = new bytes32[](0);

            // control: a normal user message stays blocked during the challenge
            vm.expectRevert(abi.encodeWithSelector(IMessageQueue.ChallengeRoot.selector));
            messageQueue.processMessage(userBlock, 1, 0, userMessage, emptyProof);

            // bypass arm 1: message relayed by the GovernanceAdmin contract itself
            vm.prank(address(governanceAdmin));
            vm.expectEmit(address(messageHandlerMock));
            emit IMessageHandlerMock.MessageHandled(relayedMessage.source, relayedMessage.payload);
            messageQueue.processMessage(relayedBlock, 1, 0, relayedMessage, emptyProof);
            assertEq(messageQueue.isProcessed(relayedMessage.nonce), true);

            // bypass arm 2: governance-admin source, relayed by anyone
            vm.expectEmit(address(messageQueue));
            emit PausableUpgradeable.Paused(address(governanceAdmin));
            messageQueue.processMessage(adminBlock, 1, 0, adminMessage, emptyProof);
            assertEq(messageQueue.paused(), true);
            assertEq(messageQueue.isProcessed(adminMessage.nonce), true);

            // user messages are still blocked after both bypasses
            vm.expectRevert(abi.encodeWithSelector(IMessageQueue.ChallengeRoot.selector));
            messageQueue.processMessage(userBlock, 1, 0, userMessage, emptyProof);
        }
    }

    // normal paths unchanged: with no watermark and no challenge, user messages process as before
    function test_NormalPathUnchanged() public {
        if (!isFork()) {
            VaraMessage memory message = VaraMessage({
                nonce: messageNonce++,
                source: bytes32(uint256(0x22)),
                destination: address(messageHandlerMock),
                payload: hex"33"
            });
            uint256 blockNumber = currentBlockNumber++;
            submitMessageRoot(messageQueue, blockNumber, message, messageQueue.PROCESS_USER_MESSAGE_DELAY());

            vm.expectEmit(address(messageHandlerMock));
            emit IMessageHandlerMock.MessageHandled(message.source, message.payload);
            messageQueue.processMessage(blockNumber, 1, 0, message, new bytes32[](0));
            assertEq(messageQueue.isProcessed(message.nonce), true);

            assertEq(messageQueue.genesisBlock(), blockNumber);
            assertEq(messageQueue.maxBlockNumber(), blockNumber);
            assertEq(messageQueue.nonceWatermark(), 0);
        }
    }
}
