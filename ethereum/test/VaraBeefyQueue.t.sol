// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {BeefyFixtureTest} from "./BeefyInterop.t.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {ERC20Manager} from "src/ERC20Manager.sol";
import {GovernanceAdmin} from "src/GovernanceAdmin.sol";
import {GovernancePauser} from "src/GovernancePauser.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {VaraQueueRootVerifier} from "src/VaraQueueRootVerifier.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {ScaleCodec} from "src/beefy/utils/ScaleCodec.sol";
import {WrappedVara} from "src/erc20/WrappedVara.sol";
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
        client = newClient();
        verifier = new VaraQueueRootVerifier(client);
        GovernanceAdmin admin = new GovernanceAdmin(
            bytes32(uint256(33)), WrappedVara(address(0)), MessageQueue(address(0)), ERC20Manager(address(0))
        );
        GovernancePauser pauser = new GovernancePauser(
            bytes32(uint256(44)), WrappedVara(address(0)), MessageQueue(address(0)), ERC20Manager(address(0))
        );
        queue = MessageQueue(
            address(
                new ERC1967Proxy(
                    address(new MessageQueue()),
                    abi.encodeCall(
                        MessageQueue.initialize,
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
        receiver = new MessageHandlerMock();
        vm.warp(block.timestamp + queue.CHALLENGE_ROOT_DELAY());
    }

    function inputs(uint32 source, bytes32 root) internal pure returns (uint256[] memory p) {
        p = new uint256[](2);
        p[0] = uint256(root) >> 64;
        p[1] = (uint256(uint64(uint256(root))) << 128) | (uint256(source) << 96);
    }

    function checkpoint(QueueProof memory p) internal {
        BeefyClient.Commitment memory c = commitment(uint32(p.anchorBlock), 0, p.anchorRoot);
        client.submitFiatShamir(c, allSigners(), signedProofs(client, c, ""), p.leaf, p.items, p.order);
    }

    function testSharedFixturesMatchLegacyPublicInputLayout() public {
        for (uint256 i; i < 8; i++) {
            BeefyClient fixtureClient = newClient();
            VaraQueueRootVerifier fixtureVerifier = new VaraQueueRootVerifier(fixtureClient);
            QueueProof memory p = queueProof(i);
            uint256[] memory publicInputs = inputs(p.leaf.parentNumber, fixtureHash(i, "queueRoot"));
            assertEq(abi.encode(publicInputs[0], publicInputs[1]), fixtureBytes(i, "publicInputs"));
            acceptFixture(fixtureClient, i);
            bool valid = fixtureVerifier.safeVerifyProof(fixtureBytes(i, "queueProof"), publicInputs);
            assertEq(valid, i != 1 && i != 2);
        }
    }

    function testConstructorRequiresDeployedClient() public {
        vm.expectRevert();
        new VaraQueueRootVerifier(BeefyClient(address(0)));
    }

    function testCanonicalBoundaryRejections() public {
        acceptFixture(client, 0);
        QueueProof memory p = queueProof(0);
        bytes32 root = fixtureHash(0, "queueRoot");
        uint256[] memory publicInputs = inputs(0, root);
        bytes memory encoded = encodeProof(p);
        assertTrue(verifier.safeVerifyProof(encoded, publicInputs));
        for (uint256 mutation; mutation < 16; mutation++) {
            QueueProof memory bad = queueProof(0);
            if (mutation == 0) bad.proofVersion = 1;
            if (mutation == 1) bad.bridgeVersion = 1;
            if (mutation == 2) bad.bridgeVersion = 255;
            if (mutation == 3) bad.queueId++;
            if (mutation == 4) bad.anchorBlock++;
            if (mutation == 5) bad.anchorRoot ^= bytes32(uint256(1));
            if (mutation == 6) bad.leaf.version = 1;
            if (mutation == 7) bad.leaf.parentNumber++;
            if (mutation == 8) bad.leaf.parentHash ^= bytes32(uint256(1));
            if (mutation == 9) bad.leaf.nextAuthoritySetID++;
            if (mutation == 10) bad.leaf.nextAuthoritySetLen++;
            if (mutation == 11) bad.leaf.nextAuthoritySetRoot ^= bytes32(uint256(1));
            if (mutation == 12) bad.leaf.parachainHeadsRoot ^= bytes32(uint256(1));
            if (mutation == 13) bad.order = 1;
            if (mutation == 14) bad.items = new bytes32[](1);
            if (mutation == 15) bad.items = new bytes32[](257);
            assertFalse(verifier.safeVerifyProof(encodeProof(bad), publicInputs));
            vm.expectRevert(IMessageQueue.InvalidPlonkProof.selector);
            queue.submitMerkleRoot(0, root, encodeProof(bad));
            assertEq(queue.getMerkleRoot(0), bytes32(0));
            assertFalse(queue.isProcessed(7));
        }
        assertFalse(verifier.safeVerifyProof(bytes.concat(encoded, hex"00"), publicInputs));
        assertFalse(verifier.safeVerifyProof(hex"00", publicInputs));
        assertFalse(verifier.safeVerifyProof(abi.encode(p), publicInputs));
        assertFalse(verifier.safeVerifyProof(encoded, new uint256[](1)));
        assertFalse(verifier.safeVerifyProof(encoded, new uint256[](3)));
        assertFalse(verifier.safeVerifyProof(encoded, inputs(0, bytes32(0))));
        assertFalse(verifier.safeVerifyProof(encoded, inputs(0, root ^ bytes32(uint256(1)))));
        assertFalse(verifier.safeVerifyProof(encoded, inputs(1, root)));
        for (uint256 word; word < 2; word++) {
            uint256[] memory padded = inputs(0, root);
            padded[word] |= uint256(1) << 192;
            assertFalse(verifier.safeVerifyProof(encoded, padded));
        }
        publicInputs[1] |= 1;
        assertFalse(verifier.safeVerifyProof(encoded, publicInputs));
        publicInputs = inputs(0, root);
        // Nonzero padding of a narrow ABI value and an incorrect dynamic offset.
        encoded[0] = bytes1(uint8(1));
        assertFalse(verifier.safeVerifyProof(encoded, publicInputs));
        encoded = encodeProof(p);
        encoded[415] = bytes1(uint8(0));
        assertFalse(verifier.safeVerifyProof(encoded, publicInputs));
        encoded = encodeProof(p);
        assembly ("memory-safe") { mstore(encoded, sub(mload(encoded), 1)) }
        assertFalse(verifier.safeVerifyProof(encoded, publicInputs));
    }

    function testProofOrderUsesEntireUint256AndRejectsUnusedBits() public {
        QueueProof memory p = queueProof(0);
        p.items = new bytes32[](256);
        p.order = type(uint256).max;
        bytes32 accumulator = keccak256(leafBytes(p.leaf));
        for (uint256 i; i < 256; i++) {
            p.items[i] = keccak256(abi.encode(i));
            accumulator = keccak256(abi.encodePacked(p.items[i], accumulator));
        }
        p.anchorRoot = accumulator;
        checkpoint(p);
        assertTrue(verifier.safeVerifyProof(encodeProof(p), inputs(0, fixtureHash(0, "queueRoot"))));
        p.order ^= uint256(1) << 255;
        assertFalse(verifier.safeVerifyProof(encodeProof(p), inputs(0, fixtureHash(0, "queueRoot"))));
        p.order = type(uint256).max;
        p.items = new bytes32[](255);
        assertFalse(verifier.safeVerifyProof(encodeProof(p), inputs(0, fixtureHash(0, "queueRoot"))));
    }

    function testUnchangedQueueMaturityReceiverAndReplay() public {
        VaraMessage memory message =
            VaraMessage(7, bytes32(uint256(77)), address(receiver), bytes("real receiver payload"));
        bytes32 messageHash =
            keccak256(abi.encodePacked(message.nonce, message.source, message.destination, message.payload));
        QueueProof memory p = queueProof(0);
        p.leaf.parachainHeadsRoot =
            keccak256(bytes.concat(hex"0076617261", ScaleCodec.encodeU64(p.queueId), messageHash));
        p.anchorRoot = keccak256(leafBytes(p.leaf));
        checkpoint(p);
        queue.submitMerkleRoot(0, messageHash, encodeProof(p));
        assertEq(queue.getMerkleRoot(0), messageHash);
        vm.expectRevert(IMessageQueue.MerkleRootDelayNotPassed.selector);
        queue.processMessage(0, 1, 0, message, new bytes32[](0));
        vm.warp(block.timestamp + queue.PROCESS_USER_MESSAGE_DELAY());
        vm.expectEmit(true, false, false, true, address(receiver));
        emit IMessageHandlerMock.MessageHandled(message.source, message.payload);
        queue.processMessage(0, 1, 0, message, new bytes32[](0));
        assertTrue(queue.isProcessed(message.nonce));
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MessageAlreadyProcessed.selector, message.nonce));
        queue.processMessage(0, 1, 0, message, new bytes32[](0));
    }

    function testStaleAnchorFailsThenSameSourceCanBeReproved() public {
        acceptFixture(client, 0);
        QueueProof memory oldProof = queueProof(0);
        bytes32 root = fixtureHash(0, "queueRoot");
        assertTrue(verifier.safeVerifyProof(encodeProof(oldProof), inputs(0, root)));
        QueueProof memory newer = queueProof(0);
        newer.anchorBlock = 2;
        newer.items = new bytes32[](1);
        newer.items[0] = keccak256("next inserted leaf");
        newer.anchorRoot = keccak256(abi.encodePacked(keccak256(leafBytes(newer.leaf)), newer.items[0]));
        checkpoint(newer);
        vm.expectRevert(IMessageQueue.InvalidPlonkProof.selector);
        queue.submitMerkleRoot(0, root, encodeProof(oldProof));
        assertEq(queue.getMerkleRoot(0), bytes32(0));
        assertTrue(verifier.safeVerifyProof(encodeProof(newer), inputs(0, root)));
        queue.submitMerkleRoot(0, root, encodeProof(newer));
        assertEq(queue.getMerkleRoot(0), root);
    }
}
