// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {BeefyFixtureTest} from "./BeefyInterop.t.sol";
import {IAccessControl} from "@openzeppelin/contracts/access/IAccessControl.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
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
import {MessageHandlerMock} from "src/mocks/MessageHandlerMock.sol";

contract VaraQueueRootVerifierTest is BeefyFixtureTest {
    BeefyClient internal client;
    VaraQueueRootVerifier internal verifier;
    MessageQueue internal queue;
    MessageHandlerMock internal receiver;

    function setUp() public {
        fixtures = vm.readFile("test/fixtures/beefy-interop.json");
        uint256 startingNonce = vm.getNonce(address(this));
        address predictedQueue = vm.computeCreateAddress(address(this), startingNonce + 5);
        client = newClient(0, block.chainid, predictedQueue);

        MessageQueue implementation = new MessageQueue();
        GovernanceAdmin admin = new GovernanceAdmin(
            bytes32(uint256(33)), WrappedVara(address(0)), MessageQueue(address(0)), ERC20Manager(address(0))
        );
        GovernancePauser pauser = new GovernancePauser(
            bytes32(uint256(44)), WrappedVara(address(0)), MessageQueue(address(0)), ERC20Manager(address(0))
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
                            verifier
                        )
                    )
                )
            )
        );
        assertEq(address(queue), predictedQueue);
        assertEq(verifier.messageQueue(), address(queue));
        receiver = new MessageHandlerMock();
    }

    function inputs(uint32 source, bytes32 root) internal pure returns (uint256[] memory p) {
        p = new uint256[](2);
        p[0] = uint256(root) >> 64;
        p[1] = (uint256(uint64(uint256(root))) << 128) | (uint256(source) << 96);
    }

    function verifyAsQueue(VaraQueueRootVerifier target, bytes memory proof, uint256[] memory publicInputs)
        internal
        returns (bool valid)
    {
        vm.prank(target.messageQueue());
        return target.safeVerifyProof(proof, publicInputs);
    }

    function testSharedFixturesMatchLegacyPublicInputLayout() public {
        vm.pauseGasMetering();
        for (uint256 i; i < caseCount(); i++) {
            if (isNativeOnly(i)) continue;
            BeefyClient fixtureClient = newClient(i);
            VaraQueueRootVerifier fixtureVerifier =
                new VaraQueueRootVerifier(fixtureClient, fixtureClient.destinationQueue(), block.chainid);
            QueueProof memory p = queueProof(i);
            bytes32 root = fixtureHash(i, "queueRoot");
            uint256[] memory publicInputs = inputs(p.leaf.parentNumber, root);
            assertEq(abi.encode(publicInputs[0], publicInputs[1]), fixtureBytes(i, "publicInputs"));
            acceptFixture(fixtureClient, i);
            bool valid = verifyAsQueue(fixtureVerifier, fixtureBytes(i, "queueProof"), publicInputs);
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

    function testFreshBeefyInitializerValidatesBindingFloorAndFrozenPolicy() public {
        MessageQueue fresh = MessageQueue(address(new InitializerTestProxy(address(new MessageQueue()))));
        IGovernance admin = IGovernance(queue.governanceAdmin());
        IGovernance pauser = IGovernance(queue.governancePauser());
        vm.expectRevert(IMessageQueue.InvalidBeefyVerifier.selector);
        fresh.initializeBeefy(admin, pauser, address(this), new address[](0), verifier);
        BeefyClient boundClient = newClient(0, block.chainid, address(fresh));
        VaraQueueRootVerifier boundVerifier = new VaraQueueRootVerifier(boundClient, address(fresh), block.chainid);
        bytes4[10] memory selectors = [
            boundClient.minNumRequiredSignatures.selector,
            boundClient.fiatShamirRequiredSignatures.selector,
            boundClient.MAX_VALIDATORS.selector,
            boundClient.randaoCommitDelay.selector,
            boundClient.randaoCommitExpiration.selector,
            boundClient.mmrStartBlock.selector,
            boundClient.sourceDomain.selector,
            boundClient.destinationChainId.selector,
            boundClient.destinationQueue.selector,
            boundClient.bridgeDomain.selector
        ];
        uint256[10] memory invalid =
            [uint256(85), 85, 257, 1, 25, 0, 0, block.chainid + 1, uint256(uint160(address(queue))), 1];
        for (uint256 i; i < selectors.length; i++) {
            vm.mockCall(address(boundClient), abi.encodeWithSelector(selectors[i]), abi.encode(invalid[i]));
            vm.expectRevert(IMessageQueue.InvalidBeefyVerifier.selector);
            fresh.initializeBeefy(admin, pauser, address(this), new address[](0), boundVerifier);
            vm.clearMockedCalls();
        }
        vm.mockCall(
            address(boundVerifier),
            abi.encodeWithSelector(boundVerifier.destinationChainId.selector),
            abi.encode(block.chainid + 1)
        );
        vm.expectRevert(IMessageQueue.InvalidBeefyVerifier.selector);
        fresh.initializeBeefy(admin, pauser, address(this), new address[](0), boundVerifier);
        vm.clearMockedCalls();
        fresh.initializeBeefy(admin, pauser, address(this), new address[](0), boundVerifier);
        assertEq(fresh.beefyRootMinimum(), boundClient.mmrStartBlock());
        assertEq(fresh.verifier(), address(boundVerifier));
        assertTrue(fresh.hasRole(fresh.DEFAULT_ADMIN_ROLE(), address(admin)));
        vm.expectRevert();
        fresh.initializeBeefy(admin, pauser, address(this), new address[](0), boundVerifier);
    }

    function testQueueAndChainBindings() public {
        bytes32 root = fixtureHash(0, "queueRoot");
        QueueProof memory p = customQueueProof(103, root);
        bytes memory proof = encodeProof(p);
        uint256[] memory publicInputs = inputs(p.leaf.parentNumber, root);
        vm.prank(address(this));
        assertFalse(verifier.safeVerifyProof(proof, publicInputs));
        vm.prank(address(0xBEEF));
        assertFalse(verifier.safeVerifyProof(proof, publicInputs));

        uint256 chainId = block.chainid;
        vm.chainId(chainId + 1);
        vm.prank(address(queue));
        assertFalse(verifier.safeVerifyProof(proof, publicInputs));
        vm.chainId(chainId);

        assertFalse(verifier.verifyProof(proof, publicInputs));
    }

    function testCanonicalBoundaryRejections() public {
        bytes32 root = fixtureHash(0, "queueRoot");
        QueueProof memory p = customQueueProof(103, root);
        uint256[] memory publicInputs = inputs(p.leaf.parentNumber, root);
        bytes memory encoded = encodeProof(p);
        bytes memory pristine = abi.encode(p);
        uint32 sourceBlock = p.leaf.parentNumber;
        assertFalse(verifier.safeVerifyProof(encoded, publicInputs));
        assertTrue(verifyAsQueue(verifier, encoded, publicInputs));

        for (uint256 mutation; mutation < 18; mutation++) {
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
            assertFalse(verifyAsQueue(verifier, encodeProof(bad), publicInputs), vm.toString(mutation));
            vm.expectRevert(IMessageQueue.InvalidPlonkProof.selector);
            queue.submitMerkleRoot(sourceBlock, root, encodeProof(bad));
            assertEq(queue.getMerkleRoot(sourceBlock), bytes32(0));
            assertEq(queue.genesisBlock(), 0);
            assertFalse(queue.isProcessed(7));
        }
        assertFalse(verifyAsQueue(verifier, bytes.concat(encoded, hex"00"), publicInputs), "trailing byte");
        assertFalse(verifyAsQueue(verifier, hex"00", publicInputs), "short proof");
        assertFalse(verifyAsQueue(verifier, abi.encode(p), publicInputs), "noncanonical tuple");
        assertFalse(verifyAsQueue(verifier, encoded, new uint256[](1)), "short inputs");
        assertFalse(verifyAsQueue(verifier, encoded, new uint256[](3)), "long inputs");
        assertFalse(verifyAsQueue(verifier, encoded, inputs(p.leaf.parentNumber, bytes32(0))), "zero root");
        assertFalse(
            verifyAsQueue(verifier, encoded, inputs(p.leaf.parentNumber, root ^ bytes32(uint256(1)))), "wrong root"
        );
        assertFalse(verifyAsQueue(verifier, encoded, inputs(p.leaf.parentNumber + 1, root)), "wrong block");
        for (uint256 word; word < 2; word++) {
            uint256[] memory padded = inputs(p.leaf.parentNumber, root);
            padded[word] |= uint256(1) << 192;
            assertFalse(verifyAsQueue(verifier, encoded, padded));
        }
        publicInputs[1] |= 1;
        assertFalse(verifyAsQueue(verifier, encoded, publicInputs), "nonzero input padding");
        publicInputs = inputs(p.leaf.parentNumber, root);
        // Nonzero padding of a narrow ABI value and an incorrect dynamic offset.
        encoded[30] = bytes1(uint8(1));
        assertFalse(verifyAsQueue(verifier, encoded, publicInputs), "narrow ABI padding");
        encoded = encodeProof(p);
        encoded[511] = bytes1(uint8(0x21));
        assertFalse(verifyAsQueue(verifier, encoded, publicInputs), "dynamic offset");
        encoded = encodeProof(p);
        assembly ("memory-safe") { mstore(encoded, sub(mload(encoded), 1)) }
        assertFalse(verifyAsQueue(verifier, encoded, publicInputs), "truncated proof");
        assertTrue(verifyAsQueue(verifier, encodeProof(p), inputs(sourceBlock, root)));
        queue.submitMerkleRoot(sourceBlock, root, encodeProof(p));
        assertEq(queue.getMerkleRoot(sourceBlock), root);
        assertEq(queue.genesisBlock(), sourceBlock);
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
        assertFalse(verifyAsQueue(verifier, encodeProof(p), inputs(belowStart, root)));
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

    function customQueueProofFor(
        BeefyClient targetClient,
        uint32 sourceBlock,
        uint64 anchorBlock,
        bytes32 queueRoot,
        uint64 historicalTimestamp,
        uint64 freshnessTimestamp
    ) internal returns (QueueProof memory p) {
        uint64 authenticatedTimestamp = targetClient.lastAuthenticatedSourceTimestampMs();
        if (freshnessTimestamp <= authenticatedTimestamp) freshnessTimestamp = authenticatedTimestamp + 1;
        p = queueProof(0);
        p.leaf.parentNumber = sourceBlock;
        bytes32 bridgeDomain = targetClient.bridgeDomain();
        uint64 queueId = uint64(vm.parseJsonUint(fixtures, ".cases[0].queueId"));
        uint64 freshnessQueueId = uint64(vm.parseJsonUint(fixtures, ".cases[0].freshnessProof.queueId"));
        bytes32 freshnessRoot = fixtureHash(0, "freshnessProof.queueRoot");
        VaraBridgeMetadata.Snapshot memory historicalSnapshot = VaraBridgeMetadata.Snapshot({
            version: 2,
            bridgeDomain: bridgeDomain,
            sourceTimestampMs: historicalTimestamp,
            initialized: true,
            queueId: queueId,
            queueRoot: queueRoot
        });
        VaraBridgeMetadata.Snapshot memory freshnessSnapshot = VaraBridgeMetadata.Snapshot({
            version: 2,
            bridgeDomain: bridgeDomain,
            sourceTimestampMs: freshnessTimestamp,
            initialized: true,
            queueId: freshnessQueueId,
            queueRoot: freshnessRoot
        });
        p.proofVersion = 2;
        p.bridgeVersion = 2;
        p.initialized = true;
        p.bridgeDomain = bridgeDomain;
        p.sourceTimestampMs = historicalTimestamp;
        p.queueId = queueId;
        p.leaf.parachainHeadsRoot = VaraBridgeMetadata.hash(historicalSnapshot);
        BeefyClient.MMRLeaf memory freshnessLeaf = BeefyClient.MMRLeaf({
            version: p.leaf.version,
            parentNumber: uint32(anchorBlock - 1),
            parentHash: p.leaf.parentHash ^ bytes32(uint256(anchorBlock)),
            nextAuthoritySetID: p.leaf.nextAuthoritySetID,
            nextAuthoritySetLen: p.leaf.nextAuthoritySetLen,
            nextAuthoritySetRoot: p.leaf.nextAuthoritySetRoot,
            parachainHeadsRoot: VaraBridgeMetadata.hash(freshnessSnapshot)
        });
        bytes32 historicalLeafHash = keccak256(leafBytes(p.leaf));
        bytes32 freshnessLeafHash = keccak256(leafBytes(freshnessLeaf));
        p.anchorBlock = anchorBlock;
        p.anchorRoot = keccak256(abi.encodePacked(historicalLeafHash, freshnessLeafHash));
        p.items = new bytes32[](1);
        p.items[0] = freshnessLeafHash;
        p.order = 0;
        BeefyClient.Commitment memory c = commitment(uint32(anchorBlock), 0, p.anchorRoot);
        for (uint256 i; i < c.payload.length; i++) {
            if (c.payload[i].payloadID == bytes2("mh")) c.payload[i].data = abi.encodePacked(p.anchorRoot);
        }
        bytes32[] memory freshnessProof = new bytes32[](1);
        freshnessProof[0] = historicalLeafHash;
        targetClient.submitFiatShamir(
            c, allSigners(), signedProofs(targetClient, c, ""), freshnessLeaf, freshnessSnapshot, freshnessProof, 1
        );
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
        vm.expectRevert(IMessageQueue.InvalidPlonkProof.selector);
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
        uint256[] memory publicInputs = inputs(p.leaf.parentNumber, bytes32(uint256(0x1234)));
        assertTrue(verifyAsQueue(verifier, encodeProof(p), publicInputs));
        p.order = uint256(1) << 255;
        assertFalse(verifyAsQueue(verifier, encodeProof(p), publicInputs));
        p.order = 0;
        p.items = new bytes32[](257);
        assertFalse(verifyAsQueue(verifier, encodeProof(p), publicInputs));
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
        assertTrue(verifyAsQueue(verifier, encodeProof(oldProof), inputs(oldProof.leaf.parentNumber, root)));
        QueueProof memory newer = customQueueProof(104, root);
        vm.expectRevert(IMessageQueue.InvalidPlonkProof.selector);
        queue.submitMerkleRoot(oldProof.leaf.parentNumber, root, encodeProof(oldProof));
        assertEq(queue.getMerkleRoot(oldProof.leaf.parentNumber), bytes32(0));
        assertTrue(verifyAsQueue(verifier, encodeProof(newer), inputs(newer.leaf.parentNumber, root)));
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

    function testAdminUpgradeRetainsEscrowAndClaims() public {
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
        vm.prank(queue.governancePauser());
        queue.pause();

        MessageQueue replacement = new MessageQueue();
        bytes32 adminRole = queue.DEFAULT_ADMIN_ROLE();
        vm.expectRevert(
            abi.encodeWithSelector(IAccessControl.AccessControlUnauthorizedAccount.selector, address(this), adminRole)
        );
        queue.upgradeToAndCall(address(replacement), "");
        vm.expectRevert(
            abi.encodeWithSelector(IAccessControl.AccessControlUnauthorizedAccount.selector, address(this), adminRole)
        );
        queue.reinitialize();

        uint256 floor = queue.beefyRootMinimum();
        vm.prank(queue.governanceAdmin());
        queue.upgradeToAndCall(address(replacement), abi.encodeCall(MessageQueue.reinitialize, ()));
        address multisig = 0x1111111111111111111111111111111111111111;
        assertTrue(queue.hasRole(adminRole, queue.governanceAdmin()));
        assertTrue(queue.hasRole(adminRole, multisig));
        assertTrue(queue.hasRole(queue.PAUSER_ROLE(), multisig));
        assertEq(queue.beefyRootMinimum(), floor);
        assertEq(queue.verifier(), address(verifier));
        assertEq(queue.genesisBlock(), rootBlock);
        assertEq(queue.maxBlockNumber(), rootBlock);
        MessageQueue nextImplementation = new MessageQueue();
        vm.prank(multisig);
        queue.upgradeToAndCall(address(nextImplementation), "");
        assertEq(queue.beefyRootMinimum(), floor);
        assertEq(queue.verifier(), address(verifier));
        assertEq(queue.getMerkleRoot(rootBlock), root);
        assertEq(queue.getMerkleRootTimestampForBlock(rootBlock), rootTimestamp);
        assertTrue(queue.isProcessed(processed.nonce));
        assertFalse(queue.isProcessed(retained.nonce));
        assertEq(token.balanceOf(address(manager)), 20);
        assertTrue(queue.paused());
        vm.prank(queue.governancePauser());
        queue.unpause();
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
}

/// @dev Test-only opt-in to exercise initializer rollback before successful initialization.
contract InitializerTestProxy is ERC1967Proxy {
    constructor(address implementation) ERC1967Proxy(implementation, "") {}

    function _unsafeAllowUninitialized() internal pure override returns (bool) {
        return true;
    }
}
