// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {BeefyFixtureTest} from "./BeefyInterop.t.sol";
import {IAccessControl} from "@openzeppelin/contracts/access/IAccessControl.sol";
import {TimelockController} from "@openzeppelin/contracts/governance/TimelockController.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {ERC20Manager} from "src/ERC20Manager.sol";
import {GovernanceAdmin} from "src/GovernanceAdmin.sol";
import {GovernancePauser} from "src/GovernancePauser.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {VaraQueueRootVerifier} from "src/VaraQueueRootVerifier.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {VaraBridgeMetadata} from "src/beefy/VaraBridgeMetadata.sol";
import {CircleToken} from "src/erc20/CircleToken.sol";
import {WrappedVara} from "src/erc20/WrappedVara.sol";
import {IERC20Manager} from "src/interfaces/IERC20Manager.sol";
import {IGovernance} from "src/interfaces/IGovernance.sol";
import {IMessageHandlerMock} from "src/interfaces/IMessageHandlerMock.sol";
import {IMessageQueue, VaraMessage} from "src/interfaces/IMessageQueue.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";
import {Hasher} from "src/libraries/Hasher.sol";
import {GovernancePacker, UnpauseProxyMessage} from "src/libraries/packing/GovernancePacker.sol";
import {BeefyClientMock} from "src/mocks/BeefyClientMock.sol";
import {MessageHandlerMock} from "src/mocks/MessageHandlerMock.sol";

