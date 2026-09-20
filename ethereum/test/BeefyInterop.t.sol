// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Test} from "forge-std/Test.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {ScaleCodec} from "src/beefy/utils/ScaleCodec.sol";
import {SubstrateMerkleProof} from "src/beefy/utils/SubstrateMerkleProof.sol";

abstract contract BeefyFixtureTest is Test {
    string internal fixtures;

    struct QueueProof {
        uint8 proofVersion;
        uint8 bridgeVersion;
        uint64 queueId;
        uint64 anchorBlock;
        bytes32 anchorRoot;
        BeefyClient.MMRLeaf leaf;
        bytes32[] items;
        uint256 order;
    }

    function fixturePath(uint256 index, string memory field) internal pure returns (string memory) {
        return string.concat(".cases[", vm.toString(index), "].", field);
    }

    function fixtureBytes(uint256 index, string memory field) internal view returns (bytes memory) {
        return vm.parseJsonBytes(fixtures, fixturePath(index, field));
    }

    function fixtureHash(uint256 index, string memory field) internal view returns (bytes32) {
        return vm.parseJsonBytes32(fixtures, fixturePath(index, field));
    }

    function queueProof(uint256 index) internal view returns (QueueProof memory p) {
        (p.proofVersion, p.bridgeVersion, p.queueId, p.anchorBlock, p.anchorRoot, p.leaf, p.items, p.order) = abi.decode(
            fixtureBytes(index, "queueProof"),
            (uint8, uint8, uint64, uint64, bytes32, BeefyClient.MMRLeaf, bytes32[], uint256)
        );
    }

    function encodeProof(QueueProof memory p) internal pure returns (bytes memory) {
        return
            abi.encode(
                p.proofVersion, p.bridgeVersion, p.queueId, p.anchorBlock, p.anchorRoot, p.leaf, p.items, p.order
            );
    }

    function leafBytes(BeefyClient.MMRLeaf memory leaf) internal pure returns (bytes memory) {
        return bytes.concat(
            ScaleCodec.encodeU8(leaf.version),
            ScaleCodec.encodeU32(leaf.parentNumber),
            leaf.parentHash,
            ScaleCodec.encodeU64(leaf.nextAuthoritySetID),
            ScaleCodec.encodeU32(leaf.nextAuthoritySetLen),
            leaf.nextAuthoritySetRoot,
            leaf.parachainHeadsRoot
        );
    }

    function newClient() internal returns (BeefyClient) {
        bytes32 root = fixtureHash(0, "authorityRoot");
        return new BeefyClient(
            128, 24, 17, 111, 0, BeefyClient.ValidatorSet(0, 3, root), BeefyClient.ValidatorSet(1, 3, root)
        );
    }

    function commitment(uint32 height, uint64 setId, bytes32 root)
        internal
        pure
        returns (BeefyClient.Commitment memory c)
    {
        c.blockNumber = height;
        c.validatorSetID = setId;
        c.payload = new BeefyClient.PayloadItem[](1);
        c.payload[0] = BeefyClient.PayloadItem(bytes2("mh"), abi.encodePacked(root));
    }

    function allSigners() internal pure returns (uint256[] memory bits) {
        bits = new uint256[](1);
        bits[0] = 7;
    }

    function validatorProof(uint256 index) internal returns (BeefyClient.ValidatorProof memory p) {
        address[] memory addresses = vm.parseJsonAddressArray(fixtures, fixturePath(0, "authorityAddresses"));
        p.index = index;
        p.account = addresses[index];
        bytes32 h0 = keccak256(abi.encodePacked(addresses[0]));
        bytes32 h1 = keccak256(abi.encodePacked(addresses[1]));
        bytes32 h2 = keccak256(abi.encodePacked(addresses[2]));
        p.proof = new bytes32[](index == 2 ? 1 : 2);
        if (index == 2) {
            p.proof[0] = keccak256(abi.encodePacked(h0, h1));
        } else {
            p.proof[0] = index == 0 ? h1 : h0;
            p.proof[1] = h2;
        }
    }

    function selectedProofs(
        BeefyClient client,
        BeefyClient.Commitment memory c,
        uint256 selected,
        bytes memory signedBytes
    ) internal returns (BeefyClient.ValidatorProof[] memory proofs) {
        proofs = new BeefyClient.ValidatorProof[](2);
        uint256 n;
        bytes32 digest = client.computeCommitmentHash(c);
        for (uint256 index; index < 3; index++) {
            if ((selected & (1 << index)) == 0) continue;
            BeefyClient.ValidatorProof memory p = validatorProof(index);
            if (signedBytes.length == 0) {
                (p.v, p.r, p.s) = vm.sign((type(uint256).max / 255) * (index + 1), digest);
            } else {
                // The canonical fixtures have three present, ordered 65-byte signatures.
                uint256 offset = signedBytes.length - 195 + index * 65;
                bytes32 r;
                bytes32 s;
                assembly ("memory-safe") {
                    r := mload(add(add(signedBytes, 32), offset))
                    s := mload(add(add(signedBytes, 64), offset))
                }
                p.r = r;
                p.s = s;
                p.v = uint8(signedBytes[offset + 64]) + 27;
            }
            proofs[n++] = p;
        }
        assertEq(n, 2);
    }

    function signedProofs(BeefyClient client, BeefyClient.Commitment memory c, bytes memory raw)
        internal
        returns (BeefyClient.ValidatorProof[] memory)
    {
        uint256[] memory selection = client.createFiatShamirFinalBitfield(c, allSigners());
        return selectedProofs(client, c, selection[0], raw);
    }

    function acceptFixture(BeefyClient client, uint256 index) internal {
        QueueProof memory p = queueProof(index);
        BeefyClient.Commitment memory c = commitment(uint32(p.anchorBlock), 0, p.anchorRoot);
        client.submitFiatShamir(
            c, allSigners(), signedProofs(client, c, fixtureBytes(index, "signedCommitment")), p.leaf, p.items, p.order
        );
    }

    function stateHash(BeefyClient client) internal view returns (bytes32) {
        (uint128 current, uint128 currentLength, bytes32 currentRoot,) = client.currentValidatorSet();
        (uint128 next, uint128 nextLength, bytes32 nextRoot,) = client.nextValidatorSet();
        return keccak256(
            abi.encode(
                client.latestMMRRoot(),
                client.latestBeefyBlock(),
                current,
                currentLength,
                currentRoot,
                next,
                nextLength,
                nextRoot
            )
        );
    }
}

