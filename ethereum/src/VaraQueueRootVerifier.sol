// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {ScaleCodec} from "src/beefy/utils/ScaleCodec.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";

/// Authenticates Vara queue snapshots against the client's latest BEEFY root only.
contract VaraQueueRootVerifier is IVerifier {
    BeefyClient public immutable beefyClient;

    constructor(BeefyClient client_) {
        require(address(client_).code.length != 0, "BEEFY client has no code");
        beefyClient = client_;
    }

    function safeVerifyProof(bytes calldata proof, uint256[] calldata publicInputs) external view returns (bool) {
        try this.verifyProof(proof, publicInputs) returns (bool valid) {
            return valid;
        } catch {
            return false;
        }
    }

    function verifyProof(bytes calldata proof, uint256[] calldata publicInputs) external view returns (bool) {
        if (proof.length < 480 || proof.length > 8672 || publicInputs.length != 2) return false;
        if (publicInputs[0] >> 192 != 0 || publicInputs[1] >> 192 != 0 || uint96(publicInputs[1]) != 0) return false;
        (
            uint8 proofVersion,
            uint8 bridgeVersion,
            uint64 queueId,
            uint64 anchorBlock,
            bytes32 anchorRoot,
            BeefyClient.MMRLeaf memory leaf,
            bytes32[] memory items,
            uint256 proofOrder
        ) = abi.decode(proof, (uint8, uint8, uint64, uint64, bytes32, BeefyClient.MMRLeaf, bytes32[], uint256));
        if (items.length > 256 || (items.length < 256 && proofOrder >> items.length != 0)) return false;
        if (
            keccak256(proof)
                != keccak256(
                    abi.encode(proofVersion, bridgeVersion, queueId, anchorBlock, anchorRoot, leaf, items, proofOrder)
                )
        ) return false;
        bytes32 root = bytes32((publicInputs[0] << 64) | (publicInputs[1] >> 128));
        uint32 sourceBlock = uint32(publicInputs[1] >> 96);
        if (proofVersion != 0 || bridgeVersion != 0 || leaf.version != 0 || root == bytes32(0)) return false;
        if (leaf.parentNumber != sourceBlock || sourceBlock >= anchorBlock || anchorRoot == bytes32(0)) return false;
        if (anchorBlock != beefyClient.latestBeefyBlock() || anchorRoot != beefyClient.latestMMRRoot()) return false;
        if (
            keccak256(bytes.concat(bytes1(bridgeVersion), bytes4("vara"), ScaleCodec.encodeU64(queueId), root))
                != leaf.parachainHeadsRoot
        ) return false;
        bytes32 leafHash = keccak256(
            bytes.concat(
                ScaleCodec.encodeU8(leaf.version),
                ScaleCodec.encodeU32(leaf.parentNumber),
                leaf.parentHash,
                ScaleCodec.encodeU64(leaf.nextAuthoritySetID),
                ScaleCodec.encodeU32(leaf.nextAuthoritySetLen),
                leaf.nextAuthoritySetRoot,
                leaf.parachainHeadsRoot
            )
        );
        return beefyClient.verifyMMRLeafProof(leafHash, items, proofOrder);
    }
}
