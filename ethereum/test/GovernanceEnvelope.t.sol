// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {Test} from "forge-std/Test.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {GovernanceControl, IMessageQueue, LegacyControl, VaraMessage} from "src/interfaces/IMessageQueue.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";
import {Hasher} from "src/libraries/Hasher.sol";
import {
    ChangeGovernanceMessage,
    GovernancePacker,
    PauseProxyMessage,
    UnpauseProxyMessage,
    UpgradeProxyMessage
} from "src/libraries/packing/GovernancePacker.sol";
import {Base, ForkBeefyFixture} from "test/Base.sol";

/// @dev Reconstructed from supplied HTML, audited 7fe1b032; not the unavailable original PoC bundle.
/// Uses the production queue, retained constructor handlers and real fixture-signed BEEFY root proofs.
contract GovernanceEnvelopeTest is Test, Base {
    using Hasher for VaraMessage;

    function setUp() public {
        deployBridgeFromConstants();
    }

    function _deployVerifier(uint256, address queueAddress) internal override returns (IVerifier) {
        forkBeefy = new ForkBeefyFixture(MessageQueue(queueAddress), 1, false);
        beefyClient = forkBeefy.client();
        return forkBeefy.verifier();
    }

    // Solidity may cache block fields across vm.warp/roll; read the fixture clocks through vm.
    function control(uint64 sequence) private view returns (GovernanceControl memory) {
        return GovernanceControl(sequence, uint64(vm.getBlockTimestamp() + 2 days), 0, bytes32(0), bytes32(0));
    }

    function message(bool admin, bytes memory payload) private returns (VaraMessage memory) {
        return VaraMessage(
            messageNonce++,
            admin ? governanceAdmin.governance() : governancePauser.governance(),
            admin ? address(governanceAdmin) : address(governancePauser),
            payload
        );
    }

    function publish(VaraMessage memory msg_) private returns (uint256 height) {
        height = currentBlockNumber++;
        messageQueue.submitMerkleRoot(height, msg_.hash(), _rootProof(height, msg_.hash()));
    }

    function execute(VaraMessage memory msg_) private {
        uint256 height = publish(msg_);
        vm.warp(vm.getBlockTimestamp() + 1 hours);
        messageQueue.processMessage(height, 1, 0, msg_, new bytes32[](0));
    }

    function testReplayExpiryAndCanonicalEncoding() public {
        GovernanceControl memory cap = control(1);
        cap.validUntil = uint64(vm.getBlockTimestamp() + 1 hours);
        VaraMessage memory pause = message(true, GovernancePacker.pack(PauseProxyMessage(address(messageQueue)), cap));
        execute(pause); // Positive inclusive expiry boundary.
        assertTrue(messageQueue.paused());
        VaraMessage memory replay = message(true, pause.payload);
        uint256 height = publish(replay);
        vm.warp(vm.getBlockTimestamp() + 1 hours);
        vm.expectRevert(IMessageQueue.GovernanceControlExpired.selector);
        messageQueue.processMessage(height, 1, 0, replay, new bytes32[](0));
        replay.payload = GovernancePacker.pack(PauseProxyMessage(address(messageQueue)), control(1));
        height = publish(replay);
        vm.warp(vm.getBlockTimestamp() + 1 hours);
        vm.expectRevert(IMessageQueue.GovernanceControlConsumed.selector);
        messageQueue.processMessage(height, 1, 0, replay, new bytes32[](0));
        replay.payload =
            bytes.concat(GovernancePacker.pack(PauseProxyMessage(address(messageQueue)), control(2)), hex"00");
        height = publish(replay);
        vm.warp(vm.getBlockTimestamp() + 1 hours);
        vm.expectRevert(IMessageQueue.InvalidGovernanceControl.selector);
        messageQueue.processMessage(height, 1, 0, replay, new bytes32[](0));
        assertFalse(messageQueue.isProcessed(replay.nonce));
    }

    function testAdminCancelsExactPauserStreamAndStaleUpgrade() public {
        VaraMessage memory pending =
            message(false, GovernancePacker.pack(PauseProxyMessage(address(messageQueue)), control(5)));
        uint256 height = publish(pending);
        execute(
            message(
                true,
                GovernancePacker.cancel(
                    control(1), address(messageQueue), governancePauser.governance(), address(governancePauser), 5
                )
            )
        );
        vm.expectRevert(IMessageQueue.GovernanceControlConsumed.selector);
        messageQueue.processMessage(height, 1, 0, pending, new bytes32[](0));
        assertFalse(messageQueue.paused());
        MessageQueue replacement = new MessageQueue();
        VaraMessage memory upgrade = message(
            true,
            GovernancePacker.pack(UpgradeProxyMessage(address(messageQueue), address(replacement), ""), control(2))
        );
        height = publish(upgrade);
        execute(
            message(
                true,
                GovernancePacker.cancel(
                    control(3), address(messageQueue), governanceAdmin.governance(), address(governanceAdmin), 2
                )
            )
        );
        vm.expectRevert(IMessageQueue.GovernanceControlConsumed.selector);
        messageQueue.processMessage(height, 1, 0, upgrade, new bytes32[](0));
    }

    function testUnpauseCannotFrontRunNamedPendingUpgrade() public {
        execute(message(false, GovernancePacker.pack(PauseProxyMessage(address(messageQueue)), control(1))));
        MessageQueue replacement = new MessageQueue();
        VaraMessage memory upgrade = message(
            true,
            GovernancePacker.pack(UpgradeProxyMessage(address(messageQueue), address(replacement), ""), control(1))
        );
        uint256 upgradeHeight = publish(upgrade);
        GovernanceControl memory cap = control(2);
        cap.incidentEpoch = messageQueue.incidentEpoch(address(messageQueue));
        cap.requiredAdminAction = upgrade.hash();
        VaraMessage memory unpause =
            message(false, GovernancePacker.pack(UnpauseProxyMessage(address(messageQueue)), cap));
        uint256 unpauseHeight = publish(unpause);
        vm.warp(vm.getBlockTimestamp() + 5 minutes);
        vm.expectRevert(IMessageQueue.GovernanceDependencyMissing.selector);
        messageQueue.processMessage(unpauseHeight, 1, 0, unpause, new bytes32[](0));
        assertFalse(messageQueue.isProcessed(unpause.nonce));
        vm.warp(vm.getBlockTimestamp() + 1 hours);
        messageQueue.processMessage(upgradeHeight, 1, 0, upgrade, new bytes32[](0));
        messageQueue.processMessage(unpauseHeight, 1, 0, unpause, new bytes32[](0));
        assertFalse(messageQueue.paused());
        // A redundant authorized unpause consumes its capability without a second transition.
        cap.sequence = 3;
        VaraMessage memory redundant =
            message(false, GovernancePacker.pack(UnpauseProxyMessage(address(messageQueue)), cap));
        execute(redundant);
        assertTrue(messageQueue.isProcessed(redundant.nonce));
        execute(message(false, GovernancePacker.pack(PauseProxyMessage(address(messageQueue)), control(4))));
        cap.sequence = 5;
        VaraMessage memory stale =
            message(false, GovernancePacker.pack(UnpauseProxyMessage(address(messageQueue)), cap));
        uint256 height = publish(stale);
        vm.warp(vm.getBlockTimestamp() + 1 hours);
        vm.expectRevert(IMessageQueue.GovernanceDependencyMissing.selector);
        messageQueue.processMessage(height, 1, 0, stale, new bytes32[](0));
        assertTrue(messageQueue.paused());
    }

    function testRotationCannotReactivateRetiredSource() public {
        bytes32 oldSource = governanceAdmin.governance();
        execute(
            message(
                true,
                GovernancePacker.pack(
                    ChangeGovernanceMessage(bytes32(uint256(99))), control(1), address(governanceAdmin)
                )
            )
        );
        VaraMessage memory rotateBack = message(
            true, GovernancePacker.pack(ChangeGovernanceMessage(oldSource), control(1), address(governanceAdmin))
        );
        uint256 height = publish(rotateBack);
        vm.warp(vm.getBlockTimestamp() + 1 hours);
        vm.expectRevert(IMessageQueue.InvalidGovernanceControl.selector);
        messageQueue.processMessage(height, 1, 0, rotateBack, new bytes32[](0));
        assertEq(governanceAdmin.governance(), bytes32(uint256(99)));
    }

    function testRejectedDispatchRollsBackCapabilityAndOriginalNonce() public {
        // A real UUPS upgrade rejects a non-implementation; no mock handler substitutes for dispatch.
        VaraMessage memory bad = message(
            true,
            GovernancePacker.pack(UpgradeProxyMessage(address(messageQueue), address(governanceAdmin), ""), control(1))
        );
        uint256 height = publish(bad);
        vm.warp(vm.getBlockTimestamp() + 1 hours);
        vm.expectRevert();
        messageQueue.processMessage(height, 1, 0, bad, new bytes32[](0));
        assertFalse(messageQueue.isProcessed(bad.nonce));
        bytes32 stream = keccak256(abi.encode(bad.source, bad.destination));
        assertEq(messageQueue.executedControlSequence(stream), 0);
        execute(message(true, GovernancePacker.pack(PauseProxyMessage(address(messageQueue)), control(1))));
        assertEq(messageQueue.executedControlSequence(stream), 1);
    }

    function stageFixture()
        private
        returns (LegacyControl memory migration, LegacyControl memory recovery, address implementation)
    {
        implementation = address(new MessageQueue());
        uint256 cut = currentBlockNumber + 2;
        migration.message = message(
            true,
            abi.encodePacked(
                uint8(3),
                address(messageQueue),
                implementation,
                abi.encodeCall(MessageQueue.migrateToBeefy, (verifier, verifier, cut, cut + 1, beefyRecoveryTimelock))
            )
        );
        recovery.message = message(
            true,
            abi.encodePacked(
                uint8(3),
                address(messageQueue),
                implementation,
                abi.encodeCall(MessageQueue.recoverLegacyPublication, (verifier, cut))
            )
        );
        migration.sourceBlock = publish(migration.message);
        recovery.sourceBlock = publish(recovery.message);
        migration.totalLeaves = 1;
        recovery.totalLeaves = 1;
        migration.proof = new bytes32[](0);
        recovery.proof = new bytes32[](0);
        migration.validUntil = uint64(vm.getBlockTimestamp() + 2 days);
        recovery.validUntil = migration.validUntil;
        execute(message(true, GovernancePacker.pack(PauseProxyMessage(address(messageQueue)), control(1))));
        assertEq(messageQueue.maxBlockNumber(), cut);
        _rootProof(cut + 1, keccak256("live prebinding checkpoint"));
        // A bounded retained-storage fixture: preserve all real authenticated roots, clear only the new floor.
        // This exercises the production stage without a mocked verifier/handler; it is not legacy ZK qualification.
        vm.store(address(messageQueue), bytes32(uint256(14)), bytes32(0));
    }

    function installStage(LegacyControl memory migration, LegacyControl memory recovery, address implementation)
        private
    {
        vm.prank(address(governanceAdmin));
        messageQueue.upgradeToAndCall(
            implementation,
            abi.encodeCall(
                MessageQueue.stageLegacyMigration, (verifier, migration.sourceBlock + 2, migration, recovery)
            )
        );
    }

    function testFreezeRaceAndExpiryRollBackInstallation() public {
        (LegacyControl memory migration, LegacyControl memory recovery, address implementation) = stageFixture();
        bytes32 originalImplementation = vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT);
        vm.expectRevert(IMessageQueue.InvalidLegacyStage.selector);
        vm.prank(address(governanceAdmin));
        messageQueue.upgradeToAndCall(
            implementation,
            abi.encodeCall(
                MessageQueue.stageLegacyMigration, (verifier, migration.sourceBlock + 1, migration, recovery)
            )
        );
        assertEq(vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT), originalImplementation);
        uint64 expiry = recovery.validUntil;
        recovery.validUntil = uint64(vm.getBlockTimestamp() - 1);
        vm.expectRevert(IMessageQueue.InvalidLegacyStage.selector);
        installStage(migration, recovery, implementation);
        recovery.validUntil = expiry;
        installStage(migration, recovery, implementation);
        assertTrue(messageQueue.legacyPublicationFrozen());
        vm.expectRevert(IMessageQueue.LegacyPublicationFrozen.selector);
        messageQueue.submitMerkleRoot(migration.sourceBlock + 3, bytes32(uint256(1)), "");
        vm.expectRevert(IMessageQueue.LegacyPublicationFrozen.selector);
        messageQueue.submitEmptyQueueProgress(migration.sourceBlock + 3, "");
    }

    function testExactRecoveryConsumesAllLegacyExceptions() public {
        (LegacyControl memory migration, LegacyControl memory recovery, address implementation) = stageFixture();
        installStage(migration, recovery, implementation);
        // A full authenticated envelope is still not an exception while frozen.
        vm.expectRevert(IMessageQueue.InvalidMerkleProof.selector);
        VaraMessage memory alias_ = VaraMessage(
            recovery.message.nonce + 1000,
            recovery.message.source,
            recovery.message.destination,
            recovery.message.payload
        );
        messageQueue.processMessage(recovery.sourceBlock, 1, 0, alias_, new bytes32[](0));
        messageQueue.processMessage(recovery.sourceBlock, 1, 0, recovery.message, new bytes32[](0));
        assertFalse(messageQueue.legacyPublicationFrozen());
        assertEq(messageQueue.stagedMigrationMessage(), bytes32(0));
        assertEq(messageQueue.stagedRecoveryMessage(), bytes32(0));
        assertTrue(messageQueue.paused());
        assertEq(messageQueue.verifier(), address(verifier));
        vm.expectRevert(IMessageQueue.InvalidGovernanceControl.selector);
        messageQueue.processMessage(migration.sourceBlock, 1, 0, migration.message, new bytes32[](0));
    }

    function testNonemptySameQueueIdMigrationBarrier() public {
        migrationBarrierScenario(false);
    }

    function testEmptySameQueueIdMigrationBarrier() public {
        migrationBarrierScenario(true);
    }

    function migrationBarrierScenario(bool empty) private {
        (LegacyControl memory migration, LegacyControl memory recovery, address implementation) = stageFixture();
        installStage(migration, recovery, implementation);
        messageQueue.processMessage(migration.sourceBlock, 1, 0, migration.message, new bytes32[](0));
        assertFalse(messageQueue.legacyPublicationFrozen());
        assertEq(messageQueue.migrationAdminAction(), migration.message.hash());
        assertEq(messageQueue.stagedRecoveryMessage(), bytes32(0));
        uint256 sourceBlock = messageQueue.beefyRootMinimum();
        vm.warp(vm.getBlockTimestamp() + 1);
        vm.roll(vm.getBlockNumber() + 1);
        bytes32 root = empty ? bytes32(0) : keccak256("same QueueId post-upgrade progress");
        // QueueId stays 1 across migration; both proofs go through the real production verifier.
        bytes memory proof = forkBeefy.rootProofForEpoch(uint32(sourceBlock), root, 1);
        if (empty) messageQueue.submitEmptyQueueProgress(sourceBlock, proof);
        else messageQueue.submitMerkleRoot(sourceBlock, root, proof);
        (uint64 queueId,, uint256 publicationBlock, bytes32 recordedRoot) = messageQueue.publications(sourceBlock);
        assertEq(queueId, 1);
        assertEq(recordedRoot, root);
        vm.expectRevert(IMessageQueue.InvalidMigrationBarrier.selector);
        messageQueue.captureMigrationPublicationHash(sourceBlock); // Current block has no blockhash yet.
        bytes32 publicationHash = keccak256(abi.encode("canonical fixture publication", publicationBlock));
        vm.roll(vm.getBlockNumber() + 1);
        vm.setBlockhash(publicationBlock, publicationHash);
        assertEq(messageQueue.migrationPublicationBlockHash(sourceBlock), bytes32(0));
        assertEq(messageQueue.captureMigrationPublicationHash(sourceBlock), publicationHash);
        vm.roll(publicationBlock + 300);
        vm.warp(vm.getBlockTimestamp() + 3600);
        assertEq(blockhash(publicationBlock), bytes32(0));
        assertEq(messageQueue.captureMigrationPublicationHash(sourceBlock), publicationHash);
        vm.expectRevert(IMessageQueue.InvalidMigrationBarrier.selector);
        vm.prank(address(governanceAdmin));
        messageQueue.recordMigrationBarrier(sourceBlock, bytes32(uint256(1)));
        // Finality attestation must survive the real retained handler's one-hour admin delay.
        currentBlockNumber = sourceBlock + 1;
        address installedImplementation =
            address(uint160(uint256(vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT))));
        VaraMessage memory attestation = message(
            true,
            GovernancePacker.pack(
                UpgradeProxyMessage(
                    address(messageQueue),
                    installedImplementation,
                    abi.encodeCall(MessageQueue.recordMigrationBarrier, (sourceBlock, publicationHash))
                ),
                control(2)
            )
        );
        execute(attestation);
        bytes32 barrier = messageQueue.migrationBarrier();
        assertEq(
            barrier,
            keccak256(
                abi.encode(
                    block.chainid,
                    address(messageQueue),
                    migration.message.hash(),
                    sourceBlock,
                    queueId,
                    root,
                    publicationBlock,
                    publicationHash
                )
            )
        );
        GovernanceControl memory cap = control(1);
        cap.incidentEpoch = messageQueue.incidentEpoch(address(messageQueue));
        cap.requiredAdminAction = migration.message.hash();
        cap.requiredBarrier = barrier;
        VaraMessage memory resume =
            message(false, GovernancePacker.pack(UnpauseProxyMessage(address(messageQueue)), cap));
        execute(resume);
        assertFalse(messageQueue.paused());
        assertFalse(messageQueue.migrationResumePending());
    }
}
