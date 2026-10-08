// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Test} from "forge-std/Test.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {VaraBridgeMetadata} from "src/beefy/VaraBridgeMetadata.sol";
import {ScaleCodec} from "src/beefy/utils/ScaleCodec.sol";
import {SubstrateMerkleProof} from "src/beefy/utils/SubstrateMerkleProof.sol";
import {Uint16Array} from "src/beefy/utils/Uint16Array.sol";

contract SubstrateMerkleProofHarness {
    function verify(bytes32 root, bytes32 leaf, uint256 position, uint256 width, bytes32[] calldata proof)
        external
        pure
        returns (bool)
    {
        return SubstrateMerkleProof.verify(root, leaf, position, width, proof);
    }
}

abstract contract BeefyFixtureTest is Test {
    string internal fixtures;

    struct QueueProof {
        uint8 proofVersion;
        uint8 bridgeVersion;
        bool initialized;
        bytes32 bridgeDomain;
        uint64 sourceTimestampMs;
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

    function fixtureUint(uint256 index, string memory field) internal view returns (uint256) {
        return vm.parseJsonUint(fixtures, fixturePath(index, field));
    }

    function caseCount() internal view returns (uint256 count) {
        while (vm.keyExistsJson(fixtures, string.concat(".cases[", vm.toString(count), "]"))) count++;
    }

    function caseIndex(string memory name) internal view returns (uint256) {
        for (uint256 i; i < caseCount(); i++) {
            if (keccak256(bytes(vm.parseJsonString(fixtures, fixturePath(i, "name")))) == keccak256(bytes(name))) {
                return i;
            }
        }
        revert("fixture case not found");
    }

    function isNativeOnly(uint256 index) internal view returns (bool) {
        return vm.parseJsonBool(fixtures, fixturePath(index, "nativeOnly"));
    }

    function queueProof(uint256 index) internal view returns (QueueProof memory p) {
        (
            p.proofVersion,
            p.bridgeVersion,
            p.initialized,
            p.bridgeDomain,
            p.sourceTimestampMs,
            p.queueId,
            p.anchorBlock,
            p.anchorRoot,
            p.leaf,
            p.items,
            p.order
        ) =
            abi.decode(
                fixtureBytes(index, "queueProof"),
                (uint8, uint8, bool, bytes32, uint64, uint64, uint64, bytes32, BeefyClient.MMRLeaf, bytes32[], uint256)
            );
    }

    function encodeProof(QueueProof memory p) internal pure returns (bytes memory) {
        return abi.encode(
            p.proofVersion,
            p.bridgeVersion,
            p.initialized,
            p.bridgeDomain,
            p.sourceTimestampMs,
            p.queueId,
            p.anchorBlock,
            p.anchorRoot,
            p.leaf,
            p.items,
            p.order
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

    function readU32LE(bytes memory data, uint256 offset) internal pure returns (uint32 value) {
        for (uint256 i; i < 4; i++) {
            value |= uint32(uint256(uint8(data[offset + i])) << (8 * i));
        }
    }

    function readU64LE(bytes memory data, uint256 offset) internal pure returns (uint64 value) {
        for (uint256 i; i < 8; i++) {
            value |= uint64(uint256(uint8(data[offset + i])) << (8 * i));
        }
    }

    function readBytes32(bytes memory data, uint256 offset) internal pure returns (bytes32 value) {
        assembly ("memory-safe") {
            value := mload(add(add(data, 32), offset))
        }
    }

    function decodeLeaf(bytes memory encoded) internal pure returns (BeefyClient.MMRLeaf memory leaf) {
        require(encoded.length == 113, "invalid outer leaf length");
        leaf.version = uint8(encoded[0]);
        leaf.parentNumber = readU32LE(encoded, 1);
        leaf.parentHash = readBytes32(encoded, 5);
        leaf.nextAuthoritySetID = readU64LE(encoded, 37);
        leaf.nextAuthoritySetLen = readU32LE(encoded, 45);
        leaf.nextAuthoritySetRoot = readBytes32(encoded, 49);
        leaf.parachainHeadsRoot = readBytes32(encoded, 81);
    }

    function snapshot(uint256 index, bool freshness) internal view returns (VaraBridgeMetadata.Snapshot memory s) {
        string memory prefix = freshness ? "freshnessProof." : "";
        bytes memory preimage = fixtureBytes(index, string.concat(prefix, "snapshotPreimage"));
        s.version = uint8(preimage[0]);
        s.bridgeDomain = fixtureHash(index, string.concat(prefix, "bridgeDomain"));
        s.sourceTimestampMs = uint64(fixtureUint(index, string.concat(prefix, "sourceTimestampMs")));
        s.initialized = vm.parseJsonBool(fixtures, fixturePath(index, string.concat(prefix, "initialized")));
        s.queueId = uint64(fixtureUint(index, string.concat(prefix, "queueId")));
        s.queueRoot = fixtureHash(index, string.concat(prefix, "queueRoot"));
    }

    function freshnessWitness(uint256 index)
        internal
        view
        returns (
            BeefyClient.MMRLeaf memory leaf,
            VaraBridgeMetadata.Snapshot memory s,
            bytes32[] memory items,
            uint256 order
        )
    {
        leaf = decodeLeaf(fixtureBytes(index, "freshnessProof.outerLeaf"));
        s = snapshot(index, true);
        items = vm.parseJsonBytes32Array(fixtures, fixturePath(index, "freshnessProof.simplifiedItems"));
        order = uint256(fixtureHash(index, "freshnessProof.proofOrder"));
    }

    function leafForSnapshot(
        VaraBridgeMetadata.Snapshot memory s,
        uint32 parentNumber,
        uint64 nextAuthoritySetID,
        uint32 nextAuthoritySetLen,
        bytes32 nextAuthoritySetRoot
    ) internal pure returns (BeefyClient.MMRLeaf memory leaf) {
        leaf.version = 0;
        leaf.parentNumber = parentNumber;
        leaf.parentHash = bytes32(0);
        leaf.nextAuthoritySetID = nextAuthoritySetID;
        leaf.nextAuthoritySetLen = nextAuthoritySetLen;
        leaf.nextAuthoritySetRoot = nextAuthoritySetRoot;
        leaf.parachainHeadsRoot = VaraBridgeMetadata.hash(s);
    }

    function bootstrapTimestamp(uint256 index) internal view returns (uint64) {
        uint256 source = fixtureUint(index, "sourceBlock");
        uint256 start = fixtureUint(index, "mmrStartBlock");
        uint256 timestamp = fixtureUint(index, "sourceTimestampMs");
        if (timestamp == 0 || source < start) return uint64(timestamp);
        uint256 delta = (source - start) * 3_000;
        require(timestamp >= delta, "invalid fixture timestamp");
        return uint64(timestamp - delta);
    }

    function newClient() internal returns (BeefyClient) {
        return newClient(0);
    }

    function newClient(uint256 index) internal returns (BeefyClient) {
        return newClient(
            index,
            fixtureUint(index, "destinationChainId"),
            vm.parseJsonAddress(fixtures, fixturePath(index, "destinationQueue"))
        );
    }

    function newClient(uint256 index, uint256 destinationChainId, address destinationQueue)
        internal
        returns (BeefyClient)
    {
        return newClient(index, fixtureHash(index, "sourceDomain"), destinationChainId, destinationQueue);
    }

    function newClient(uint256 index, bytes32 sourceDomain, uint256 destinationChainId, address destinationQueue)
        internal
        returns (BeefyClient)
    {
        uint64 sourceTimestamp = bootstrapTimestamp(index);
        vm.warp(uint256(sourceTimestamp) / 1000);
        bytes32 authorityRoot = fixtureHash(index, "authorityRoot");
        uint64 start = uint64(fixtureUint(index, "mmrStartBlock"));
        return new BeefyClient(
            sourceDomain,
            destinationChainId,
            destinationQueue,
            start,
            start + 1,
            sourceTimestamp,
            BeefyClient.ValidatorSet(0, uint128(fixtureUint(index, "validatorCount")), authorityRoot),
            BeefyClient.ValidatorSet(1, uint128(fixtureUint(index, "validatorCount")), authorityRoot)
        );
    }

    function fixtureBytes2(uint256 index, string memory field) internal view returns (bytes2 value) {
        bytes memory data = fixtureBytes(index, field);
        require(data.length == 2, "invalid bytes2 fixture");
        assembly ("memory-safe") {
            value := mload(add(data, 32))
        }
    }

    function commitment(uint32 height, uint64 setId, bytes32 root)
        internal
        view
        returns (BeefyClient.Commitment memory c)
    {
        return commitment(0, height, setId, root);
    }

    function commitment(uint256 index, uint32 height, uint64 setId, bytes32 root)
        internal
        view
        returns (BeefyClient.Commitment memory c)
    {
        uint256 count;
        while (vm.keyExistsJson(
                fixtures, fixturePath(index, string.concat("payloadItems[", vm.toString(count), "]"))
            )) {
            count++;
        }
        c.blockNumber = height;
        c.validatorSetID = setId;
        c.payload = new BeefyClient.PayloadItem[](count);
        for (uint256 i; i < count; i++) {
            string memory base = string.concat("payloadItems[", vm.toString(i), "].");
            c.payload[i] = BeefyClient.PayloadItem(
                fixtureBytes2(index, string.concat(base, "payloadID")), fixtureBytes(index, string.concat(base, "data"))
            );
            if (c.payload[i].payloadID == bytes2("mh")) c.payload[i].data = abi.encodePacked(root);
        }
    }

    function allSigners() internal view returns (uint256[] memory bits) {
        return allSigners(0);
    }

    function allSigners(uint256 index) internal view returns (uint256[] memory bits) {
        uint256 count = fixtureUint(index, "validatorCount");
        bits = new uint256[]((count + 255) / 256);
        for (uint256 i; i < count; i++) bits[i / 256] |= uint256(1) << (i % 256);
    }

    function validatorProof(uint256 index) internal returns (BeefyClient.ValidatorProof memory p) {
        return validatorProof(0, index);
    }

    function validatorProof(uint256 caseNumber, uint256 index) internal returns (BeefyClient.ValidatorProof memory p) {
        address[] memory addresses = vm.parseJsonAddressArray(fixtures, fixturePath(caseNumber, "authorityAddresses"));
        p.index = index;
        p.account = addresses[index];
        p.proof = vm.parseJsonBytes32Array(
            fixtures, fixturePath(caseNumber, string.concat("authorityProofs[", vm.toString(index), "]"))
        );
    }

    function selectedProofs(
        BeefyClient client,
        BeefyClient.Commitment memory c,
        uint256 selected,
        bytes memory signedBytes
    ) internal returns (BeefyClient.ValidatorProof[] memory proofs) {
        return selectedProofs(client, c, 0, selected, signedBytes);
    }

    function selectedProofs(
        BeefyClient client,
        BeefyClient.Commitment memory c,
        uint256 caseNumber,
        uint256 selected,
        bytes memory signedBytes
    ) internal returns (BeefyClient.ValidatorProof[] memory proofs) {
        uint256 validatorCount = fixtureUint(caseNumber, "validatorCount");
        string memory json = fixtures;
        address[] memory addresses = vm.parseJsonAddressArray(json, fixturePath(caseNumber, "authorityAddresses"));
        uint256 count;
        for (uint256 i; i < validatorCount; i++) {
            if ((selected & (uint256(1) << i)) != 0) count++;
        }
        proofs = new BeefyClient.ValidatorProof[](count);
        bytes32 digest = client.computeCommitmentHash(c);
        uint256 n;
        for (uint256 index; index < validatorCount; index++) {
            if ((selected & (uint256(1) << index)) == 0) continue;
            BeefyClient.ValidatorProof memory p;
            p.index = index;
            p.account = addresses[index];
            p.proof = vm.parseJsonBytes32Array(
                json, fixturePath(caseNumber, string.concat("authorityProofs[", vm.toString(index), "]"))
            );
            if (signedBytes.length == 0) {
                (p.v, p.r, p.s) = vm.sign(index + 1, digest);
            } else {
                uint256 offset = signedBytes.length - validatorCount * 65 + index * 65;
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
    }

    function signedProofs(BeefyClient client, BeefyClient.Commitment memory c, bytes memory raw)
        internal
        returns (BeefyClient.ValidatorProof[] memory)
    {
        return signedProofs(client, c, 0, raw);
    }

    function signedProofs(BeefyClient client, BeefyClient.Commitment memory c, uint256 caseNumber, bytes memory raw)
        internal
        returns (BeefyClient.ValidatorProof[] memory)
    {
        uint256[] memory selection = client.createFiatShamirFinalBitfield(c, allSigners(caseNumber));
        return selectedProofs(client, c, caseNumber, selection[0], raw);
    }

    function acceptFixture(BeefyClient client, uint256 index) internal {
        acceptFixture(client, index, false);
    }

    function acceptFixture(BeefyClient client, uint256 index, bool measureGas) internal {
        require(!isNativeOnly(index), "native-only fixture");
        (BeefyClient.MMRLeaf memory leaf, VaraBridgeMetadata.Snapshot memory s, bytes32[] memory items, uint256 order) =
            freshnessWitness(index);
        vm.warp(uint256(s.sourceTimestampMs) / 1000);
        BeefyClient.Commitment memory c =
            commitment(index, uint32(fixtureUint(index, "anchorBlock")), 0, fixtureHash(index, "mmrRoot"));
        uint256[] memory signers = allSigners(index);
        BeefyClient.ValidatorProof[] memory proofs =
            signedProofs(client, c, index, fixtureBytes(index, "signedCommitment"));
        uint256 gasBefore = gasleft();
        client.submitFiatShamir(c, signers, proofs, leaf, s, items, order);
        uint256 gasUsed = gasBefore - gasleft();
        if (measureGas) {
            emit log_named_uint(
                string.concat("submitFiatShamir-N", vm.toString(fixtureUint(index, "validatorCount"))), gasUsed
            );
        }
    }

    function rejectFixtureForClient(BeefyClient client, uint256 index) internal {
        (BeefyClient.MMRLeaf memory leaf, VaraBridgeMetadata.Snapshot memory s, bytes32[] memory items, uint256 order) =
            freshnessWitness(index);
        vm.warp(uint256(s.sourceTimestampMs) / 1000);
        BeefyClient.Commitment memory c =
            commitment(index, uint32(fixtureUint(index, "anchorBlock")), 0, fixtureHash(index, "mmrRoot"));
        uint256[] memory signers = allSigners(index);
        BeefyClient.ValidatorProof[] memory proofs =
            signedProofs(client, c, index, fixtureBytes(index, "signedCommitment"));
        vm.expectRevert(BeefyClient.InvalidSourceTimestamp.selector);
        client.submitFiatShamir(c, signers, proofs, leaf, s, items, order);
    }

    function encodeCommitmentBytes(BeefyClient.Commitment memory c) internal pure returns (bytes memory) {
        bytes memory encoded = ScaleCodec.checkedEncodeCompactU32(c.payload.length);
        for (uint256 i; i < c.payload.length; i++) {
            encoded = bytes.concat(
                encoded,
                c.payload[i].payloadID,
                ScaleCodec.checkedEncodeCompactU32(c.payload[i].data.length),
                c.payload[i].data
            );
        }
        encoded = bytes.concat(encoded, ScaleCodec.encodeU32(c.blockNumber));
        return bytes.concat(encoded, ScaleCodec.encodeU64(c.validatorSetID));
    }

    function stateHash(BeefyClient client) internal view returns (bytes32) {
        (uint128 current, uint128 currentLength, bytes32 currentRoot,) = client.currentValidatorSet();
        (uint128 next, uint128 nextLength, bytes32 nextRoot,) = client.nextValidatorSet();
        return keccak256(
            abi.encode(
                client.latestMMRRoot(),
                client.latestBeefyBlock(),
                client.lastAuthenticatedSourceTimestampMs(),
                current,
                currentLength,
                currentRoot,
                next,
                nextLength,
                nextRoot
            )
        );
    }
    function finishUpdate(
        bool interactive,
        BeefyClient client,
        BeefyClient.Commitment memory c,
        uint256[] memory bits,
        BeefyClient.ValidatorProof[] memory proofs,
        BeefyClient.MMRLeaf memory leaf,
        VaraBridgeMetadata.Snapshot memory s,
        bytes32[] memory items,
        uint256 order
    ) internal {
        if (interactive) client.submitFinal(c, bits, proofs, leaf, s, items, order);
        else client.submitFiatShamir(c, bits, proofs, leaf, s, items, order);
    }
}

contract BeefyInteropTest is BeefyFixtureTest {
    function setUp() public {
        fixtures = vm.readFile("test/fixtures/beefy-interop.json");
    }

    function testSharedSignedFixturesAndEncodings() public {
        assertEq(vm.parseJsonUint(fixtures, ".schemaVersion"), 3);
        vm.pauseGasMetering();
        for (uint256 i; i < caseCount(); i++) {
            QueueProof memory p = queueProof(i);
            bytes32 sourceDomain = fixtureHash(i, "sourceDomain");
            uint256 destinationChainId = fixtureUint(i, "destinationChainId");
            address destinationQueue = vm.parseJsonAddress(fixtures, fixturePath(i, "destinationQueue"));
            bytes32 bridgeDomain = keccak256(
                abi.encodePacked(
                    "vara/gear-eth-bridge-domain/v2", sourceDomain, bytes32(destinationChainId), destinationQueue
                )
            );
            assertEq(fixtureHash(i, "bridgeDomain"), bridgeDomain);
            assertEq(fixtureHash(i, "sourceGenesis"), fixtureHash(i, "freshnessProof.sourceGenesis"));
            if (i == 0) {
                assertEq(
                    fixtureHash(i, "sourceGenesis"),
                    bytes32(0x9999999999999999999999999999999999999999999999999999999999999999)
                );
            }
            assertEq(p.proofVersion, 2);
            assertEq(p.bridgeVersion, 2);
            if (i == 0) assertEq(bridgeDomain, 0x9aac6d72e183672d20696112082accb15719870152120212f541e3e233837944);
            VaraBridgeMetadata.Snapshot memory historical = snapshot(i, false);
            assertEq(VaraBridgeMetadata.hash(historical), fixtureHash(i, "bridgeCommitment"));
            assertEq(
                bytes.concat(
                    ScaleCodec.encodeU8(historical.version),
                    bytes4("vara"),
                    historical.bridgeDomain,
                    ScaleCodec.encodeU64(historical.sourceTimestampMs),
                    ScaleCodec.encodeU8(historical.initialized ? 1 : 0),
                    ScaleCodec.encodeU64(historical.queueId),
                    historical.queueRoot
                ),
                fixtureBytes(i, "snapshotPreimage")
            );
            assertEq(leafBytes(p.leaf).length, 113);
            assertEq(leafBytes(p.leaf), fixtureBytes(i, "outerLeaf"));
            assertEq(keccak256(leafBytes(p.leaf)), fixtureHash(i, "outerLeafHash"));
            assertEq(encodeProof(p), fixtureBytes(i, "queueProof"));
            BeefyClient.Commitment memory c = commitment(i, uint32(p.anchorBlock), 0, p.anchorRoot);
            bytes memory encoded = encodeCommitmentBytes(c);
            assertEq(encoded, fixtureBytes(i, "commitmentBytes"));
            BeefyClient client;
            if (!isNativeOnly(i) && i == 0) {
                client = newClient(i);
                assertEq(client.sourceDomain(), fixtureHash(i, "sourceDomain"));
                assertEq(client.destinationChainId(), fixtureUint(i, "destinationChainId"));
                assertEq(client.destinationQueue(), vm.parseJsonAddress(fixtures, fixturePath(i, "destinationQueue")));
                assertEq(client.bridgeDomain(), fixtureHash(i, "bridgeDomain"));
                assertEq(client.computeCommitmentHash(c), fixtureHash(i, "commitmentHash"));
                assertEq(keccak256(encoded), fixtureHash(i, "commitmentHash"));
                acceptFixture(client, i);
                assertEq(client.latestMMRRoot(), fixtureHash(i, "mmrRoot"));
                assertEq(client.latestBeefyBlock(), p.anchorBlock);
                assertEq(
                    client.lastAuthenticatedSourceTimestampMs(), fixtureUint(i, "freshnessProof.sourceTimestampMs")
                );
                (BeefyClient.MMRLeaf memory fresh,, bytes32[] memory items, uint256 order) = freshnessWitness(i);
                assertTrue(client.verifyMMRLeafProof(keccak256(leafBytes(fresh)), items, order));
            }
        }
        vm.resumeGasMetering();
    }

    function testSubstrateMerkleProofRejectsLastLeafPositionAlias() public {
        bytes32 a = keccak256("A");
        bytes32 b = keccak256("B");
        bytes32 c = keccak256("C");
        bytes32 ab = keccak256(abi.encodePacked(a, b));
        bytes32 root = keccak256(abi.encodePacked(ab, c));
        bytes32[] memory proof = new bytes32[](1);
        proof[0] = ab;
        SubstrateMerkleProofHarness harness = new SubstrateMerkleProofHarness();
        assertTrue(harness.verify(root, c, 2, 3, proof));
        assertFalse(harness.verify(root, c, 1, 3, proof));
        delete proof;
        assertFalse(harness.verify(root, c, 2, 3, proof));

        bytes32 d = keccak256("D");
        bytes32 e = keccak256("E");
        bytes32 abcd = keccak256(abi.encodePacked(ab, keccak256(abi.encodePacked(c, d))));
        root = keccak256(abi.encodePacked(abcd, e));
        proof = new bytes32[](1);
        proof[0] = abcd;
        assertTrue(harness.verify(root, e, 4, 5, proof));
        for (uint256 position; position < 4; position++) {
            assertFalse(harness.verify(root, e, position, 5, proof));
        }
        assertFalse(harness.verify(root, e, 5, 5, proof));
        assertFalse(harness.verify(root, e, 0, 0, proof));
        assertFalse(harness.verify(root, e, 4, 5, new bytes32[](0)));
        proof = new bytes32[](2);
        proof[0] = abcd;
        assertFalse(harness.verify(root, e, 4, 5, proof));
    }

    function testMMRProofOrderRejectsUnusedBits() public {
        uint256 index = caseIndex("synthetic-mmr-3-primary");
        BeefyClient client = newClient(index);
        (BeefyClient.MMRLeaf memory leaf, VaraBridgeMetadata.Snapshot memory s, bytes32[] memory items, uint256 order) =
            freshnessWitness(index);
        vm.warp(uint256(s.sourceTimestampMs) / 1000);
        BeefyClient.Commitment memory c = commitment(index, 102, 0, fixtureHash(index, "mmrRoot"));
        client.submitFiatShamir(
            c,
            allSigners(index),
            signedProofs(client, c, index, fixtureBytes(index, "signedCommitment")),
            leaf,
            s,
            items,
            order
        );
        assertTrue(client.verifyMMRLeafProof(keccak256(leafBytes(leaf)), items, order));
        if (items.length < 256) {
            assertFalse(
                client.verifyMMRLeafProof(keccak256(leafBytes(leaf)), items, order | (uint256(1) << items.length))
            );
        }
    }

    function testCompletePayloadAndParserRejections() public {
        uint256 index = caseIndex("synthetic-mmr-3-primary");
        BeefyClient client = newClient(index);
        (BeefyClient.MMRLeaf memory leaf, VaraBridgeMetadata.Snapshot memory s, bytes32[] memory items, uint256 order) =
            freshnessWitness(index);
        vm.warp(uint256(s.sourceTimestampMs) / 1000);
        BeefyClient.Commitment memory c = commitment(index, 102, 0, fixtureHash(index, "mmrRoot"));
        client.submitFiatShamir(
            c,
            allSigners(index),
            signedProofs(client, c, index, fixtureBytes(index, "signedCommitment")),
            leaf,
            s,
            items,
            order
        );

        BeefyClient.Commitment memory missing = commitment(index, 103, 0, fixtureHash(index, "mmrRoot"));
        missing.payload = new BeefyClient.PayloadItem[](1);
        missing.payload[0] = BeefyClient.PayloadItem(bytes2("aa"), bytes("payload"));
        vm.expectRevert(BeefyClient.CommitmentNotRelevant.selector);
        client.submitFiatShamir(missing, allSigners(index), new BeefyClient.ValidatorProof[](0), leaf, s, items, order);

        BeefyClient.Commitment memory shortRoot = commitment(index, 104, 0, fixtureHash(index, "mmrRoot"));
        shortRoot.payload = new BeefyClient.PayloadItem[](1);
        shortRoot.payload[0] = BeefyClient.PayloadItem(bytes2("mh"), new bytes(31));
        vm.expectRevert(BeefyClient.InvalidMMRRootLength.selector);
        client.submitFiatShamir(
            shortRoot, allSigners(index), new BeefyClient.ValidatorProof[](0), leaf, s, items, order
        );

        BeefyClient.Commitment memory duplicate = commitment(index, 105, 0, fixtureHash(index, "mmrRoot"));
        duplicate.payload = new BeefyClient.PayloadItem[](3);
        duplicate.payload[0] = BeefyClient.PayloadItem(bytes2("aa"), bytes("payload"));
        duplicate.payload[1] = BeefyClient.PayloadItem(bytes2("mh"), abi.encodePacked(fixtureHash(index, "mmrRoot")));
        duplicate.payload[2] = BeefyClient.PayloadItem(bytes2("mh"), abi.encodePacked(fixtureHash(index, "mmrRoot")));
        vm.expectRevert(BeefyClient.InvalidCommitment.selector);
        client.submitFiatShamir(
            duplicate, allSigners(index), new BeefyClient.ValidatorProof[](0), leaf, s, items, order
        );

        BeefyClient.Commitment memory zeroRoot = commitment(index, 106, 0, bytes32(0));
        vm.expectRevert(BeefyClient.InvalidCommitment.selector);
        client.submitFiatShamir(zeroRoot, allSigners(index), new BeefyClient.ValidatorProof[](0), leaf, s, items, order);
    }

    function testConsensusRejectionsPreserveAuthenticatedCheckpoint() public {
        uint256 index = caseIndex("synthetic-mmr-3-primary");
        BeefyClient client = newClient(index);
        (BeefyClient.MMRLeaf memory leaf, VaraBridgeMetadata.Snapshot memory s, bytes32[] memory items, uint256 order) =
            freshnessWitness(index);
        vm.warp(uint256(s.sourceTimestampMs) / 1000);
        BeefyClient.Commitment memory c = commitment(index, 102, 0, fixtureHash(index, "mmrRoot"));
        bytes32 beforeState = stateHash(client);
        for (uint256 mutation; mutation < 4; mutation++) {
            uint256[] memory bits = allSigners(index);
            BeefyClient.ValidatorProof[] memory proofs =
                signedProofs(client, c, index, fixtureBytes(index, "signedCommitment"));
            if (mutation == 0) proofs[0].r = bytes32(uint256(1));
            if (mutation == 1) proofs[0].index = 1;
            if (mutation == 2) proofs[1] = proofs[0];
            if (mutation == 3) c.validatorSetID = 2;
            vm.expectRevert();
            client.submitFiatShamir(c, bits, proofs, leaf, s, items, order);
            assertEq(stateHash(client), beforeState);
            c.validatorSetID = 0;
        }
        client.submitFiatShamir(
            c,
            allSigners(index),
            signedProofs(client, c, index, fixtureBytes(index, "signedCommitment")),
            leaf,
            s,
            items,
            order
        );
        bytes32 accepted = stateHash(client);
        assertEq(client.latestBeefyBlock(), 102);
        BeefyClient.ValidatorProof[] memory staleProofs =
            signedProofs(client, c, index, fixtureBytes(index, "signedCommitment"));
        vm.expectRevert(BeefyClient.StaleCommitment.selector);
        client.submitFiatShamir(c, allSigners(index), staleProofs, leaf, s, items, order);
        assertEq(stateHash(client), accepted);
    }

    function testAuthenticatedHandoverRejectsMutatedLeaf() public {
        uint256 index = caseIndex("synthetic-mmr-3-primary");
        BeefyClient client = newClient(index);
        acceptFixture(client, index);
        (, VaraBridgeMetadata.Snapshot memory s,,) = freshnessWitness(index);
        BeefyClient.MMRLeaf memory leaf = decodeLeaf(fixtureBytes(index, "freshnessProof.outerLeaf"));
        leaf.parentNumber = 102;
        leaf.nextAuthoritySetID = 2;
        leaf.nextAuthoritySetRoot = keccak256("new-key-set");
        bytes32 root = keccak256(leafBytes(leaf));
        BeefyClient.Commitment memory c = commitment(index, 103, 1, root);
        BeefyClient.ValidatorProof[] memory proofs = signedProofs(client, c, index, "");
        bytes32 beforeState = stateHash(client);
        leaf.parentHash ^= bytes32(uint256(1));
        vm.expectRevert(BeefyClient.InvalidMMRLeafProof.selector);
        client.submitFiatShamir(c, allSigners(index), proofs, leaf, s, new bytes32[](0), 0);
        assertEq(stateHash(client), beforeState);
        leaf.parentHash ^= bytes32(uint256(1));
        client.submitFiatShamir(c, allSigners(index), proofs, leaf, s, new bytes32[](0), 0);
        (uint128 current,,,) = client.currentValidatorSet();
        assertEq(current, 1);
    }

    function testInteractiveEntryPointAcceptsGenuineSignatures() public {
        uint256 index = caseIndex("synthetic-mmr-3-primary");
        BeefyClient client = newClient(index);
        (BeefyClient.MMRLeaf memory leaf, VaraBridgeMetadata.Snapshot memory s, bytes32[] memory items, uint256 order) =
            freshnessWitness(index);
        BeefyClient.Commitment memory c = commitment(index, 102, 0, fixtureHash(index, "mmrRoot"));
        BeefyClient.ValidatorProof memory initial = validatorProof(index, 0);
        (initial.v, initial.r, initial.s) = vm.sign(1, client.computeCommitmentHash(c));
        client.submitInitial(c, allSigners(index), initial);
        vm.roll(block.number + 128);
        vm.prevrandao(bytes32(uint256(12345)));
        vm.warp(uint256(s.sourceTimestampMs) / 1000);
        client.commitPrevRandao(client.computeCommitmentHash(c));
        uint256[] memory selection = client.createFinalBitfield(client.computeCommitmentHash(c), allSigners(index));
        client.submitFinal(
            c, allSigners(index), selectedProofs(client, c, index, selection[0], ""), leaf, s, items, order
        );
        assertEq(client.latestMMRRoot(), fixtureHash(index, "mmrRoot"));
    }

    function testSubmitFinalRejectsDestinationDomainMismatch() public {
        uint256 index = caseIndex("synthetic-mmr-3-primary");
        uint256 destinationChainId = fixtureUint(index, "destinationChainId");
        address destinationQueue = vm.parseJsonAddress(fixtures, fixturePath(index, "destinationQueue"));
        BeefyClient client = newClient(index, destinationChainId, address(uint160(destinationQueue) + 1));
        (
            BeefyClient.MMRLeaf memory leaf,
            VaraBridgeMetadata.Snapshot memory snapshot,
            bytes32[] memory items,
            uint256 order
        ) = freshnessWitness(index);
        BeefyClient.Commitment memory c = commitment(index, 102, 0, fixtureHash(index, "mmrRoot"));
        uint256[] memory signers = allSigners(index);
        BeefyClient.ValidatorProof memory initial = validatorProof(index, 0);
        (initial.v, initial.r, initial.s) = vm.sign(1, client.computeCommitmentHash(c));
        client.submitInitial(c, signers, initial);
        vm.roll(block.number + 128);
        vm.prevrandao(bytes32(uint256(12345)));
        vm.warp(uint256(snapshot.sourceTimestampMs) / 1000);
        client.commitPrevRandao(client.computeCommitmentHash(c));
        uint256[] memory selection = client.createFinalBitfield(client.computeCommitmentHash(c), signers);
        BeefyClient.ValidatorProof[] memory proofs = selectedProofs(client, c, index, selection[0], "");

        vm.expectRevert(BeefyClient.InvalidSourceTimestamp.selector);
        client.submitFinal(c, signers, proofs, leaf, snapshot, items, order);
    }

    function testAuthoritySamplingGas59() public {
        _acceptNamedCase("synthetic-authorities-59");
    }

    function testAuthoritySamplingGas150() public {
        _acceptNamedCase("synthetic-authorities-150");
    }

    function testAuthoritySamplingGas256() public {
        _acceptNamedCase("synthetic-authorities-256");
    }

    function testAuthoritySamplingGas59Interactive() public {
        _acceptNamedCaseInteractive("synthetic-authorities-59");
    }

    function testAuthoritySamplingGas150Interactive() public {
        _acceptNamedCaseInteractive("synthetic-authorities-150");
    }

    function testAuthoritySamplingGas256Interactive() public {
        _acceptNamedCaseInteractive("synthetic-authorities-256");
    }

    function _acceptNamedCaseInteractive(string memory name) internal {
        uint256 index = caseIndex(name);
        BeefyClient client = newClient(index);
        (BeefyClient.MMRLeaf memory leaf, VaraBridgeMetadata.Snapshot memory s, bytes32[] memory items, uint256 order) =
            freshnessWitness(index);
        vm.warp(uint256(s.sourceTimestampMs) / 1000);
        BeefyClient.Commitment memory c =
            commitment(index, uint32(fixtureUint(index, "anchorBlock")), 0, fixtureHash(index, "mmrRoot"));
        bytes32 commitmentHash = client.computeCommitmentHash(c);
        BeefyClient.ValidatorProof memory initial = validatorProof(index, 0);
        (initial.v, initial.r, initial.s) = vm.sign(1, commitmentHash);
        client.submitInitial(c, allSigners(index), initial);
        vm.roll(block.number + 128);
        vm.prevrandao(bytes32(uint256(1)));
        client.commitPrevRandao(commitmentHash);
        uint256[] memory selection = client.createFinalBitfield(commitmentHash, allSigners(index));
        BeefyClient.ValidatorProof[] memory proofs =
            selectedProofs(client, c, index, selection[0], fixtureBytes(index, "signedCommitment"));
        uint256[] memory signers = allSigners(index);
        uint256 gasBefore = gasleft();
        client.submitFinal(c, signers, proofs, leaf, s, items, order);
        uint256 gasUsed = gasBefore - gasleft();
        emit log_named_uint(string.concat("submitFinal-N", vm.toString(fixtureUint(index, "validatorCount"))), gasUsed);
        assertEq(client.latestMMRRoot(), fixtureHash(index, "mmrRoot"));
        assertEq(client.lastAuthenticatedSourceTimestampMs(), fixtureUint(index, "freshnessProof.sourceTimestampMs"));
    }

    function testFreshnessMetadataAndLeafRejections() public {
        uint256 index = caseIndex("synthetic-mmr-3-primary");
        BeefyClient client = newClient(index);
        (BeefyClient.MMRLeaf memory leaf, VaraBridgeMetadata.Snapshot memory s, bytes32[] memory items, uint256 order) =
            freshnessWitness(index);
        BeefyClient.Commitment memory c = commitment(index, 102, 0, fixtureHash(index, "mmrRoot"));
        uint256[] memory bits = allSigners(index);
        BeefyClient.ValidatorProof[] memory proofs = signedProofs(client, c, index, "");
        bytes32 beforeState = stateHash(client);

        VaraBridgeMetadata.Snapshot memory badSnapshot = snapshot(index, true);
        badSnapshot.bridgeDomain ^= bytes32(uint256(1));
        vm.expectRevert();
        client.submitFiatShamir(c, bits, proofs, leaf, badSnapshot, items, order);
        assertEq(stateHash(client), beforeState);
        badSnapshot = snapshot(index, true);
        badSnapshot.bridgeDomain = bytes32(0);
        vm.expectRevert(BeefyClient.InvalidSourceTimestamp.selector);
        client.submitFiatShamir(c, bits, proofs, leaf, badSnapshot, items, order);
        assertEq(stateHash(client), beforeState);

        badSnapshot = snapshot(index, true);
        badSnapshot.version = 0;
        vm.expectRevert();
        client.submitFiatShamir(c, bits, proofs, leaf, badSnapshot, items, order);
        assertEq(stateHash(client), beforeState);

        BeefyClient.MMRLeaf memory badLeaf = leaf;
        badLeaf.parentNumber = 100;
        vm.expectRevert();
        client.submitFiatShamir(c, bits, proofs, badLeaf, s, items, order);
        assertEq(stateHash(client), beforeState);

        badSnapshot = snapshot(index, true);
        badSnapshot.sourceTimestampMs = client.lastAuthenticatedSourceTimestampMs() - 1;
        badLeaf = leafForSnapshot(badSnapshot, 101, 1, 3, fixtureHash(index, "authorityRoot"));
        c.payload[1].data = abi.encodePacked(keccak256(leafBytes(badLeaf)));
        proofs = signedProofs(client, c, index, "");
        vm.expectRevert(BeefyClient.InvalidSourceTimestamp.selector);
        client.submitFiatShamir(c, bits, proofs, badLeaf, badSnapshot, items, order);

        badLeaf = decodeLeaf(fixtureBytes(index, "outerLeaf"));
        vm.expectRevert();
        client.submitFiatShamir(c, bits, proofs, badLeaf, s, items, order);
        assertEq(stateHash(client), beforeState);
    }

    function testSourceTimestampFutureBoundariesFiatShamir() public {
        uint256 index = caseIndex("synthetic-mmr-3-primary");
        BeefyClient client = newClient(index);
        uint64 nowMs = uint64(block.timestamp * 1000);
        VaraBridgeMetadata.Snapshot memory s = snapshot(index, true);
        s.sourceTimestampMs = nowMs + 120_000;
        BeefyClient.MMRLeaf memory leaf = leafForSnapshot(s, 101, 1, 3, fixtureHash(index, "authorityRoot"));
        BeefyClient.Commitment memory c = commitment(index, 102, 0, keccak256(leafBytes(leaf)));
        uint256[] memory bits = allSigners(index);
        BeefyClient.ValidatorProof[] memory proofs = signedProofs(client, c, index, "");
        client.submitFiatShamir(c, bits, proofs, leaf, s, new bytes32[](0), 0);
        assertEq(client.lastAuthenticatedSourceTimestampMs(), s.sourceTimestampMs);

        VaraBridgeMetadata.Snapshot memory tooFar = snapshot(index, true);
        tooFar.sourceTimestampMs = nowMs + 120_001;
        BeefyClient.MMRLeaf memory tooFarLeaf = leafForSnapshot(tooFar, 102, 1, 3, fixtureHash(index, "authorityRoot"));
        BeefyClient.Commitment memory tooFarCommitment = commitment(index, 103, 0, keccak256(leafBytes(tooFarLeaf)));
        BeefyClient.ValidatorProof[] memory tooFarProofs = signedProofs(client, tooFarCommitment, index, "");
        vm.expectRevert(BeefyClient.InvalidSourceTimestamp.selector);
        client.submitFiatShamir(tooFarCommitment, bits, tooFarProofs, tooFarLeaf, tooFar, new bytes32[](0), 0);
        assertEq(client.lastAuthenticatedSourceTimestampMs(), s.sourceTimestampMs);
    }

    function testSourceAgeBoundaryDoesNotExtendWithEqualTimestamp() public {
        uint256 index = caseIndex("synthetic-mmr-3-primary");
        BeefyClient client = newClient(index);
        uint64 initialTimestamp = client.lastAuthenticatedSourceTimestampMs();
        vm.warp(uint256(initialTimestamp) / 1000 + 86_400);
        VaraBridgeMetadata.Snapshot memory s = snapshot(index, true);
        s.sourceTimestampMs = initialTimestamp;
        BeefyClient.MMRLeaf memory leaf = leafForSnapshot(s, 101, 1, 3, fixtureHash(index, "authorityRoot"));
        BeefyClient.Commitment memory c = commitment(index, 102, 0, keccak256(leafBytes(leaf)));
        uint256[] memory bits = allSigners(index);
        BeefyClient.ValidatorProof[] memory proofs = signedProofs(client, c, index, "");
        client.submitFiatShamir(c, bits, proofs, leaf, s, new bytes32[](0), 0);
        assertTrue(client.isLive());
        bytes32 acceptedState = stateHash(client);

        vm.warp(uint256(initialTimestamp) / 1000 + 86_401);
        BeefyClient.MMRLeaf memory expiredLeaf = leafForSnapshot(s, 102, 1, 3, fixtureHash(index, "authorityRoot"));
        BeefyClient.Commitment memory expired = commitment(index, 103, 0, keccak256(leafBytes(expiredLeaf)));
        BeefyClient.ValidatorProof[] memory expiredProofs = signedProofs(client, expired, index, "");
        vm.expectRevert(BeefyClient.ClientExpired.selector);
        client.submitFiatShamir(expired, bits, expiredProofs, expiredLeaf, s, new bytes32[](0), 0);
        assertEq(stateHash(client), acceptedState);
    }

    function testExpiredClientRejectsInitialAndFinalWithoutConsumingTicket() public {
        uint256 index = caseIndex("synthetic-mmr-3-primary");
        BeefyClient.Commitment memory c = commitment(index, 102, 0, fixtureHash(index, "mmrRoot"));

        BeefyClient client = newClient(index);
        BeefyClient.ValidatorProof memory initial = validatorProof(index, 0);
        bytes32 commitmentHash = client.computeCommitmentHash(c);
        (initial.v, initial.r, initial.s) = vm.sign(1, commitmentHash);
        vm.warp(uint256(client.lastAuthenticatedSourceTimestampMs()) / 1000 + 86_401);
        bytes32 beforeState = stateHash(client);
        vm.expectRevert(BeefyClient.ClientExpired.selector);
        client.submitInitial(c, allSigners(index), initial);
        assertEq(stateHash(client), beforeState);

        client = newClient(index);
        (initial.v, initial.r, initial.s) = vm.sign(1, client.computeCommitmentHash(c));
        client.submitInitial(c, allSigners(index), initial);
        vm.roll(block.number + 128);
        vm.prevrandao(bytes32(uint256(1)));
        client.commitPrevRandao(client.computeCommitmentHash(c));
        uint256[] memory bits = client.createFinalBitfield(client.computeCommitmentHash(c), allSigners(index));
        BeefyClient.ValidatorProof[] memory proofs = selectedProofs(client, c, index, bits[0], "");
        (BeefyClient.MMRLeaf memory leaf, VaraBridgeMetadata.Snapshot memory s, bytes32[] memory items, uint256 order) =
            freshnessWitness(index);
        bytes32 ticketId = keccak256(abi.encode(address(this), client.computeCommitmentHash(c)));
        (uint64 ticketBlock,,,,) = client.tickets(ticketId);
        assertGt(ticketBlock, 0);
        vm.warp(uint256(client.lastAuthenticatedSourceTimestampMs()) / 1000 + 86_401);
        beforeState = stateHash(client);
        vm.expectRevert(BeefyClient.ClientExpired.selector);
        client.submitFinal(c, allSigners(index), proofs, leaf, s, items, order);
        assertEq(stateHash(client), beforeState);
        (uint64 remainingTicket,,,,) = client.tickets(ticketId);
        assertEq(remainingTicket, ticketBlock);
    }

    function testInteractiveAuthenticatedHandover() public {
        uint256 index = caseIndex("synthetic-mmr-3-primary");
        BeefyClient client = newClient(index);
        (BeefyClient.MMRLeaf memory leaf, VaraBridgeMetadata.Snapshot memory s, bytes32[] memory items, uint256 order) =
            freshnessWitness(index);
        vm.warp(uint256(s.sourceTimestampMs) / 1000);

        BeefyClient.Commitment memory c = commitment(index, 102, 0, fixtureHash(index, "mmrRoot"));
        bytes32 commitmentHash = client.computeCommitmentHash(c);
        BeefyClient.ValidatorProof memory initial = validatorProof(index, 0);
        (initial.v, initial.r, initial.s) = vm.sign(1, commitmentHash);
        uint256[] memory signers = allSigners(index);
        client.submitInitial(c, signers, initial);
        vm.roll(block.number + 128);
        vm.prevrandao(bytes32(uint256(1)));
        client.commitPrevRandao(commitmentHash);
        uint256[] memory selection = client.createFinalBitfield(commitmentHash, signers);
        client.submitFinal(c, signers, selectedProofs(client, c, index, selection[0], ""), leaf, s, items, order);

        BeefyClient.MMRLeaf memory handoverLeaf = leafForSnapshot(s, 102, 2, 3, fixtureHash(index, "authorityRoot"));
        BeefyClient.Commitment memory handover = commitment(index, 103, 1, keccak256(leafBytes(handoverLeaf)));
        bytes32 handoverHash = client.computeCommitmentHash(handover);
        (initial.v, initial.r, initial.s) = vm.sign(1, handoverHash);
        client.submitInitial(handover, signers, initial);
        vm.roll(block.number + 128);
        vm.prevrandao(bytes32(uint256(2)));
        client.commitPrevRandao(handoverHash);
        selection = client.createFinalBitfield(handoverHash, signers);
        client.submitFinal(
            handover,
            signers,
            selectedProofs(client, handover, index, selection[0], ""),
            handoverLeaf,
            s,
            new bytes32[](0),
            0
        );

        (uint128 current,,,) = client.currentValidatorSet();
        assertEq(current, 1);
        assertEq(client.latestBeefyBlock(), 103);
        assertEq(client.latestMMRRoot(), keccak256(leafBytes(handoverLeaf)));
    }

    function testFiatShamirQuorumBoundaries59() public {
        _assertFiatShamirQuorumBoundaries("synthetic-authorities-59");
    }

    function testFiatShamirQuorumBoundaries150() public {
        _assertFiatShamirQuorumBoundaries("synthetic-authorities-150");
    }

    function testFiatShamirQuorumBoundaries256() public {
        _assertFiatShamirQuorumBoundaries("synthetic-authorities-256");
    }

    function _assertFiatShamirQuorumBoundaries(string memory name) internal {
        uint256 index = caseIndex(name);
        BeefyClient client = newClient(index);
        (BeefyClient.MMRLeaf memory leaf, VaraBridgeMetadata.Snapshot memory s, bytes32[] memory items, uint256 order) =
            freshnessWitness(index);
        vm.warp(uint256(s.sourceTimestampMs) / 1000);
        BeefyClient.Commitment memory c =
            commitment(index, uint32(fixtureUint(index, "anchorBlock")), 0, fixtureHash(index, "mmrRoot"));
        uint256[] memory all = allSigners(index);
        uint256[] memory selection = client.createFiatShamirFinalBitfield(c, all);
        BeefyClient.ValidatorProof[] memory proofs = selectedProofs(client, c, index, selection[0], "");
        bytes32 beforeState = stateHash(client);
        uint256 count = fixtureUint(index, "validatorCount");

        uint256[] memory badPadding;
        if (count == 256) {
            badPadding = new uint256[](2);
            badPadding[0] = all[0];
            badPadding[1] = 1;
        } else {
            badPadding = new uint256[](1);
            badPadding[0] = all[0] | (uint256(1) << count);
        }
        vm.expectRevert();
        client.submitFiatShamir(c, badPadding, proofs, leaf, s, items, order);
        assertEq(stateHash(client), beforeState);

        uint256 firstSelected;
        while ((selection[0] & (uint256(1) << firstSelected)) == 0) firstSelected++;
        uint256[] memory missing = new uint256[](1);
        missing[0] = selection[0] & ~(uint256(1) << firstSelected);
        vm.expectRevert();
        client.submitFiatShamir(c, missing, proofs, leaf, s, items, order);
        assertEq(stateHash(client), beforeState);

        BeefyClient.ValidatorProof[] memory duplicate = new BeefyClient.ValidatorProof[](proofs.length);
        for (uint256 i; i < proofs.length; i++) {
            duplicate[i] = proofs[i];
        }
        duplicate[1] = duplicate[0];
        vm.expectRevert(BeefyClient.InvalidValidatorProof.selector);
        client.submitFiatShamir(c, all, duplicate, leaf, s, items, order);
        assertEq(stateHash(client), beforeState);
    }

    function testBootstrapAndValidatorBounds() public {
        uint64 timestamp = uint64(fixtureUint(0, "sourceTimestampMs"));
        vm.warp(uint256(timestamp) / 1000);
        bytes32 sourceDomain = fixtureHash(0, "sourceDomain");
        uint256 destinationChainId = fixtureUint(0, "destinationChainId");
        address destinationQueue = vm.parseJsonAddress(fixtures, fixturePath(0, "destinationQueue"));
        bytes32 root = fixtureHash(0, "authorityRoot");
        BeefyClient.ValidatorSet memory current = BeefyClient.ValidatorSet(0, 3, root);
        BeefyClient.ValidatorSet memory next = BeefyClient.ValidatorSet(1, 3, root);

        vm.expectRevert(BeefyClient.InvalidBootstrap.selector);
        new BeefyClient(sourceDomain, destinationChainId, destinationQueue, 0, 2, timestamp, current, next);

        vm.expectRevert(BeefyClient.InvalidBootstrap.selector);
        new BeefyClient(bytes32(0), destinationChainId, destinationQueue, 1, 2, timestamp, current, next);
        vm.expectRevert(BeefyClient.InvalidBootstrap.selector);
        new BeefyClient(sourceDomain, destinationChainId + 1, destinationQueue, 1, 2, timestamp, current, next);
        vm.expectRevert(BeefyClient.InvalidBootstrap.selector);
        new BeefyClient(sourceDomain, destinationChainId, address(0), 1, 2, timestamp, current, next);

        vm.expectRevert(BeefyClient.InvalidValidatorSet.selector);
        new BeefyClient(
            sourceDomain,
            destinationChainId,
            destinationQueue,
            1,
            2,
            timestamp,
            BeefyClient.ValidatorSet(0, 257, root),
            next
        );

        vm.expectRevert(BeefyClient.InvalidValidatorSet.selector);
        new BeefyClient(
            sourceDomain,
            destinationChainId,
            destinationQueue,
            1,
            2,
            timestamp,
            current,
            BeefyClient.ValidatorSet(1, 257, root)
        );

        vm.expectRevert(BeefyClient.InvalidValidatorSet.selector);
        new BeefyClient(
            sourceDomain,
            destinationChainId,
            destinationQueue,
            1,
            2,
            timestamp,
            BeefyClient.ValidatorSet(type(uint128).max, 3, root),
            BeefyClient.ValidatorSet(0, 3, root)
        );
    }

    function testDestinationChainAndQueueRejectFixtureReplay() public {
        BeefyClient original = newClient(0);
        uint256 originalChainId = original.destinationChainId();
        address originalQueue = original.destinationQueue();
        BeefyClient otherSourceClient =
            newClient(0, original.sourceDomain() ^ bytes32(uint256(1)), originalChainId, originalQueue);
        assertNotEq(otherSourceClient.bridgeDomain(), original.bridgeDomain());
        rejectFixtureForClient(otherSourceClient, 0);

        BeefyClient otherQueueClient = newClient(0, originalChainId, address(uint160(originalQueue) + 1));
        assertNotEq(otherQueueClient.bridgeDomain(), original.bridgeDomain());
        rejectFixtureForClient(otherQueueClient, 0);

        vm.chainId(originalChainId + 1);
        BeefyClient otherChainClient = newClient(0, originalChainId + 1, originalQueue);
        assertNotEq(otherChainClient.bridgeDomain(), original.bridgeDomain());
        rejectFixtureForClient(otherChainClient, 0);
        vm.chainId(originalChainId);
    }

    function _acceptNamedCase(string memory name) internal {
        uint256 index = caseIndex(name);
        BeefyClient client = newClient(index);
        acceptFixture(client, index, true);
        assertEq(client.latestMMRRoot(), fixtureHash(index, "mmrRoot"));
        assertEq(client.lastAuthenticatedSourceTimestampMs(), fixtureUint(index, "freshnessProof.sourceTimestampMs"));
    }

    function prepareProofs(BeefyClient client, BeefyClient.Commitment memory c, uint256 index, bool interactive)
        internal
        returns (BeefyClient.ValidatorProof[] memory)
    {
        if (!interactive) return signedProofs(client, c, index, "");
        bytes32 digest = client.computeCommitmentHash(c);
        BeefyClient.ValidatorProof memory initial = validatorProof(index, 0);
        (initial.v, initial.r, initial.s) = vm.sign(1, digest);
        uint256[] memory bits = allSigners(index);
        client.submitInitial(c, bits, initial);
        vm.roll(block.number + 128);
        vm.prevrandao(bytes32(uint256(1)));
        client.commitPrevRandao(digest);
        uint256[] memory selection = client.createFinalBitfield(digest, bits);
        return selectedProofs(client, c, index, selection[0], "");
    }


    function ticketHash(BeefyClient client, BeefyClient.Commitment memory c) internal view returns (bytes32) {
        bytes32 id = keccak256(abi.encode(address(this), client.computeCommitmentHash(c)));
        (uint64 height, uint32 length, uint32 required, uint256 seed, bytes32 bits) = client.tickets(id);
        return keccak256(abi.encode(height, length, required, seed, bits));
    }

    function testNewestWitnessFailuresBothPathsAndSets() public {
        vm.pauseGasMetering();
        for (uint256 mode; mode < 2; mode++) {
            for (uint64 set; set < 2; set++) {
                for (uint256 mutation; mutation < 15; mutation++) {
                    BeefyClient client = newClient(0);
                    VaraBridgeMetadata.Snapshot memory s = snapshot(0, true);
                    if (mutation == 0) s.bridgeDomain ^= bytes32(uint256(1));
                    if (mutation == 6) s.sourceTimestampMs = client.lastAuthenticatedSourceTimestampMs() - 1;
                    if (mutation == 7) s.sourceTimestampMs = uint64(block.timestamp * 1000 + 120_001);
                    BeefyClient.MMRLeaf memory leaf =
                        leafForSnapshot(s, 101, set + 1, 3, fixtureHash(0, "authorityRoot"));
                    if (mutation == 3) leaf.version = 1;
                    if (mutation == 4 || mutation == 5) leaf.parentNumber = 100;
                    if (mutation == 8) leaf.nextAuthoritySetLen = 0;
                    if (mutation == 9) leaf.nextAuthoritySetLen = 257;
                    if (mutation == 10) leaf.nextAuthoritySetRoot = bytes32(0);
                    if (mutation == 14) leaf.parentNumber = 102;
                    BeefyClient.Commitment memory c = commitment(0, 102, set, keccak256(leafBytes(leaf)));
                    if (mutation == 1) s.version = 0;
                    if (mutation == 2) s.queueId++;
                    if (mutation == 11) {
                        s.initialized = false;
                        s.queueId = 1;
                    }
                    bytes32[] memory items = new bytes32[](mutation == 12 ? 1 : 0);
                    uint256 order = mutation == 13 ? 1 : 0;
                    BeefyClient.ValidatorProof[] memory proofs = prepareProofs(client, c, 0, mode == 1);
                    uint256[] memory bits = allSigners(0);
                    bytes32 beforeState = stateHash(client);
                    bytes32 beforeTicket = ticketHash(client, c);
                    vm.expectRevert();
                    finishUpdate(mode == 1, client, c, bits, proofs, leaf, s, items, order);
                    assertEq(stateHash(client), beforeState);
                    assertEq(ticketHash(client, c), beforeTicket);
                }
            }
        }
        vm.resumeGasMetering();
    }

    function testFutureAndExpiryBoundariesBothPathsAndSets() public {
        vm.pauseGasMetering();
        for (uint256 mode; mode < 2; mode++) {
            for (uint64 set; set < 2; set++) {
                BeefyClient client = newClient(0);
                VaraBridgeMetadata.Snapshot memory s = snapshot(0, true);
                s.sourceTimestampMs = uint64(vm.getBlockTimestamp() * 1000 + 120_000);
                BeefyClient.MMRLeaf memory leaf = leafForSnapshot(s, 101, set + 1, 3, fixtureHash(0, "authorityRoot"));
                BeefyClient.Commitment memory c = commitment(0, 102, set, keccak256(leafBytes(leaf)));
                uint256[] memory bits = allSigners(0);
                BeefyClient.ValidatorProof[] memory proofs = prepareProofs(client, c, 0, mode == 1);
                finishUpdate(mode == 1, client, c, bits, proofs, leaf, s, new bytes32[](0), 0);
                assertEq(client.lastAuthenticatedSourceTimestampMs(), s.sourceTimestampMs);
                assertEq(client.latestBeefyBlock(), 102);
                (uint128 current,,,) = client.currentValidatorSet();
                assertEq(current, set);

                client = newClient(0);
                uint64 trusted = client.lastAuthenticatedSourceTimestampMs();
                s = snapshot(0, true);
                s.sourceTimestampMs = trusted;
                vm.warp(uint256(trusted) / 1000 + 86_400);
                leaf = leafForSnapshot(s, 101, set + 1, 3, fixtureHash(0, "authorityRoot"));
                c = commitment(0, 102, set, keccak256(leafBytes(leaf)));
                proofs = prepareProofs(client, c, 0, mode == 1);
                finishUpdate(mode == 1, client, c, bits, proofs, leaf, s, new bytes32[](0), 0);
                assertTrue(client.isLive());
                assertEq(client.lastAuthenticatedSourceTimestampMs(), trusted);
                s.sourceTimestampMs = trusted + 86_401_000;
                leaf = leafForSnapshot(s, 102, set + 1, 3, fixtureHash(0, "authorityRoot"));
                c = commitment(0, 103, set, keccak256(leafBytes(leaf)));
                proofs = prepareProofs(client, c, 0, mode == 1);
                bytes32 beforeState = stateHash(client);
                bytes32 beforeTicket = ticketHash(client, c);
                vm.warp(uint256(trusted) / 1000 + 86_401);
                vm.expectRevert(BeefyClient.ClientExpired.selector);
                finishUpdate(mode == 1, client, c, bits, proofs, leaf, s, new bytes32[](0), 0);
                assertEq(stateHash(client), beforeState);
                assertEq(ticketHash(client, c), beforeTicket);
            }
        }
        vm.resumeGasMetering();
    }

    function testMissingSelectedSignaturesBothPathsAllSizes() public {
        vm.pauseGasMetering();
        string[3] memory names =
            [string("synthetic-authorities-59"), "synthetic-authorities-150", "synthetic-authorities-256"];
        for (uint256 size; size < names.length; size++) {
            uint256 index = caseIndex(names[size]);
            for (uint256 mode; mode < 2; mode++) {
                BeefyClient client = newClient(index);
                (
                    BeefyClient.MMRLeaf memory leaf,
                    VaraBridgeMetadata.Snapshot memory s,
                    bytes32[] memory items,
                    uint256 order
                ) = freshnessWitness(index);
                BeefyClient.Commitment memory c =
                    commitment(index, uint32(fixtureUint(index, "anchorBlock")), 0, fixtureHash(index, "mmrRoot"));
                uint256[] memory bits = allSigners(index);
                if (mode == 1) {
                    uint256 count = fixtureUint(index, "validatorCount");
                    BeefyClient.ValidatorProof memory initial = validatorProof(index, 0);
                    (initial.v, initial.r, initial.s) = vm.sign(1, client.computeCommitmentHash(c));
                    uint256[] memory invalidBits = new uint256[](count == 256 ? 2 : 1);
                    invalidBits[0] = count == 256 ? bits[0] : bits[0] | (uint256(1) << count);
                    if (count == 256) invalidBits[1] = 1;
                    bytes32 initialState = stateHash(client);
                    bytes32 initialTicket = ticketHash(client, c);
                    vm.expectRevert();
                    client.submitInitial(c, invalidBits, initial);
                    invalidBits = new uint256[](1);
                    invalidBits[0] = (uint256(1) << (count / 3 + 1)) - 1;
                    vm.expectRevert(BeefyClient.InvalidBitfield.selector);
                    client.submitInitial(c, invalidBits, initial);
                    assertEq(stateHash(client), initialState);
                    assertEq(ticketHash(client, c), initialTicket);
                }
                BeefyClient.ValidatorProof[] memory proofs = prepareProofs(client, c, index, mode == 1);
                assertEq(proofs.length, fixtureUint(index, "validatorCount") / 3 + 1);
                BeefyClient.ValidatorProof[] memory shortProof = new BeefyClient.ValidatorProof[](proofs.length - 1);
                for (uint256 i; i < shortProof.length; i++) {
                    shortProof[i] = proofs[i];
                }
                bytes32 beforeState = stateHash(client);
                bytes32 beforeTicket = ticketHash(client, c);
                vm.expectRevert(BeefyClient.InvalidValidatorProofLength.selector);
                finishUpdate(mode == 1, client, c, bits, shortProof, leaf, s, items, order);
                assertEq(stateHash(client), beforeState);
                assertEq(ticketHash(client, c), beforeTicket);
                BeefyClient.ValidatorProof memory saved = proofs[1];
                proofs[1] = proofs[0];
                vm.expectRevert(BeefyClient.InvalidValidatorProof.selector);
                finishUpdate(mode == 1, client, c, bits, proofs, leaf, s, items, order);
                assertEq(stateHash(client), beforeState);
                assertEq(ticketHash(client, c), beforeTicket);
                proofs[1] = saved;
                finishUpdate(mode == 1, client, c, bits, proofs, leaf, s, items, order);
                assertEq(client.latestMMRRoot(), fixtureHash(index, "mmrRoot"));
            }
        }
        vm.resumeGasMetering();
    }

    function testRepeatedAccountCannotAliasValidatorPosition() public {
        BeefyClient client = newClient(0);
        VaraBridgeMetadata.Snapshot memory s = snapshot(0, true);
        BeefyClient.MMRLeaf memory leaf = leafForSnapshot(s, 101, 1, 3, fixtureHash(0, "authorityRoot"));
        BeefyClient.Commitment memory c = commitment(0, 102, 0, keccak256(leafBytes(leaf)));
        uint256[] memory bits = allSigners(0);
        BeefyClient.ValidatorProof[] memory proofs = selectedProofs(client, c, 0, 7, "");
        proofs[1] = validatorProof(0, 2);
        proofs[1].index = 1;
        (proofs[1].v, proofs[1].r, proofs[1].s) = vm.sign(3, client.computeCommitmentHash(c));
        bytes32 beforeState = stateHash(client);
        vm.expectRevert(BeefyClient.InvalidValidatorProof.selector);
        client.submitFiatShamir(c, bits, proofs, leaf, s, new bytes32[](0), 0);
        assertEq(stateHash(client), beforeState);
        vm.expectRevert(BeefyClient.InvalidValidatorProof.selector);
        client.submitInitial(c, bits, proofs[1]);
        client.submitInitial(c, bits, proofs[2]);
    }

    function testTinySetsAuthenticateNativeQuorumBothPaths() public {
        vm.pauseGasMetering();
        string[3] memory names =
            [string("synthetic-authorities-1"), "synthetic-authorities-2", "synthetic-mmr-3-primary"];
        for (uint256 mode; mode < 2; mode++) {
            for (uint256 size; size < names.length; size++) {
                uint256 index = caseIndex(names[size]);
                BeefyClient client = newClient(index);
                (
                    BeefyClient.MMRLeaf memory leaf,
                    VaraBridgeMetadata.Snapshot memory s,
                    bytes32[] memory items,
                    uint256 order
                ) = freshnessWitness(index);
                vm.warp(uint256(s.sourceTimestampMs) / 1000);
                BeefyClient.Commitment memory c = commitment(index, 102, 0, fixtureHash(index, "mmrRoot"));
                BeefyClient.ValidatorProof[] memory proofs = prepareProofs(client, c, index, mode == 1);
                assertEq(proofs.length, size + 1);
                finishUpdate(mode == 1, client, c, allSigners(index), proofs, leaf, s, items, order);
                assertEq(client.latestMMRRoot(), fixtureHash(index, "mmrRoot"));
            }
        }
    }

    function testTwoValidatorsRejectOneSignerGrindingBothPathsAndSets() public {
        vm.pauseGasMetering();
        uint256 index = caseIndex("synthetic-authorities-2");
        for (uint256 mode; mode < 2; mode++) {
            for (uint64 set; set < 2; set++) {
                BeefyClient client = newClient(index);
                VaraBridgeMetadata.Snapshot memory s = snapshot(index, true);
                vm.warp(uint256(s.sourceTimestampMs) / 1000);
                uint256[] memory bits = allSigners(index);
                bytes32 beforeState = stateHash(client);
                for (uint256 attempt; attempt < 16; attempt++) {
                    BeefyClient.MMRLeaf memory leaf =
                        leafForSnapshot(s, 101, set + 1, 2, fixtureHash(index, "authorityRoot"));
                    leaf.parentHash = keccak256(abi.encode(attempt));
                    BeefyClient.Commitment memory c = commitment(index, 102, set, keccak256(leafBytes(leaf)));
                    BeefyClient.ValidatorProof[] memory one = selectedProofs(client, c, index, 1, "");
                    if (mode == 1) {
                        client.submitInitial(c, bits, one[0]);
                        vm.roll(block.number + 128);
                        vm.prevrandao(bytes32(attempt + 1));
                        client.commitPrevRandao(client.computeCommitmentHash(c));
                    }
                    vm.expectRevert();
                    finishUpdate(mode == 1, client, c, bits, one, leaf, s, new bytes32[](0), 0);
                    assertEq(stateHash(client), beforeState);
                    BeefyClient.ValidatorProof[] memory invalid = new BeefyClient.ValidatorProof[](2);
                    invalid[0] = one[0];
                    invalid[1] = one[0];
                    vm.expectRevert();
                    finishUpdate(mode == 1, client, c, bits, invalid, leaf, s, new bytes32[](0), 0);
                    assertEq(stateHash(client), beforeState);
                    invalid[1] = validatorProof(index, 1);
                    vm.expectRevert();
                    finishUpdate(mode == 1, client, c, bits, invalid, leaf, s, new bytes32[](0), 0);
                    assertEq(stateHash(client), beforeState);
                    if (attempt == 15) {
                        BeefyClient.ValidatorProof[] memory both = selectedProofs(client, c, index, 3, "");
                        finishUpdate(mode == 1, client, c, bits, both, leaf, s, new bytes32[](0), 0);
                        assertEq(client.latestMMRRoot(), keccak256(leafBytes(leaf)));
                        (uint128 current,,,) = client.currentValidatorSet();
                        assertEq(current, set);
                    }
                }
            }
        }
    }

    function testInteractiveRandaoBoundaryBlocksTinySet() public {
        uint256 index = caseIndex("synthetic-authorities-2");
        uint256[4] memory offsets = [uint256(127), 128, 152, 153];
        for (uint256 i; i < offsets.length; i++) {
            BeefyClient client = newClient(index);
            (
                BeefyClient.MMRLeaf memory leaf,
                VaraBridgeMetadata.Snapshot memory s,
                bytes32[] memory items,
                uint256 order
            ) = freshnessWitness(index);
            vm.warp(uint256(s.sourceTimestampMs) / 1000);
            BeefyClient.Commitment memory c = commitment(index, 102, 0, fixtureHash(index, "mmrRoot"));
            BeefyClient.ValidatorProof[] memory proofs = selectedProofs(client, c, index, 3, "");
            uint256[] memory bits = allSigners(index);
            bytes32 digest = client.computeCommitmentHash(c);
            bytes32 beforeState = stateHash(client);
            client.submitInitial(c, bits, proofs[0]);
            vm.roll(block.number + offsets[i]);
            vm.prevrandao(bytes32(uint256(1)));
            if (offsets[i] == 127) vm.expectRevert(BeefyClient.WaitPeriodNotOver.selector);
            client.commitPrevRandao(digest);
            if (offsets[i] == 128 || offsets[i] == 152) {
                assertEq(client.createFinalBitfield(digest, bits)[0], 3);
                client.submitFinal(c, bits, proofs, leaf, s, items, order);
                assertEq(client.latestMMRRoot(), fixtureHash(index, "mmrRoot"));
            } else {
                vm.expectRevert();
                client.submitFinal(c, bits, proofs, leaf, s, items, order);
                assertEq(stateHash(client), beforeState);
            }
        }
    }

    function testBootstrapRejectsZeroSetsAndInvalidCheckpoint() public {
        uint64 timestamp = uint64(fixtureUint(0, "sourceTimestampMs"));
        vm.warp(uint256(timestamp) / 1000);
        bytes32 sourceDomain = fixtureHash(0, "sourceDomain");
        uint256 destinationChainId = fixtureUint(0, "destinationChainId");
        address destinationQueue = vm.parseJsonAddress(fixtures, fixturePath(0, "destinationQueue"));
        bytes32 root = fixtureHash(0, "authorityRoot");
        for (uint256 mutation; mutation < 12; mutation++) {
            BeefyClient.ValidatorSet memory current = BeefyClient.ValidatorSet(0, 3, root);
            BeefyClient.ValidatorSet memory next = BeefyClient.ValidatorSet(1, 3, root);
            if (mutation == 0) current.length = 0;
            if (mutation == 1) next.length = 0;
            if (mutation == 2) current.root = bytes32(0);
            if (mutation == 3) next.root = bytes32(0);
            if (mutation == 4) next.id = uint128(type(uint64).max) + 1;
            vm.expectRevert();
            new BeefyClient(
                mutation == 5 ? bytes32(0) : sourceDomain,
                mutation == 10 ? destinationChainId + 1 : destinationChainId,
                mutation == 11 ? address(0) : destinationQueue,
                1,
                mutation == 6 ? 1 : mutation == 7 ? uint64(type(uint32).max) + 1 : 2,
                mutation == 8 ? 0 : mutation == 9 ? timestamp + 120_001 : timestamp,
                current,
                next
            );
        }
    }
    function testCapacity256AuthenticatedHandoverBothPaths() public {
        uint256 index = caseIndex("synthetic-authorities-256");
        for (uint256 mode; mode < 2; mode++) {
            BeefyClient client = newClient(index);
            VaraBridgeMetadata.Snapshot memory s = snapshot(index, true);
            vm.warp(uint256(s.sourceTimestampMs) / 1000);
            uint256[] memory bits = allSigners(index);
            for (uint64 set; set < 2; set++) {
                BeefyClient.MMRLeaf memory leaf = leafForSnapshot(s, uint32(client.latestBeefyBlock()),
                    set + 1, 256, fixtureHash(index, "authorityRoot"));
                BeefyClient.Commitment memory c = commitment(index, leaf.parentNumber + 1, set,
                    keccak256(leafBytes(leaf)));
                BeefyClient.ValidatorProof[] memory proofs = prepareProofs(client, c, index, mode == 1);
                assertEq(proofs.length, 86);
                finishUpdate(mode == 1, client, c, bits, proofs, leaf, s, new bytes32[](0), 0);
            }
            (uint128 current, uint128 count,,) = client.currentValidatorSet();
            assertEq(current, 1);
            assertEq(count, 256);
            (uint128 next,,, Uint16Array memory nextCounters) = client.nextValidatorSet();
            assertEq(next, 2);
            assertEq(nextCounters.length, 256);
            assertEq(nextCounters.data.length, 16);
        }
    }

}

// Real secp256k1 signatures and positional Substrate trees, including odd promoted nodes.
contract BeefyCapacityTest is BeefyFixtureTest {
    function authorityTree(uint256 n, bool repeated) internal returns (bytes32[][] memory levels) {
        uint256 depth = 1;
        for (uint256 width = n; width > 1; width = (width + 1) / 2) depth++;
        levels = new bytes32[][](depth);
        levels[0] = new bytes32[](n);
        for (uint256 i; i < n; i++) levels[0][i] = keccak256(abi.encodePacked(vm.addr(repeated ? 1 : i + 1)));
        for (uint256 d = 1; d < depth; d++) {
            bytes32[] memory previous = levels[d - 1];
            levels[d] = new bytes32[]((previous.length + 1) / 2);
            for (uint256 i; i < previous.length; i += 2) {
                levels[d][i / 2] = i + 1 == previous.length
                    ? previous[i] : keccak256(abi.encodePacked(previous[i], previous[i + 1]));
            }
        }
    }

    function capacityClient(bytes32[][] memory levels) internal returns (BeefyClient client) {
        vm.warp(1_700_000_000);
        bytes32 root = levels[levels.length - 1][0];
        client = new BeefyClient(bytes32(uint256(1)), block.chainid, address(0x1234), 100, 101,
            uint64(block.timestamp * 1000), BeefyClient.ValidatorSet(0, uint128(levels[0].length), root),
            BeefyClient.ValidatorSet(1, uint128(levels[0].length), root));
    }

    function capacityProof(bytes32[][] memory levels, uint256 index, bytes32 digest, bool repeated)
        internal returns (BeefyClient.ValidatorProof memory p)
    {
        p.index = index;
        p.account = vm.addr(repeated ? 1 : index + 1);
        uint256 count;
        uint256 position = index;
        for (uint256 d; d + 1 < levels.length; d++) {
            if ((position ^ 1) < levels[d].length) count++;
            position >>= 1;
        }
        p.proof = new bytes32[](count);
        position = index;
        count = 0;
        for (uint256 d; d + 1 < levels.length; d++) {
            if ((position ^ 1) < levels[d].length) p.proof[count++] = levels[d][position ^ 1];
            position >>= 1;
        }
        (p.v, p.r, p.s) = vm.sign(repeated ? 1 : index + 1, digest);
    }

    function capacityWitness(BeefyClient client, bytes32[][] memory levels, uint64 set)
        internal view returns (BeefyClient.Commitment memory c, BeefyClient.MMRLeaf memory leaf,
            VaraBridgeMetadata.Snapshot memory s)
    {
        s = VaraBridgeMetadata.Snapshot(2, client.bridgeDomain(), uint64(block.timestamp * 1000), true, 0,
            bytes32(uint256(42)));
        leaf = leafForSnapshot(s, uint32(client.latestBeefyBlock()), set + 1, uint32(levels[0].length),
            levels[levels.length - 1][0]);
        c.blockNumber = leaf.parentNumber + 1;
        c.validatorSetID = set;
        c.payload = new BeefyClient.PayloadItem[](1);
        c.payload[0] = BeefyClient.PayloadItem(bytes2("mh"), abi.encodePacked(keccak256(leafBytes(leaf))));
    }

    function capacityBits(BeefyClient client, uint256 n, uint256 signers) internal view returns (uint256[] memory) {
        uint256[] memory positions = new uint256[](signers);
        for (uint256 i; i < signers; i++) positions[i] = i;
        return client.createInitialBitfield(positions, n);
    }

    function capacityProofs(BeefyClient client, bytes32[][] memory levels, BeefyClient.Commitment memory c,
        uint256[] memory bits, bool interactive, bool repeated)
        internal returns (BeefyClient.ValidatorProof[] memory proofs)
    {
        bytes32 digest = client.computeCommitmentHash(c);
        uint256[] memory selected;
        if (interactive) {
            client.submitInitial(c, bits, capacityProof(levels, 0, digest, repeated));
            vm.roll(block.number + 128);
            vm.prevrandao(bytes32(uint256(1)));
            client.commitPrevRandao(digest);
            selected = client.createFinalBitfield(digest, bits);
        } else selected = client.createFiatShamirFinalBitfield(c, bits);
        uint256 n = levels[0].length;
        uint256 count;
        for (uint256 i; i < n; i++) if ((selected[0] & (uint256(1) << i)) != 0) count++;
        proofs = new BeefyClient.ValidatorProof[](count);
        count = 0;
        for (uint256 i; i < n; i++) {
            if ((selected[0] & (uint256(1) << i)) != 0)
                proofs[count++] = capacityProof(levels, i, digest, repeated);
        }
    }

    function testCapacityAllSizesBothAcceptancePaths() public {
        uint256[6] memory sizes = [uint256(2), 3, 4, 59, 150, 256];
        for (uint256 size; size < sizes.length; size++) {
            bytes32[][] memory levels = authorityTree(sizes[size], false);
            for (uint256 mode; mode < 2; mode++) {
                BeefyClient client = capacityClient(levels);
                (BeefyClient.Commitment memory c, BeefyClient.MMRLeaf memory leaf,
                    VaraBridgeMetadata.Snapshot memory s) = capacityWitness(client, levels, 0);
                uint256[] memory bits = capacityBits(client, sizes[size], sizes[size] - (sizes[size] - 1) / 3);
                BeefyClient.ValidatorProof[] memory proofs = capacityProofs(client, levels, c, bits, mode == 1, false);
                assertEq(proofs.length, sizes[size] <= 3 ? sizes[size] : sizes[size] / 3 + 1);
                finishUpdate(mode == 1, client, c, bits, proofs, leaf, s, new bytes32[](0), 0);
                assertEq(client.latestMMRRoot(), keccak256(leafBytes(leaf)));
                assertEq(client.latestBeefyBlock(), 102);
            }
        }
    }

    function testCapacity256BoundaryAndCounterPacking() public {
        bytes32[][] memory levels = authorityTree(256, false);
        BeefyClient client = capacityClient(levels);
        (BeefyClient.Commitment memory c,,) = capacityWitness(client, levels, 0);
        uint256[] memory bits = capacityBits(client, 256, 256);
        assertEq(bits.length, 1);
        assertEq(bits[0], type(uint256).max);
        uint256[3] memory positions = [uint256(15), 16, 255];
        for (uint256 i; i < positions.length; i++) {
            client.submitInitial(c, bits, capacityProof(levels, positions[i], client.computeCommitmentHash(c), false));
        }
        (,,, Uint16Array memory counters) = client.currentValidatorSet();
        assertEq(counters.length, 256);
        assertEq(counters.data.length, 16);
        for (uint256 i; i < positions.length; i++)
            assertEq((counters.data[positions[i] / 16] >> (16 * (positions[i] % 16))) & 0xffff, 1);
        assertEq(client.minNumRequiredSignatures(), 86);
        assertEq(client.fiatShamirRequiredSignatures(), 86);
        assertEq(client.MAX_VALIDATORS(), 256);
        assertEq(client.randaoCommitDelay(), 128);
        assertEq(client.randaoCommitExpiration(), 24);
    }

    function testCapacity59And256RejectQuorumPaddingWidthsAndDuplicates() public {
        uint256[2] memory sizes = [uint256(59), 256];
        for (uint256 size; size < sizes.length; size++) {
            uint256 n = sizes[size];
            uint256 quorum = n - (n - 1) / 3;
            assertEq(quorum, n == 256 ? 171 : 40);
            bytes32[][] memory levels = authorityTree(n, false);
            BeefyClient client = capacityClient(levels);
            (BeefyClient.Commitment memory c, BeefyClient.MMRLeaf memory leaf,
                VaraBridgeMetadata.Snapshot memory s) = capacityWitness(client, levels, 0);
            uint256[] memory bits = capacityBits(client, n, quorum - 1);
            BeefyClient.ValidatorProof memory initial = capacityProof(levels, 0, client.computeCommitmentHash(c), false);
            vm.expectRevert(BeefyClient.InvalidBitfield.selector);
            client.createFiatShamirFinalBitfield(c, bits);
            vm.expectRevert(BeefyClient.InvalidBitfield.selector);
            client.submitInitial(c, bits, initial);
            vm.expectRevert(BeefyClient.InvalidBitfield.selector);
            client.submitFiatShamir(c, bits, new BeefyClient.ValidatorProof[](0), leaf, s, new bytes32[](0), 0);
            for (uint256 mutation; mutation < 3; mutation++) {
                bits = capacityBits(client, n, n);
                if (mutation == 0 && n < 256) bits[0] |= uint256(1) << n;
                else if (mutation == 1) bits = new uint256[](0);
                else {
                    uint256[] memory longBits = new uint256[](2);
                    longBits[0] = bits[0];
                    longBits[1] = 1;
                    bits = longBits;
                }
                vm.expectRevert(); client.createFiatShamirFinalBitfield(c, bits);
                vm.expectRevert(); client.submitInitial(c, bits, initial);
                vm.expectRevert(); client.submitFiatShamir(c, bits, new BeefyClient.ValidatorProof[](0), leaf, s, new bytes32[](0), 0);
            }
            bits = capacityBits(client, n, quorum);
            for (uint256 mode; mode < 2; mode++) {
                BeefyClient.ValidatorProof[] memory proofs = capacityProofs(client, levels, c, bits, mode == 1, false);
                assertEq(proofs.length, n / 3 + 1);
                proofs[1] = proofs[0];
                vm.expectRevert(BeefyClient.InvalidValidatorProof.selector);
                finishUpdate(mode == 1, client, c, bits, proofs, leaf, s, new bytes32[](0), 0);
            }
            uint256[] memory positions = new uint256[](2);
            positions[0] = n - 1; positions[1] = n - 1;
            vm.expectRevert(); client.createInitialBitfield(positions, n);
            positions[1] = n;
            vm.expectRevert(); client.createInitialBitfield(positions, n);
            vm.expectRevert(); client.createInitialBitfield(new uint256[](0), 257);
            assertEq(client.latestMMRRoot(), bytes32(0));
        }
    }

    function testRepeatedAuthoritiesAtDistinctPositionsBothPaths() public {
        bytes32[][] memory levels = authorityTree(4, true);
        for (uint256 mode; mode < 2; mode++) {
            BeefyClient client = capacityClient(levels);
            (BeefyClient.Commitment memory c, BeefyClient.MMRLeaf memory leaf,
                VaraBridgeMetadata.Snapshot memory s) = capacityWitness(client, levels, 0);
            uint256[] memory bits = capacityBits(client, 4, 4);
            BeefyClient.ValidatorProof[] memory proofs = capacityProofs(client, levels, c, bits, mode == 1, true);
            assertNotEq(proofs[0].index, proofs[1].index);
            vm.expectRevert(BeefyClient.InvalidValidatorProof.selector);
            if (mode == 0) client.submitFiatShamir(c, bits, proofs, leaf, s, new bytes32[](0), 0);
            else client.submitFinal(c, bits, proofs, leaf, s, new bytes32[](0), 0);
        }
    }
}