contract VaraQueueRootVerifierTest is BeefyFixtureTest {
    using GovernancePacker for UnpauseProxyMessage;

    TimelockController internal recovery;
    BeefyClient internal client;
    VaraQueueRootVerifier internal verifier;
    MessageQueue internal queue;
    MessageHandlerMock internal receiver;

    function setUp() public {
        fixtures = vm.readFile("test/fixtures/beefy-interop.json");
        address[] memory proposers = new address[](1);
        proposers[0] = address(this);
        recovery = new TimelockController(1 hours, proposers, proposers, address(0));
        uint256 startingNonce = vm.getNonce(address(this));
        address predictedQueue = vm.computeCreateAddress(address(this), startingNonce + 5);
        client = newClient(0, block.chainid, predictedQueue);

        MessageQueue implementation = new MessageQueue();
        GovernanceAdmin admin = new GovernanceAdmin(
            bytes32(uint256(33)), WrappedVara(address(0)), MessageQueue(predictedQueue), ERC20Manager(address(0))
        );
        GovernancePauser pauser = new GovernancePauser(
            bytes32(uint256(44)), WrappedVara(address(0)), MessageQueue(predictedQueue), ERC20Manager(address(0))
        );
        assertEq(vm.computeCreateAddress(address(this), vm.getNonce(address(this)) + 1), predictedQueue);
        verifier = new VaraQueueRootVerifier(client, predictedQueue, block.chainid);
        queue = MessageQueue(
            address(
                new ERC1967Proxy(
                    address(implementation),
                    abi.encodeCall(
                        MessageQueue.initializeBeefy,
                        (
                            IGovernance(address(admin)),
                            IGovernance(address(pauser)),
                            address(this),
                            new address[](0),
                            verifier,
                            address(recovery)
                        )
                    )
                )
            )
        );
        assertEq(address(queue), predictedQueue);
        assertEq(verifier.messageQueue(), address(queue));
        receiver = new MessageHandlerMock();
    }

    function verifyAsQueue(VaraQueueRootVerifier target, uint256 sourceBlock, bytes32 root, bytes memory proof)
        internal
        returns (bool valid)
    {
        vm.prank(target.messageQueue());
        return target.safeVerifyProof(sourceBlock, root, proof);
    }

    function testSharedFixturesVerifyTypedQueueRoots() public {
        vm.pauseGasMetering();
        for (uint256 i; i < caseCount(); i++) {
            if (isNativeOnly(i)) continue;
            BeefyClient fixtureClient = newClient(i);
            VaraQueueRootVerifier fixtureVerifier =
                new VaraQueueRootVerifier(fixtureClient, fixtureClient.destinationQueue(), block.chainid);
            QueueProof memory p = queueProof(i);
            bytes32 root = fixtureHash(i, "queueRoot");
            acceptFixture(fixtureClient, i);
            bool valid = verifyAsQueue(fixtureVerifier, p.leaf.parentNumber, root, fixtureBytes(i, "queueProof"));
            assertEq(valid, root != bytes32(0));
        }
        vm.resumeGasMetering();
    }

    function testConstructorRequiresDeployedClientAndBoundQueue() public {
        vm.expectRevert();
        new VaraQueueRootVerifier(BeefyClient(address(0)), address(queue), block.chainid);
        vm.expectRevert();
        new VaraQueueRootVerifier(client, address(0), block.chainid);
        vm.expectRevert();
        new VaraQueueRootVerifier(client, address(queue), block.chainid + 1);
        vm.expectRevert();
        new VaraQueueRootVerifier(client, address(0x4444), block.chainid);
        assertEq(verifier.messageQueue(), address(queue));
        assertEq(verifier.destinationChainId(), block.chainid);
    }

    function testFrozenPolicyRejectsEveryWeakerDeploymentClient() public {
        BeefyClient candidate = newClient(0, block.chainid, address(queue));
        bytes4[5] memory selectors = [
            candidate.minNumRequiredSignatures.selector,
            candidate.fiatShamirRequiredSignatures.selector,
            candidate.MAX_VALIDATORS.selector,
            candidate.randaoCommitDelay.selector,
            candidate.randaoCommitExpiration.selector
        ];
        uint256[5] memory weakened = [uint256(85), 85, 257, 1, 25];
        for (uint256 i; i < selectors.length; i++) {
            vm.mockCall(address(candidate), abi.encodeWithSelector(selectors[i]), abi.encode(weakened[i]));
            vm.expectRevert();
            new VaraQueueRootVerifier(candidate, address(queue), block.chainid);
            vm.clearMockedCalls();
        }
    }

    function testFreshBeefyInitializerValidatesBindingFloorAndRecovery() public {
        MessageQueue fresh = MessageQueue(address(new InitializerTestProxy(address(new MessageQueue()))));
        IGovernance admin = IGovernance(queue.governanceAdmin());
        IGovernance pauser = IGovernance(queue.governancePauser());
        vm.expectRevert(IMessageQueue.InvalidBeefyVerifier.selector);
        fresh.initializeBeefy(admin, pauser, address(this), new address[](0), verifier, address(recovery));
        BeefyClient boundClient = newClient(0, block.chainid, address(fresh));
        VaraQueueRootVerifier boundVerifier = new VaraQueueRootVerifier(boundClient, address(fresh), block.chainid);
        vm.expectRevert(IMessageQueue.InvalidBeefyRecoveryTimelock.selector);
        fresh.initializeBeefy(admin, pauser, address(this), new address[](0), boundVerifier, address(0xBEEF));
        address[] memory proposers = new address[](1);
        proposers[0] = address(this);
        TimelockController immediate = new TimelockController(0, proposers, proposers, address(0));
        vm.expectRevert(IMessageQueue.InvalidBeefyRecoveryTimelock.selector);
        fresh.initializeBeefy(admin, pauser, address(this), new address[](0), boundVerifier, address(immediate));
        fresh.initializeBeefy(admin, pauser, address(this), new address[](0), boundVerifier, address(recovery));
        assertEq(fresh.beefyRootMinimum(), boundClient.mmrStartBlock());
        assertEq(fresh.verifier(), address(boundVerifier));
        assertEq(fresh.beefyRecoveryTimelock(), address(recovery));
        assertTrue(fresh.hasRole(fresh.DEFAULT_ADMIN_ROLE(), address(admin)));
        assertFalse(fresh.hasRole(fresh.DEFAULT_ADMIN_ROLE(), address(recovery)));
        vm.expectRevert();
        fresh.initializeBeefy(admin, pauser, address(this), new address[](0), boundVerifier, address(recovery));
    }

    function testQueueAndChainBindings() public {
        bytes32 root = fixtureHash(0, "queueRoot");
        QueueProof memory p = customQueueProof(103, root);
        bytes memory proof = encodeProof(p);
        vm.prank(address(this));
        assertFalse(verifier.safeVerifyProof(p.leaf.parentNumber, root, proof));
        vm.prank(address(0xBEEF));
        assertFalse(verifier.safeVerifyProof(p.leaf.parentNumber, root, proof));

        uint256 chainId = block.chainid;
        vm.chainId(chainId + 1);
        vm.prank(address(queue));
        assertFalse(verifier.safeVerifyProof(p.leaf.parentNumber, root, proof));
        vm.chainId(chainId);

        assertFalse(verifier.verifyProof(p.leaf.parentNumber, root, proof));
    }

    function testCanonicalBoundaryRejections() public {
        bytes32 root = fixtureHash(0, "queueRoot");
        QueueProof memory p = customQueueProof(103, root);
        bytes memory encoded = encodeProof(p);
        bytes memory pristine = abi.encode(p);
        uint32 sourceBlock = p.leaf.parentNumber;
        assertFalse(verifier.safeVerifyProof(sourceBlock, root, encoded));
        assertTrue(verifyAsQueue(verifier, sourceBlock, root, encoded));

        for (uint256 mutation; mutation < 19; mutation++) {
            QueueProof memory bad = abi.decode(pristine, (QueueProof));
            if (mutation == 0) bad.proofVersion = 1;
            if (mutation == 1) bad.bridgeVersion = 1;
            if (mutation == 2) bad.initialized = false;
            if (mutation == 3) bad.bridgeDomain ^= bytes32(uint256(1));
            if (mutation == 4) bad.sourceTimestampMs++;
            if (mutation == 5) bad.queueId++;
            if (mutation == 6) bad.anchorBlock++;
            if (mutation == 7) bad.anchorRoot ^= bytes32(uint256(1));
            if (mutation == 8) bad.leaf.version = 1;
            if (mutation == 9) bad.leaf.parentNumber++;
            if (mutation == 10) bad.leaf.parentHash ^= bytes32(uint256(1));
            if (mutation == 11) bad.leaf.nextAuthoritySetID++;
            if (mutation == 12) bad.leaf.nextAuthoritySetLen++;
            if (mutation == 13) bad.leaf.nextAuthoritySetRoot ^= bytes32(uint256(1));
            if (mutation == 14) bad.leaf.parachainHeadsRoot ^= bytes32(uint256(1));
            if (mutation == 15) bad.order = 1;
            if (mutation == 16) bad.proofVersion = 0;
            if (mutation == 17) bad.bridgeVersion = 0;
            if (mutation == 18) bad.items[0] ^= bytes32(uint256(1));
            assertFalse(verifyAsQueue(verifier, sourceBlock, root, encodeProof(bad)), vm.toString(mutation));
            vm.expectRevert(IMessageQueue.InvalidQueueRootProof.selector);
            queue.submitMerkleRoot(sourceBlock, root, encodeProof(bad));
            assertEq(queue.getMerkleRoot(sourceBlock), bytes32(0));
            assertEq(queue.genesisBlock(), 0);
            assertFalse(queue.isProcessed(7));
        }
        assertFalse(verifyAsQueue(verifier, sourceBlock, root, bytes.concat(encoded, hex"00")), "trailing byte");
        assertFalse(verifyAsQueue(verifier, sourceBlock, root, hex"00"), "short proof");
        assertFalse(verifyAsQueue(verifier, sourceBlock, root, abi.encode(p)), "noncanonical tuple");
        assertFalse(verifyAsQueue(verifier, sourceBlock, bytes32(0), encoded), "zero root");
        assertFalse(verifyAsQueue(verifier, sourceBlock, root ^ bytes32(uint256(1)), encoded), "wrong root");
        assertFalse(verifyAsQueue(verifier, sourceBlock + 1, root, encoded), "wrong block");
        assertFalse(verifyAsQueue(verifier, uint256(sourceBlock) + (uint256(1) << 32), root, encoded), "uint32 alias");
        assertFalse(verifyAsQueue(verifier, type(uint256).max, root, encoded), "overflow");
        // Nonzero padding of a narrow ABI value and an incorrect dynamic offset.
        encoded[30] = bytes1(uint8(1));
        assertFalse(verifyAsQueue(verifier, sourceBlock, root, encoded), "narrow ABI padding");
        encoded = encodeProof(p);
        encoded[511] = bytes1(uint8(0x21));
        assertFalse(verifyAsQueue(verifier, sourceBlock, root, encoded), "dynamic offset");
        encoded = encodeProof(p);
        assembly ("memory-safe") { mstore(encoded, sub(mload(encoded), 1)) }
        assertFalse(verifyAsQueue(verifier, sourceBlock, root, encoded), "truncated proof");
        assertTrue(verifyAsQueue(verifier, sourceBlock, root, encodeProof(p)));
        queue.submitMerkleRoot(sourceBlock, root, encodeProof(p));
        assertEq(queue.getMerkleRoot(sourceBlock), root);
        assertEq(queue.genesisBlock(), sourceBlock);
    }

    function testMockCheckpointStillRequiresExactMmrWitness() public {
        bytes32 root = fixtureHash(0, "queueRoot");
        QueueProof memory p = customQueueProof(103, root);
        uint128 count = uint128(fixtureUint(0, "validatorCount"));
        bytes32 authorityRoot = fixtureHash(0, "authorityRoot");
        BeefyClientMock mockClient = new BeefyClientMock(
            client.sourceDomain(),
            block.chainid,
            address(queue),
            client.mmrStartBlock(),
            client.mmrStartBlock() + 1,
            client.lastAuthenticatedSourceTimestampMs(),
            BeefyClient.ValidatorSet(0, count, authorityRoot),
            BeefyClient.ValidatorSet(1, count, authorityRoot)
        );
        mockClient.setCheckpoint(p.anchorBlock, p.anchorRoot, client.lastAuthenticatedSourceTimestampMs());
        VaraQueueRootVerifier mockVerifier = new VaraQueueRootVerifier(mockClient, address(queue), block.chainid);
        uint256 sourceBlock = p.leaf.parentNumber;
        assertTrue(verifyAsQueue(mockVerifier, sourceBlock, root, encodeProof(p)));
        assertFalse(verifyAsQueue(mockVerifier, sourceBlock + 1, root, encodeProof(p)));
        p.items[0] ^= bytes32(uint256(1));
        assertFalse(verifyAsQueue(mockVerifier, sourceBlock, root, encodeProof(p)));
        p.items[0] ^= bytes32(uint256(1));
        p.leaf.parentNumber++;
        assertFalse(verifyAsQueue(mockVerifier, sourceBlock + 1, root, encodeProof(p)));
    }

    function testAuthenticatedHistoricalLeafBelowMmrStartIsRejected() public {
        bytes32 root = fixtureHash(0, "queueRoot");
        uint32 belowStart = uint32(client.mmrStartBlock() - 1);
        QueueProof memory p = customQueueProofFor(
            client,
            belowStart,
            103,
            root,
            uint64(vm.parseJsonUint(fixtures, ".cases[0].sourceTimestampMs")),
            uint64(block.timestamp * 1000)
        );
        assertTrue(client.verifyMMRLeafProof(keccak256(leafBytes(p.leaf)), p.items, p.order));
        assertFalse(verifyAsQueue(verifier, belowStart, root, encodeProof(p)));
        vm.expectRevert(
            abi.encodeWithSelector(IMessageQueue.BlockNumberBelowMinimum.selector, belowStart, client.mmrStartBlock())
        );
        queue.submitMerkleRoot(belowStart, root, encodeProof(p));
        assertEq(queue.genesisBlock(), 0);
        assertEq(queue.getMerkleRoot(belowStart), bytes32(0));
    }

    function customQueueProof(uint64 anchorBlock, bytes32 messageHash) internal returns (QueueProof memory p) {
        uint64 historicalTimestamp = uint64(vm.parseJsonUint(fixtures, ".cases[0].sourceTimestampMs"));
        uint64 freshnessTimestamp = uint64(block.timestamp * 1000);
        if (freshnessTimestamp < client.lastAuthenticatedSourceTimestampMs()) {
            freshnessTimestamp = client.lastAuthenticatedSourceTimestampMs();
        }
        return customQueueProofFor(
            client, queueProof(0).leaf.parentNumber, anchorBlock, messageHash, historicalTimestamp, freshnessTimestamp
        );
    }

    function customQueueProof(
        uint64 anchorBlock,
        bytes32 messageHash,
        uint64 historicalTimestamp,
        uint64 freshnessTimestamp
    ) internal returns (QueueProof memory p) {
        return customQueueProofFor(
            client, queueProof(0).leaf.parentNumber, anchorBlock, messageHash, historicalTimestamp, freshnessTimestamp
        );
    }

    function testHistoricalBackfillHasIndependentMaturity() public {
        _historicalBackfill(false);
    }

    function testIdenticalHistoricalBackfillHasIndependentMaturity() public {
        _historicalBackfill(true);
    }

    function _historicalBackfill(bool identical) private {
        assertEq(queue.beefyRootMinimum(), 100);
        VaraMessage memory message = VaraMessage(710, bytes32(uint256(77)), address(receiver), bytes("backfill"));
        bytes32 root = keccak256(abi.encodePacked(message.nonce, message.source, message.destination, message.payload));
        uint64 timestamp = uint64(vm.getBlockTimestamp() * 1000);
        bytes32 firstRoot = identical ? root : bytes32(uint256(0x1234));
        QueueProof memory first = customQueueProofFor(client, 102, 103, firstRoot, timestamp, timestamp);
        queue.submitMerkleRoot(102, firstRoot, encodeProof(first));
        uint256 firstTime = vm.getBlockTimestamp();
        vm.warp(firstTime + queue.PROCESS_USER_MESSAGE_DELAY());
        QueueProof memory historical =
            customQueueProofFor(client, 100, 104, root, timestamp, uint64(vm.getBlockTimestamp() * 1000));
        queue.submitMerkleRoot(100, root, encodeProof(historical));
        assertEq(queue.genesisBlock(), 102);
        assertEq(queue.maxBlockNumber(), 102);
        assertEq(queue.getMerkleRootTimestampForBlock(102), firstTime);
        uint256 registered = vm.getBlockTimestamp();
        assertEq(queue.getMerkleRootTimestampForBlock(100), registered);
        vm.expectRevert(IMessageQueue.MerkleRootDelayNotPassed.selector);
        queue.processMessage(100, 1, 0, message, new bytes32[](0));
        vm.warp(registered + queue.PROCESS_USER_MESSAGE_DELAY() - 1);
        vm.expectRevert(IMessageQueue.MerkleRootDelayNotPassed.selector);
        queue.processMessage(100, 1, 0, message, new bytes32[](0));
        vm.warp(registered + queue.PROCESS_USER_MESSAGE_DELAY());
        queue.processMessage(100, 1, 0, message, new bytes32[](0));
        assertTrue(queue.isProcessed(message.nonce));
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MessageAlreadyProcessed.selector, message.nonce));
        queue.processMessage(identical ? 102 : 100, 1, 0, message, new bytes32[](0));
    }

    function testBackfillAfterEmptyProgressAndForwardBoundary() public {
        bytes32 root = bytes32(uint256(0x712));
        uint64 timestamp = uint64(block.timestamp * 1000);
        QueueProof memory initial = customQueueProofFor(client, 100, 103, root, timestamp, timestamp);
        queue.submitMerkleRoot(100, root, encodeProof(initial));
        vm.expectRevert(
            abi.encodeWithSelector(IMessageQueue.BlockNumberBelowMinimum.selector, uint256(99), uint256(100))
        );
        queue.submitMerkleRoot(99, root, "");
        vm.expectRevert(
            abi.encodeWithSelector(IMessageQueue.BlockNumberTooFar.selector, uint256(57701), uint256(57700))
        );
        queue.submitMerkleRoot(57701, root, "");
        uint256 saved = vm.snapshotState();
        QueueProof memory boundary = customQueueProofFor(client, 57700, 57701, root, timestamp, timestamp);
        queue.submitMerkleRoot(57700, root, encodeProof(boundary));
        assertEq(queue.maxBlockNumber(), 57700);
        assertTrue(vm.revertToState(saved));
        QueueProof memory progress = customQueueProofFor(client, 104, 105, bytes32(0), timestamp, timestamp);
        queue.submitEmptyQueueProgress(104, encodeProof(progress));
        QueueProof memory backfill = customQueueProofFor(client, 103, 106, root, timestamp, timestamp);
        queue.submitMerkleRoot(103, root, encodeProof(backfill));
        assertEq(queue.genesisBlock(), 100);
        assertEq(queue.maxBlockNumber(), 104);
        assertEq(queue.getMerkleRoot(103), root);
        assertEq(queue.getMerkleRoot(104), bytes32(0));
        vm.expectRevert(IMessageQueue.EmptyQueueProgressNotForward.selector);
        queue.submitEmptyQueueProgress(104, encodeProof(progress));
        vm.expectRevert(IMessageQueue.EmptyQueueProgressNotForward.selector);
        queue.submitEmptyQueueProgress(103, encodeProof(backfill));
    }

    function testHistoricalQueueTimestampAllowedUnderLiveAnchor() public {
        uint64 baseTimestamp = uint64(vm.parseJsonUint(fixtures, ".cases[0].sourceTimestampMs"));
        uint64 historicalTimestamp = baseTimestamp - 86_401_000;
        uint64 freshnessTimestamp =
            uint64(vm.parseJsonUint(fixtures, ".cases[0].freshnessProof.sourceTimestampMs")) + 3_000;
        bytes32 messageHash = bytes32(uint256(0x5678));
        QueueProof memory p = customQueueProof(103, messageHash, historicalTimestamp, freshnessTimestamp);
        assertTrue(client.isLive());
        queue.submitMerkleRoot(p.leaf.parentNumber, messageHash, encodeProof(p));
        assertEq(queue.getMerkleRoot(p.leaf.parentNumber), messageHash);
    }

    function testExpiredClientRejectsNewRootButOldRootRemainsDeliverable() public {
        VaraMessage memory message =
            VaraMessage(9, bytes32(uint256(99)), address(receiver), bytes("expired client keeps old root live"));
        bytes32 messageHash =
            keccak256(abi.encodePacked(message.nonce, message.source, message.destination, message.payload));
        QueueProof memory p = customQueueProof(103, messageHash);
        uint256 sourceBlock = p.leaf.parentNumber;
        queue.submitMerkleRoot(sourceBlock, messageHash, encodeProof(p));

        bytes32 nextRoot = bytes32(uint256(0x9999));
        QueueProof memory nextProof = customQueueProof(104, nextRoot);
        vm.warp(block.timestamp + 86_407);
        assertFalse(client.isLive());
        vm.expectRevert(IMessageQueue.InvalidQueueRootProof.selector);
        queue.submitMerkleRoot(nextProof.leaf.parentNumber, nextRoot, encodeProof(nextProof));
        assertEq(queue.getMerkleRoot(sourceBlock), messageHash);

        queue.processMessage(sourceBlock, 1, 0, message, new bytes32[](0));
        assertTrue(queue.isProcessed(message.nonce));
    }

    function testProofOrderUsesEntireUint256AndRejectsUnusedBits() public {
        QueueProof memory p = customQueueProof(103, bytes32(uint256(0x1234)));
        VaraBridgeMetadata.Snapshot memory historical = VaraBridgeMetadata.Snapshot({
            version: p.proofVersion,
            bridgeDomain: p.bridgeDomain,
            sourceTimestampMs: p.sourceTimestampMs,
            initialized: p.initialized,
            queueId: p.queueId,
            queueRoot: bytes32(uint256(0x1234))
        });
        assertEq(VaraBridgeMetadata.hash(historical), p.leaf.parachainHeadsRoot);
        bytes32 root = bytes32(uint256(0x1234));
        assertTrue(verifyAsQueue(verifier, p.leaf.parentNumber, root, encodeProof(p)));
        p.order = uint256(1) << 255;
        assertFalse(verifyAsQueue(verifier, p.leaf.parentNumber, root, encodeProof(p)));
        p.order = 0;
        p.items = new bytes32[](257);
        assertFalse(verifyAsQueue(verifier, p.leaf.parentNumber, root, encodeProof(p)));
    }

    function testUnchangedQueueMaturityReceiverAndReplay() public {
        VaraMessage memory message =
            VaraMessage(7, bytes32(uint256(77)), address(receiver), bytes("real receiver payload"));
        bytes32 messageHash =
            keccak256(abi.encodePacked(message.nonce, message.source, message.destination, message.payload));
        QueueProof memory p = customQueueProof(103, messageHash);
        uint256 sourceBlock = p.leaf.parentNumber;
        queue.submitMerkleRoot(sourceBlock, messageHash, encodeProof(p));
        assertEq(queue.getMerkleRoot(sourceBlock), messageHash);
        vm.expectRevert(IMessageQueue.MerkleRootDelayNotPassed.selector);
        queue.processMessage(sourceBlock, 1, 0, message, new bytes32[](0));
        vm.warp(block.timestamp + queue.PROCESS_USER_MESSAGE_DELAY());
        vm.expectEmit(true, false, false, true, address(receiver));
        emit IMessageHandlerMock.MessageHandled(message.source, message.payload);
        queue.processMessage(sourceBlock, 1, 0, message, new bytes32[](0));
        assertTrue(queue.isProcessed(message.nonce));
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MessageAlreadyProcessed.selector, message.nonce));
        queue.processMessage(sourceBlock, 1, 0, message, new bytes32[](0));
    }

    function testStaleAnchorFailsThenSameSourceCanBeReproved() public {
        bytes32 root = bytes32(uint256(0x1234));
        QueueProof memory oldProof = customQueueProof(103, root);
        assertTrue(verifyAsQueue(verifier, oldProof.leaf.parentNumber, root, encodeProof(oldProof)));
        QueueProof memory newer = customQueueProof(104, root);
        vm.expectRevert(IMessageQueue.InvalidQueueRootProof.selector);
        queue.submitMerkleRoot(oldProof.leaf.parentNumber, root, encodeProof(oldProof));
        assertEq(queue.getMerkleRoot(oldProof.leaf.parentNumber), bytes32(0));
        assertTrue(verifyAsQueue(verifier, newer.leaf.parentNumber, root, encodeProof(newer)));
        queue.submitMerkleRoot(newer.leaf.parentNumber, root, encodeProof(newer));
        assertEq(queue.getMerkleRoot(newer.leaf.parentNumber), root);
    }

    function testAuthenticatedEmptyQueueProgressPreservesRootMaturityAndReplay() public {
        uint256 floor = client.mmrStartBlock();
        vm.expectRevert(IMessageQueue.EmptyQueueNotInitialized.selector);
        queue.submitEmptyQueueProgress(floor, bytes(""));
        vm.expectRevert(IMessageQueue.InvalidMerkleRoot.selector);
        queue.submitMerkleRoot(100, bytes32(0), bytes(""));

        VaraMessage memory message = VaraMessage(701, bytes32(uint256(0x701)), address(receiver), bytes("still mature"));
        bytes32 root = keccak256(abi.encodePacked(message.nonce, message.source, message.destination, message.payload));
        QueueProof memory rootProof = customQueueProof(103, root);
        uint32 rootBlock = rootProof.leaf.parentNumber;
        queue.submitMerkleRoot(rootBlock, root, encodeProof(rootProof));
        uint256 rootTimestamp = queue.getMerkleRootTimestampForBlock(rootBlock);
        uint256 genesis = queue.genesisBlock();

        uint256 maxProgressBlock = uint256(rootBlock) + queue.MAX_BLOCK_DISTANCE();
        uint256 tooFarBlock = maxProgressBlock + 1;
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.BlockNumberTooFar.selector, tooFarBlock, maxProgressBlock));
        queue.submitEmptyQueueProgress(tooFarBlock, bytes(""));
        vm.expectRevert(IMessageQueue.InvalidEmptyQueueProgressProof.selector);
        queue.submitEmptyQueueProgress(uint256(rootBlock) + 1, bytes("not an authenticated snapshot"));

        uint32 progressBlock = rootBlock + 1;
        uint64 historicalTimestamp = uint64(vm.parseJsonUint(fixtures, ".cases[0].sourceTimestampMs"));
        uint64 freshnessTimestamp = uint64(block.timestamp * 1000);
        if (freshnessTimestamp < client.lastAuthenticatedSourceTimestampMs()) {
            freshnessTimestamp = client.lastAuthenticatedSourceTimestampMs();
        }
        QueueProof memory progressProof = customQueueProofFor(
            client, progressBlock, rootProof.anchorBlock + 1, bytes32(0), historicalTimestamp, freshnessTimestamp
        );

        vm.expectRevert(IMessageQueue.InvalidEmptyQueueProgressProof.selector);
        queue.submitEmptyQueueProgress(uint256(progressBlock) + 1, encodeProof(progressProof));
        vm.expectEmit(true, false, false, false, address(queue));
        emit IMessageQueue.EmptyQueueProgress(progressBlock);
        queue.submitEmptyQueueProgress(progressBlock, encodeProof(progressProof));
        assertEq(queue.genesisBlock(), genesis);
        assertEq(queue.maxBlockNumber(), progressBlock);
        assertEq(queue.getMerkleRoot(rootBlock), root);
        assertEq(queue.getMerkleRootTimestampForBlock(rootBlock), rootTimestamp);
        assertEq(queue.getMerkleRoot(progressBlock), bytes32(0));
        assertFalse(queue.isProcessed(message.nonce));

        vm.expectRevert(IMessageQueue.MerkleRootDelayNotPassed.selector);
        queue.processMessage(rootBlock, 1, 0, message, new bytes32[](0));
        vm.warp(rootTimestamp + queue.PROCESS_USER_MESSAGE_DELAY());
        queue.processMessage(rootBlock, 1, 0, message, new bytes32[](0));
        assertTrue(queue.isProcessed(message.nonce));
    }

    function testEmptyQueueProgressCanBridgeRepeatedRootDistanceWindows() public {
        bytes32 root = bytes32(uint256(0xabcdef));
        QueueProof memory initial = customQueueProof(103, root);
        uint32 initialBlock = initial.leaf.parentNumber;
        queue.submitMerkleRoot(initialBlock, root, encodeProof(initial));
        uint64 historicalTimestamp = uint64(vm.parseJsonUint(fixtures, ".cases[0].sourceTimestampMs"));
        uint64 freshnessTimestamp = uint64(block.timestamp * 1000);
        if (freshnessTimestamp < client.lastAuthenticatedSourceTimestampMs()) {
            freshnessTimestamp = client.lastAuthenticatedSourceTimestampMs();
        }

        uint32 progressBlock = initialBlock;
        for (uint32 step = 1; step <= 3; step++) {
            progressBlock = initialBlock + step * uint32(queue.MAX_BLOCK_DISTANCE());
            QueueProof memory progressProof = customQueueProofFor(
                client, progressBlock, uint64(progressBlock + 1), bytes32(0), historicalTimestamp, freshnessTimestamp
            );
            queue.submitEmptyQueueProgress(progressBlock, encodeProof(progressProof));
            assertEq(queue.maxBlockNumber(), progressBlock);
            assertEq(queue.getMerkleRoot(progressBlock), bytes32(0));
        }

        uint32 nextRootBlock = progressBlock + 1;
        bytes32 nextRoot = bytes32(uint256(0x123456));
        QueueProof memory nextRootProof = customQueueProofFor(
            client, nextRootBlock, uint64(nextRootBlock + 1), nextRoot, historicalTimestamp, freshnessTimestamp
        );
        queue.submitMerkleRoot(nextRootBlock, nextRoot, encodeProof(nextRootProof));
        assertEq(queue.getMerkleRoot(nextRootBlock), nextRoot);
        assertEq(queue.maxBlockNumber(), nextRootBlock);
    }

    function testTimelockedExpiryRecoveryRetainsEscrowAndClaims() public {
        CircleToken token = new CircleToken(address(this));
        IERC20Manager.TokenInfo[] memory tokens = new IERC20Manager.TokenInfo[](1);
        tokens[0] = IERC20Manager.TokenInfo(address(token), IERC20Manager.TokenType.Ethereum);
        bytes32 vftManager = bytes32(uint256(0x909));
        ERC20Manager manager = ERC20Manager(
            address(
                new ERC1967Proxy(
                    address(new ERC20Manager()),
                    abi.encodeCall(
                        ERC20Manager.initialize,
                        (
                            IGovernance(queue.governanceAdmin()),
                            IGovernance(queue.governancePauser()),
                            queue,
                            vftManager,
                            tokens
                        )
                    )
                )
            )
        );
        token.mint(address(this), 30);
        token.approve(address(manager), 30);
        manager.requestBridging(address(token), 30, vftManager);
        assertEq(token.balanceOf(address(manager)), 30);

        VaraMessage memory processed = VaraMessage(
            702,
            vftManager,
            address(manager),
            abi.encodePacked(bytes32(uint256(1)), address(this), address(token), uint256(10))
        );
        VaraMessage memory retained = VaraMessage(
            703,
            vftManager,
            address(manager),
            abi.encodePacked(bytes32(uint256(2)), address(this), address(token), uint256(20))
        );
        bytes32 firstLeaf =
            keccak256(abi.encodePacked(processed.nonce, processed.source, processed.destination, processed.payload));
        bytes32 secondLeaf =
            keccak256(abi.encodePacked(retained.nonce, retained.source, retained.destination, retained.payload));
        bytes32 root = keccak256(abi.encodePacked(firstLeaf, secondLeaf));
        QueueProof memory oldProof = customQueueProof(103, root);
        uint32 rootBlock = oldProof.leaf.parentNumber;
        queue.submitMerkleRoot(rootBlock, root, encodeProof(oldProof));
        uint256 rootTimestamp = queue.getMerkleRootTimestampForBlock(rootBlock);
        bytes32[] memory siblings = new bytes32[](1);
        siblings[0] = secondLeaf;
        vm.expectRevert(IMessageQueue.MerkleRootDelayNotPassed.selector);
        queue.processMessage(rootBlock, 2, 0, processed, siblings);
        vm.warp(rootTimestamp + queue.PROCESS_USER_MESSAGE_DELAY());
        queue.processMessage(rootBlock, 2, 0, processed, siblings);
        assertEq(token.balanceOf(address(manager)), 20);
        assertTrue(queue.isProcessed(processed.nonce));
        assertFalse(queue.isProcessed(retained.nonce));

        MessageQueue implementation = new MessageQueue();
        bytes32 adminRole = queue.DEFAULT_ADMIN_ROLE();
        vm.expectRevert(
            abi.encodeWithSelector(IAccessControl.AccessControlUnauthorizedAccount.selector, address(this), adminRole)
        );
        queue.upgradeToAndCall(address(implementation), "");

        uint256 floor = queue.beefyRootMinimum();
        vm.warp(vm.getBlockTimestamp() + client.MAX_SOURCE_AGE_MS() / 1000 + 1);
        assertFalse(client.isLive());
        VaraQueueRootVerifier replacement = migrationVerifier(queue);
        VaraMessage memory unpause = VaraMessage(
            704,
            GovernancePauser(queue.governancePauser()).governance(),
            queue.governancePauser(),
            UnpauseProxyMessage(address(queue)).pack()
        );
        QueueProof memory recoveryProof = customQueueProofFor(
            replacement.beefyClient(),
            rootBlock + 1,
            client.latestBeefyBlock() + 2,
            Hasher.hash(unpause),
            uint64(vm.getBlockTimestamp() * 1000),
            uint64(vm.getBlockTimestamp() * 1000)
        );
        bytes memory data =
            abi.encodeCall(MessageQueue.recoverBeefyVerifier, (IVerifier(address(verifier)), replacement, rootBlock));
        vm.expectRevert(IMessageQueue.NotBeefyRecoveryTimelock.selector);
        queue.recoverBeefyVerifier(verifier, replacement, rootBlock);
        bytes32 salt = keccak256("expired-client-recovery");
        recovery.schedule(address(queue), 0, data, bytes32(0), salt, recovery.getMinDelay());
        vm.expectRevert();
        recovery.execute(address(queue), 0, data, bytes32(0), salt);
        assertEq(queue.verifier(), address(verifier));
        assertFalse(queue.paused());
        vm.warp(vm.getBlockTimestamp() + recovery.getMinDelay());
        recovery.execute(address(queue), 0, data, bytes32(0), salt);
        assertTrue(queue.hasRole(adminRole, queue.governanceAdmin()));
        assertFalse(queue.hasRole(adminRole, address(recovery)));
        assertEq(queue.beefyRootMinimum(), floor);
        assertEq(queue.verifier(), address(replacement));
        assertEq(queue.getMerkleRoot(rootBlock), root);
        assertEq(queue.getMerkleRootTimestampForBlock(rootBlock), rootTimestamp);
        assertTrue(queue.isProcessed(processed.nonce));
        assertFalse(queue.isProcessed(retained.nonce));
        assertEq(token.balanceOf(address(manager)), 20);
        assertTrue(queue.paused());
        queue.submitMerkleRoot(recoveryProof.leaf.parentNumber, Hasher.hash(unpause), encodeProof(recoveryProof));
        vm.warp(vm.getBlockTimestamp() + queue.PROCESS_PAUSER_MESSAGE_DELAY());
        queue.processMessage(recoveryProof.leaf.parentNumber, 1, 0, unpause, new bytes32[](0));
        assertFalse(queue.paused());
        siblings[0] = firstLeaf;
        queue.processMessage(rootBlock, 2, 1, retained, siblings);
        assertEq(token.balanceOf(address(manager)), 0);
        assertEq(token.balanceOf(address(this)), 30);
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MessageAlreadyProcessed.selector, processed.nonce));
        siblings[0] = secondLeaf;
        queue.processMessage(rootBlock, 2, 0, processed, siblings);
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MessageAlreadyProcessed.selector, retained.nonce));
        siblings[0] = firstLeaf;
        queue.processMessage(rootBlock, 2, 1, retained, siblings);
    }

    function testRecoveryRejectsLiveOrStaleStateWithoutChangingVerifier() public {
        VaraQueueRootVerifier replacement = migrationVerifier(queue);
        customQueueProofFor(
            replacement.beefyClient(),
            104,
            106,
            bytes32(uint256(1)),
            uint64(block.timestamp * 1000),
            uint64(block.timestamp * 1000)
        );
        vm.expectRevert(IMessageQueue.InvalidBeefyRecovery.selector);
        vm.prank(address(recovery));
        queue.recoverBeefyVerifier(verifier, replacement, 0);
        vm.warp(block.timestamp + client.MAX_SOURCE_AGE_MS() / 1000 + 1);
        vm.expectRevert(IMessageQueue.InvalidBeefyRecovery.selector);
        vm.prank(address(recovery));
        queue.recoverBeefyVerifier(IVerifier(address(1)), replacement, 0);
        vm.expectRevert(IMessageQueue.InvalidBeefyRecovery.selector);
        vm.prank(address(recovery));
        queue.recoverBeefyVerifier(verifier, replacement, 1);
        vm.expectRevert(IMessageQueue.InvalidBeefyRecovery.selector);
        vm.prank(address(recovery));
        queue.recoverBeefyVerifier(verifier, replacement, 0);
        assertEq(queue.verifier(), address(verifier));
        assertFalse(queue.paused());
    }

    function legacyQueue(string memory artifact, uint256 height) internal returns (MessageQueue legacy) {
        address implementation = vm.parseJsonAddress(artifact, ".implementation");
        bytes memory code = vm.parseJsonBytes(artifact, ".deployedBytecode");
        assertEq(keccak256(code), vm.parseJsonBytes32(artifact, ".codeHash"));
        vm.etch(implementation, code);
        legacy = MessageQueue(
            address(
                new ERC1967Proxy(
                    implementation,
                    abi.encodeWithSignature(
                        "initialize(address,address,address,address[],address)",
                        queue.governanceAdmin(),
                        queue.governancePauser(),
                        address(this),
                        new address[](0),
                        address(0xBEEF)
                    )
                )
            )
        );
        vm.prank(legacy.governanceAdmin());
        (bool ok,) = address(legacy).call(abi.encodeWithSignature("reinitialize()"));
        assertTrue(ok);
        assertTrue(legacy.isProcessed(836));
        VaraMessage memory message = VaraMessage(835, bytes32(uint256(77)), address(receiver), hex"abcd");
        // Exact frozen verifier calldata is mocked only while registering the historical claim.
        address historicalVerifier = address(0xBEEF);
        vm.etch(historicalVerifier, hex"00");
        bytes32 historicalRoot = Hasher.hash(message);
        uint256[] memory historicalInputs = new uint256[](2);
        historicalInputs[0] = uint256(historicalRoot) >> 64;
        historicalInputs[1] = (uint256(uint64(uint256(historicalRoot))) << 128) | (height << 96);
        vm.mockCall(
            historicalVerifier,
            abi.encodeWithSignature("safeVerifyProof(bytes,uint256[])", bytes(""), historicalInputs),
            abi.encode(true)
        );
        legacy.submitMerkleRoot(height, Hasher.hash(message), "");
        vm.clearMockedCalls();
    }

    // Fixture authorities exercise contract migration, not a production BEEFY bootstrap.
    function migrationVerifier(MessageQueue target) internal returns (VaraQueueRootVerifier next) {
        uint64 start = uint64(fixtureUint(0, "mmrStartBlock"));
        bytes32 authorityRoot = fixtureHash(0, "authorityRoot");
        uint128 count = uint128(fixtureUint(0, "validatorCount"));
        BeefyClient nextClient = new BeefyClient(
            fixtureHash(0, "sourceDomain"),
            block.chainid,
            address(target),
            start,
            start + 1,
            uint64(vm.getBlockTimestamp() * 1000),
            BeefyClient.ValidatorSet(0, count, authorityRoot),
            BeefyClient.ValidatorSet(1, count, authorityRoot)
        );
        next = new VaraQueueRootVerifier(nextClient, address(target), block.chainid);
    }

    function migrateLegacy(MessageQueue legacy, VaraQueueRootVerifier next, uint256 floor, bytes32 initializableSlot)
        internal
    {
        address implementation = address(new MessageQueue());
        bytes memory data = abi.encodeCall(
            MessageQueue.migrateToBeefy,
            (IVerifier(legacy.verifier()), IVerifier(address(next)), legacy.maxBlockNumber(), floor, address(recovery))
        );
        address admin = legacy.governanceAdmin();
        vm.record();
        vm.prank(admin);
        legacy.upgradeToAndCall(implementation, data);
        (, bytes32[] memory writes) = vm.accesses(address(legacy));
        for (uint256 i; i < writes.length; ++i) {
            assertTrue(
                writes[i] == ERC1967Utils.IMPLEMENTATION_SLOT || writes[i] == initializableSlot
                    || writes[i] == bytes32(uint256(5)) || writes[i] == bytes32(uint256(14))
                    || writes[i] == bytes32(uint256(16)),
                "migration overwrote legacy state"
            );
        }
        assertTrue(legacy.paused());
        assertEq(legacy.verifier(), address(next));
        assertEq(legacy.beefyRootMinimum(), floor);
        assertEq(legacy.beefyRecoveryTimelock(), address(recovery));
        assertTrue(legacy.hasRole(legacy.DEFAULT_ADMIN_ROLE(), admin));
    }

    function testLegacyMigrationPreservesClaimsAndAdmitsBeefyRoots() public {
        string memory artifact = vm.readFile("test/fixtures/mainnet-legacy-queue.json");
        MessageQueue legacy = legacyQueue(artifact, 120);
        VaraMessage memory oldMessage = VaraMessage(835, bytes32(uint256(77)), address(receiver), hex"abcd");
        VaraMessage memory newMessage = VaraMessage(837, oldMessage.source, address(receiver), hex"1234");
        bytes32 oldRoot = Hasher.hash(oldMessage);
        uint256 timestamp = block.timestamp;
        VaraQueueRootVerifier next = migrationVerifier(legacy);
        QueueProof memory proof = customQueueProofFor(
            next.beefyClient(), 121, 123, Hasher.hash(newMessage), uint64(timestamp * 1000), uint64(timestamp * 1000)
        );
        vm.prank(legacy.governancePauser());
        legacy.pause();
        migrateLegacy(legacy, next, 121, vm.parseJsonBytes32(artifact, ".initializableSlot"));
        assertEq(legacy.genesisBlock(), 120);
        assertEq(legacy.maxBlockNumber(), 120);
        assertEq(legacy.getMerkleRoot(120), oldRoot);
        assertEq(legacy.getMerkleRootTimestampForBlock(120), timestamp);
        assertTrue(legacy.isProcessed(836));
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.BlockNumberBelowMinimum.selector, 120, 121));
        legacy.submitMerkleRoot(120, oldRoot, "");
        legacy.submitMerkleRoot(121, Hasher.hash(newMessage), encodeProof(proof));
        vm.warp(timestamp + legacy.PROCESS_USER_MESSAGE_DELAY());
        vm.prank(legacy.governancePauser());
        legacy.unpause();
        legacy.processMessage(120, 1, 0, oldMessage, new bytes32[](0));
        legacy.processMessage(121, 1, 0, newMessage, new bytes32[](0));
        assertTrue(legacy.isProcessed(835));
        assertTrue(legacy.isProcessed(837));
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MessageAlreadyProcessed.selector, 835));
        legacy.processMessage(120, 1, 0, oldMessage, new bytes32[](0));
    }

    function testLegacyMigrationRejectsUnsafeCutoversAtomically() public {
        string memory artifact = vm.readFile("test/fixtures/mainnet-legacy-queue.json");
        MessageQueue legacy = legacyQueue(artifact, 90);
        VaraQueueRootVerifier next = migrationVerifier(legacy);
        address admin = legacy.governanceAdmin();
        address implementation = address(new MessageQueue());
        IVerifier oldVerifier = IVerifier(legacy.verifier());
        bytes memory validData =
            abi.encodeCall(MessageQueue.migrateToBeefy, (oldVerifier, next, 90, 100, address(recovery)));
        vm.expectRevert(
            abi.encodeWithSelector(IAccessControl.AccessControlUnauthorizedAccount.selector, address(this), bytes32(0))
        );
        legacy.upgradeToAndCall(implementation, validData);
        vm.expectRevert(bytes4(keccak256("ExpectedPause()")));
        vm.prank(admin);
        legacy.upgradeToAndCall(implementation, validData);
        vm.prank(legacy.governancePauser());
        legacy.pause();
        // A bootstrap without any authenticated root must not disable the legacy verifier.
        vm.expectRevert(IMessageQueue.InvalidBeefyMigration.selector);
        vm.prank(admin);
        legacy.upgradeToAndCall(implementation, validData);
        customQueueProofFor(
            next.beefyClient(),
            100,
            103,
            bytes32(uint256(1)),
            uint64(block.timestamp * 1000),
            uint64(block.timestamp * 1000)
        );
        uint256[5] memory invalidFloors =
            [uint256(90), 99, 103, 90 + legacy.MAX_BLOCK_DISTANCE() + 1, uint256(type(uint32).max) + 1];
        for (uint256 i; i < invalidFloors.length; ++i) {
            bytes memory data = abi.encodeCall(
                MessageQueue.migrateToBeefy, (oldVerifier, next, 90, invalidFloors[i], address(recovery))
            );
            vm.expectRevert(IMessageQueue.InvalidBeefyMigration.selector);
            vm.prank(admin);
            legacy.upgradeToAndCall(implementation, data);
        }
        bytes memory staleData =
            abi.encodeCall(MessageQueue.migrateToBeefy, (oldVerifier, next, 91, 100, address(recovery)));
        vm.expectRevert(IMessageQueue.InvalidBeefyMigration.selector);
        vm.prank(admin);
        legacy.upgradeToAndCall(implementation, staleData);
        staleData =
            abi.encodeCall(MessageQueue.migrateToBeefy, (IVerifier(address(1)), next, 90, 100, address(recovery)));
        vm.expectRevert(IMessageQueue.InvalidBeefyMigration.selector);
        vm.prank(admin);
        legacy.upgradeToAndCall(implementation, staleData);
        vm.warp(block.timestamp + next.beefyClient().MAX_SOURCE_AGE_MS() / 1000 + 1);
        vm.expectRevert(IMessageQueue.InvalidBeefyMigration.selector);
        vm.prank(admin);
        legacy.upgradeToAndCall(implementation, validData);
        assertEq(
            address(uint160(uint256(vm.load(address(legacy), ERC1967Utils.IMPLEMENTATION_SLOT)))),
            vm.parseJsonAddress(artifact, ".implementation")
        );
        assertEq(legacy.verifier(), address(oldVerifier));
        assertEq(vm.load(address(legacy), bytes32(uint256(14))), bytes32(0));
        assertEq(uint64(uint256(vm.load(address(legacy), vm.parseJsonBytes32(artifact, ".initializableSlot")))), 6);
        assertTrue(legacy.isProcessed(836));
    }

    function testMainnetForkMigrationPreservesCustodyAndRedeemsLegacyClaim() public {
        string memory rpc = vm.envOr("MAINNET_MIGRATION_RPC_URL", string(""));
        if (bytes(rpc).length == 0) {
            vm.skip(true);
            return;
        }
        string memory artifact = vm.readFile("test/fixtures/mainnet-legacy-queue.json");
        uint256 height = vm.parseJsonUint(artifact, ".ethereumBlock");
        vm.createSelectFork(rpc, height);
        // Contracts created by setUp are not carried into the newly selected fork.
        address[] memory proposers = new address[](1);
        proposers[0] = address(this);
        recovery = new TimelockController(1 hours, proposers, proposers, address(0));
        assertEq(block.chainid, 1);
        vm.roll(height + 1);
        assertEq(blockhash(height), vm.parseJsonBytes32(artifact, ".ethereumBlockHash"));
        vm.roll(height);
        MessageQueue legacy = MessageQueue(vm.parseJsonAddress(artifact, ".queue"));
        address oldImplementation =
            address(uint160(uint256(vm.load(address(legacy), ERC1967Utils.IMPLEMENTATION_SLOT))));
        assertEq(oldImplementation.codehash, vm.parseJsonBytes32(artifact, ".codeHash"));
        bytes32 initSlot = vm.parseJsonBytes32(artifact, ".initializableSlot");
        assertEq(uint64(uint256(vm.load(address(legacy), initSlot))), 6);
        VaraMessage memory claim = VaraMessage(
            vm.parseJsonUint(artifact, ".claims.pending21.nonce"),
            vm.parseJsonBytes32(artifact, ".claims.pending21.source"),
            vm.parseJsonAddress(artifact, ".claims.pending21.destination"),
            vm.parseJsonBytes(artifact, ".claims.pending21.payload")
        );
        uint256 sourceBlock = vm.parseJsonUint(artifact, ".claims.pending21.sourceBlock");
        bytes32 root = vm.parseJsonBytes32(artifact, ".claims.pending21.root");
        assertEq(legacy.getMerkleRoot(sourceBlock), root);
        assertFalse(legacy.isProcessed(claim.nonce));
        assertTrue(legacy.isProcessed(864));
        assertTrue(legacy.isProcessed(836));
        uint256 legacyTimestamp = uint256(vm.load(address(legacy), keccak256(abi.encode(root, uint256(12)))));
        uint256 maxBlock = legacy.maxBlockNumber();
        uint256 genesis = legacy.genesisBlock();
        address[] memory tokens = ERC20Manager(claim.destination).tokens();
        uint256[] memory custody = new uint256[](tokens.length);
        uint256[] memory supplies = new uint256[](tokens.length);
        for (uint256 i; i < tokens.length; ++i) {
            custody[i] = IERC20(tokens[i]).balanceOf(claim.destination);
            supplies[i] = IERC20(tokens[i]).totalSupply();
        }
        VaraQueueRootVerifier next = migrationVerifier(legacy);
        customQueueProofFor(
            next.beefyClient(),
            uint32(maxBlock + 1),
            uint64(maxBlock + 3),
            bytes32(uint256(1)),
            uint64(block.timestamp * 1000),
            uint64(block.timestamp * 1000)
        );
        // Local authority impersonation only; no source governance proof or public transaction.
        vm.prank(legacy.governancePauser());
        legacy.pause();
        migrateLegacy(legacy, next, maxBlock + 1, initSlot);
        assertEq(legacy.genesisBlock(), genesis);
        assertEq(legacy.maxBlockNumber(), maxBlock);
        assertEq(legacy.getMerkleRoot(sourceBlock), root);
        assertEq(legacy.getMerkleRootTimestampForBlock(sourceBlock), legacyTimestamp);
        assertTrue(legacy.isProcessed(864));
        assertTrue(legacy.isProcessed(836));
        for (uint256 i; i < tokens.length; ++i) {
            assertEq(IERC20(tokens[i]).balanceOf(claim.destination), custody[i]);
            assertEq(IERC20(tokens[i]).totalSupply(), supplies[i]);
        }
        IERC20 token = IERC20(vm.parseJsonAddress(artifact, ".claims.pending21.token"));
        address recipient = vm.parseJsonAddress(artifact, ".claims.pending21.recipient");
        uint256 amount = vm.parseUint(vm.parseJsonString(artifact, ".claims.pending21.amount"));
        uint256 recipientBefore = token.balanceOf(recipient);
        uint256 supplyBefore = token.totalSupply();
        vm.prank(legacy.governancePauser());
        legacy.unpause();
        legacy.processMessage(sourceBlock, 1, 0, claim, new bytes32[](0));
        assertEq(token.balanceOf(recipient), recipientBefore + amount);
        assertEq(token.totalSupply(), supplyBefore + amount);
        assertTrue(legacy.isProcessed(claim.nonce));
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MessageAlreadyProcessed.selector, claim.nonce));
        legacy.processMessage(sourceBlock, 1, 0, claim, new bytes32[](0));
    }
}

/// @dev Test-only opt-in to exercise initializer rollback before successful initialization.
contract InitializerTestProxy is ERC1967Proxy {
    constructor(address implementation) ERC1967Proxy(implementation, "") {}

    function _unsafeAllowUninitialized() internal pure override returns (bool) {
        return true;
    }
}