contract BeefyInteropTest is BeefyFixtureTest {
    function setUp() public {
        fixtures = vm.readFile("test/fixtures/beefy-interop.json");
    }

    function testSharedSignedFixturesAndEncodings() public {
        assertEq(vm.parseJsonUint(fixtures, ".schemaVersion"), 1);
        for (uint256 i; i < 8; i++) {
            BeefyClient client = newClient();
            QueueProof memory p = queueProof(i);
            bytes memory snapshot = bytes.concat(
                bytes1(uint8(vm.parseJsonUint(fixtures, fixturePath(i, "bridgeVersion")))),
                bytes4("vara"),
                ScaleCodec.encodeU64(p.queueId),
                fixtureHash(i, "queueRoot")
            );
            assertEq(snapshot.length, 45);
            assertEq(snapshot, fixtureBytes(i, "snapshotPreimage"));
            assertEq(keccak256(snapshot), fixtureHash(i, "bridgeCommitment"));
            assertEq(leafBytes(p.leaf), fixtureBytes(i, "outerLeaf"));
            assertEq(leafBytes(p.leaf).length, 113);
            assertEq(keccak256(leafBytes(p.leaf)), fixtureHash(i, "outerLeafHash"));
            BeefyClient.Commitment memory c = commitment(uint32(p.anchorBlock), 0, p.anchorRoot);
            bytes memory encoded = bytes.concat(
                hex"046d6880",
                abi.encodePacked(p.anchorRoot),
                ScaleCodec.encodeU32(c.blockNumber),
                ScaleCodec.encodeU64(c.validatorSetID)
            );
            assertEq(encoded, fixtureBytes(i, "commitmentBytes"));
            assertEq(client.computeCommitmentHash(c), fixtureHash(i, "commitmentHash"));
            assertEq(keccak256(encoded), fixtureHash(i, "commitmentHash"));
            acceptFixture(client, i);
            assertEq(client.latestMMRRoot(), p.anchorRoot);
            assertEq(client.latestBeefyBlock(), p.anchorBlock);
            assertTrue(client.verifyMMRLeafProof(keccak256(leafBytes(p.leaf)), p.items, p.order));
        }
    }

    function testConsensusRejectionsPreserveEntireCheckpoint() public {
        BeefyClient client = newClient();
        QueueProof memory p = queueProof(0);
        BeefyClient.Commitment memory c = commitment(uint32(p.anchorBlock), 0, p.anchorRoot);
        bytes32 beforeState = stateHash(client);
        for (uint256 mutation; mutation < 6; mutation++) {
            uint256[] memory bits = allSigners();
            BeefyClient.ValidatorProof[] memory proofs = signedProofs(client, c, fixtureBytes(0, "signedCommitment"));
            if (mutation == 0) proofs[0].r = bytes32(uint256(1));
            if (mutation == 1) proofs[0].index = (proofs[0].index + 1) % 3;
            if (mutation == 2) proofs[1] = proofs[0];
            if (mutation == 3) bits[0] = 3;
            if (mutation == 4) bits[0] |= 8;
            if (mutation == 5) c.validatorSetID = 2;
            vm.expectRevert();
            client.submitFiatShamir(c, bits, proofs, p.leaf, p.items, p.order);
            assertEq(stateHash(client), beforeState);
        }
        c.validatorSetID = 0;
        acceptFixture(client, 0);
        bytes32 accepted = stateHash(client);
        BeefyClient.ValidatorProof[] memory stale = signedProofs(client, c, fixtureBytes(0, "signedCommitment"));
        vm.expectRevert(BeefyClient.StaleCommitment.selector);
        client.submitFiatShamir(c, allSigners(), stale, p.leaf, p.items, p.order);
        assertEq(stateHash(client), accepted);
    }

    function testAuthenticatedHandoverRejectsMutatedLeaf() public {
        BeefyClient client = newClient();
        acceptFixture(client, 0);
        BeefyClient.MMRLeaf memory leaf = queueProof(0).leaf;
        leaf.parentNumber = 1;
        leaf.nextAuthoritySetID = 2;
        leaf.nextAuthoritySetRoot = keccak256("new-key-set");
        bytes32 root = keccak256(leafBytes(leaf));
        BeefyClient.Commitment memory c = commitment(2, 1, root);
        BeefyClient.ValidatorProof[] memory proofs = signedProofs(client, c, "");
        bytes32 beforeState = stateHash(client);
        leaf.parentHash ^= bytes32(uint256(1));
        vm.expectRevert(BeefyClient.InvalidMMRLeafProof.selector);
        client.submitFiatShamir(c, allSigners(), proofs, leaf, new bytes32[](0), 0);
        assertEq(stateHash(client), beforeState);
        leaf.parentHash ^= bytes32(uint256(1));
        client.submitFiatShamir(c, allSigners(), proofs, leaf, new bytes32[](0), 0);
        (uint128 current,,,) = client.currentValidatorSet();
        (uint128 next,, bytes32 nextRoot,) = client.nextValidatorSet();
        assertEq(current, 1);
        assertEq(next, 2);
        assertEq(nextRoot, leaf.nextAuthoritySetRoot);
        assertEq(client.latestMMRRoot(), root);
        assertEq(client.latestBeefyBlock(), 2);
    }

    function testInteractiveEntryPointAcceptsGenuineSignatures() public {
        BeefyClient client = newClient();
        QueueProof memory p = queueProof(0);
        BeefyClient.Commitment memory c = commitment(1, 0, p.anchorRoot);
        BeefyClient.ValidatorProof memory initial = validatorProof(0);
        (initial.v, initial.r, initial.s) = vm.sign(type(uint256).max / 255, client.computeCommitmentHash(c));
        client.submitInitial(c, allSigners(), initial);
        vm.roll(block.number + 128);
        vm.prevrandao(bytes32(uint256(12345)));
        client.commitPrevRandao(client.computeCommitmentHash(c));
        uint256[] memory selection = client.createFinalBitfield(client.computeCommitmentHash(c), allSigners());
        client.submitFinal(c, allSigners(), selectedProofs(client, c, selection[0], ""), p.leaf, p.items, p.order);
        assertEq(client.latestMMRRoot(), p.anchorRoot);
    }
}
