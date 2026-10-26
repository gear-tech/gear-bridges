// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {VaraBridgeMetadata} from "src/beefy/VaraBridgeMetadata.sol";
import {BeefyFixtureTest} from "test/BeefyInterop.t.sol";

contract BeefySubmissionParityTest is BeefyFixtureTest {
    function setUp() public {
        fixtures = vm.readFile("test/fixtures/beefy-interop.json");
    }

    // VB-13 reconstructed from HTML, audited 7fe1b032. Both real signature protocols,
    // not a bootstrap/mocked root; execution evidence is recorded by integration.
    function testVB13ReconstructedAcceptedAnchorRingBothProtocols() public {
        // Fixture parsing/signing across 130 updates is not a per-transaction gas measurement.
        vm.pauseGasMetering();
        uint256 index = caseIndex("synthetic-mmr-3-primary");
        for (uint256 mode; mode < 2; mode++) {
            BeefyClient client = newClient(index);
            uint64 bootstrap = client.latestBeefyBlock();
            assertFalse(client.isAcceptedAnchor(bootstrap, bytes32(0)));
            assertFalse(client.isAcceptedAnchor(bootstrap, fixtureHash(index, "mmrRoot")));
            assertFalse(client.verifyMMRLeafProof(bootstrap, bytes32(0), bytes32(0), new bytes32[](0), 0));
            (, VaraBridgeMetadata.Snapshot memory s,,) = freshnessWitness(index);
            vm.warp(uint256(s.sourceTimestampMs) / 1000);
            bytes32[65] memory roots;
            uint256[] memory bits = allSigners(index);
            for (uint32 i; i < 65; i++) {
                uint32 height = uint32(bootstrap) + i + 1;
                BeefyClient.MMRLeaf memory leaf =
                    leafForSnapshot(s, height - 1, 1, 3, fixtureHash(index, "authorityRoot"));
                roots[i] = keccak256(leafBytes(leaf));
                BeefyClient.Commitment memory c = commitment(index, height, 0, roots[i]);
                BeefyClient.ValidatorProof[] memory proofs = _proofs(mode == 1, client, c, index, bits);
                finishUpdate(mode == 1, client, c, bits, proofs, leaf, s, new bytes32[](0), 0);
                assertTrue(client.isAcceptedAnchor(height, roots[i]));
                assertTrue(client.verifyMMRLeafProof(height, roots[i], roots[i], new bytes32[](0), 0));
                assertFalse(client.isAcceptedAnchor(height, roots[i] ^ bytes32(uint256(1))));
                assertFalse(
                    client.verifyMMRLeafProof(height, roots[i] ^ bytes32(uint256(1)), roots[i], new bytes32[](0), 0)
                );
                if (i < 64) {
                    assertTrue(client.isAcceptedAnchor(bootstrap + 1, roots[0]));
                    assertTrue(client.verifyMMRLeafProof(bootstrap + 1, roots[0], roots[0], new bytes32[](0), 0));
                }
                // Same-height aliases cannot replace the accepted pair or consume a ring slot.
                c.payload[0].data = abi.encodePacked(roots[i] ^ bytes32(uint256(1)));
                vm.expectRevert(BeefyClient.StaleCommitment.selector);
                client.submitFiatShamir(c, bits, proofs, leaf, s, new bytes32[](0), 0);
                assertTrue(client.isAcceptedAnchor(height, roots[i]));
            }
            assertFalse(client.isAcceptedAnchor(bootstrap + 1, roots[0]));
            assertFalse(client.verifyMMRLeafProof(bootstrap + 1, roots[0], roots[0], new bytes32[](0), 0));
            for (uint64 i = 1; i < 65; i++) {
                assertTrue(client.isAcceptedAnchor(bootstrap + i + 1, roots[i]));
            }
            assertFalse(client.isAcceptedAnchor(bootstrap + 65, roots[63]));
            vm.warp(uint256(s.sourceTimestampMs) / 1000 + 86_401);
            assertTrue(client.isAcceptedAnchor(bootstrap + 65, roots[64]));
            vm.expectRevert(BeefyClient.ClientExpired.selector);
            client.verifyMMRLeafProof(bootstrap + 65, roots[64], roots[64], new bytes32[](0), 0);
        }
        vm.resumeGasMetering();
    }

    function testBothProtocolsRotateSetsEmitAndPreserveFailedTickets() public {
        uint256 index = caseIndex("synthetic-mmr-3-primary");
        for (uint256 mode; mode < 2; mode++) {
            bool interactive = mode == 1;
            BeefyClient client = newClient(index);
            (, VaraBridgeMetadata.Snapshot memory s,,) = freshnessWitness(index);
            vm.warp(uint256(s.sourceTimestampMs) / 1000);
            BeefyClient.MMRLeaf memory leaf = leafForSnapshot(s, 102, 2, 3, fixtureHash(index, "authorityRoot"));
            bytes32 root = keccak256(leafBytes(leaf));
            BeefyClient.Commitment memory c = commitment(index, 103, 1, root);
            uint256[] memory bits = allSigners(index);
            BeefyClient.ValidatorProof[] memory proofs = _proofs(interactive, client, c, index, bits);
            bytes32 digest = client.computeCommitmentHash(c);
            bytes32 ticketId = keccak256(abi.encode(address(this), digest));
            bytes32 beforeState = stateHash(client);

            leaf.parentHash ^= bytes32(uint256(1));
            vm.expectRevert(BeefyClient.InvalidMMRLeafProof.selector);
            finishUpdate(interactive, client, c, bits, proofs, leaf, s, new bytes32[](0), 0);
            assertEq(stateHash(client), beforeState);
            (uint64 ticketBlock,,,,) = client.tickets(ticketId);
            if (interactive) assertGt(ticketBlock, 0);
            else assertEq(ticketBlock, 0);

            leaf.parentHash ^= bytes32(uint256(1));
            vm.expectEmit(false, false, false, true, address(client));
            emit BeefyClient.NewMMRRoot(root, 103);
            finishUpdate(interactive, client, c, bits, proofs, leaf, s, new bytes32[](0), 0);
            (uint128 currentId, uint128 currentLength, bytes32 currentRoot) = client.currentValidatorSet();
            (uint128 nextId, uint128 nextLength, bytes32 nextRoot) = client.nextValidatorSet();
            assertEq(currentId, 1);
            assertEq(currentLength, 3);
            assertEq(currentRoot, fixtureHash(index, "authorityRoot"));
            assertEq(nextId, 2);
            assertEq(nextLength, 3);
            assertEq(nextRoot, leaf.nextAuthoritySetRoot);
            assertEq(client.latestMMRRoot(), root);
            assertEq(client.latestBeefyBlock(), 103);
            assertEq(client.lastAuthenticatedSourceTimestampMs(), s.sourceTimestampMs);
            (ticketBlock,,,,) = client.tickets(ticketId);
            assertEq(ticketBlock, 0);
        }
    }

    function testSignatureRevertsAndProtocolSpecificProofOrderingStayExact() public {
        uint256 index = caseIndex("synthetic-mmr-3-primary");
        for (uint256 mode; mode < 2; mode++) {
            bool interactive = mode == 1;
            BeefyClient client = newClient(index);
            (
                BeefyClient.MMRLeaf memory leaf,
                VaraBridgeMetadata.Snapshot memory s,
                bytes32[] memory items,
                uint256 order
            ) = freshnessWitness(index);
            vm.warp(uint256(s.sourceTimestampMs) / 1000);
            BeefyClient.Commitment memory c = commitment(index, 102, 0, fixtureHash(index, "mmrRoot"));
            uint256[] memory bits = allSigners(index);
            BeefyClient.ValidatorProof[] memory proofs = _proofs(interactive, client, c, index, bits);
            bytes32 beforeState = stateHash(client);
            for (uint256 mutation; mutation < 3; mutation++) {
                uint8 originalV = proofs[0].v;
                bytes32 originalR = proofs[0].r;
                bytes32 originalS = proofs[0].s;
                if (mutation == 0) {
                    (proofs[0].v, proofs[0].r, proofs[0].s) = vm.sign(99, client.computeCommitmentHash(c));
                    vm.expectRevert(BeefyClient.InvalidSignature.selector);
                } else if (mutation == 1) {
                    proofs[0].v = 0;
                    vm.expectRevert(ECDSA.ECDSAInvalidSignature.selector);
                } else {
                    proofs[0].index = 256;
                    // Fiat-Shamir checks the bitfield first; interactive checks membership first.
                    if (interactive) vm.expectRevert(BeefyClient.InvalidValidatorProof.selector);
                    else vm.expectRevert(abi.encodeWithSignature("Panic(uint256)", 0x32));
                }
                finishUpdate(interactive, client, c, bits, proofs, leaf, s, items, order);
                assertEq(stateHash(client), beforeState);
                proofs[0].v = originalV;
                proofs[0].r = originalR;
                proofs[0].s = originalS;
            }
        }
    }

    function _proofs(
        bool interactive,
        BeefyClient client,
        BeefyClient.Commitment memory c,
        uint256 index,
        uint256[] memory bits
    ) internal returns (BeefyClient.ValidatorProof[] memory) {
        if (!interactive) return signedProofs(client, c, index, "");
        bytes32 digest = client.computeCommitmentHash(c);
        BeefyClient.ValidatorProof memory initial = validatorProof(index, 0);
        (initial.v, initial.r, initial.s) = vm.sign(1, digest);
        client.submitInitial(c, bits, initial);
        vm.roll(block.number + client.randaoCommitDelay());
        vm.prevrandao(bytes32(uint256(12345)));
        client.commitPrevRandao(digest);
        uint256[] memory selection = client.createFinalBitfield(digest, bits);
        return selectedProofs(client, c, index, selection[0], "");
    }
}
