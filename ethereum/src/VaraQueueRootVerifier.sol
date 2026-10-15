// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {VaraBridgeMetadata} from "src/beefy/VaraBridgeMetadata.sol";
import {ScaleCodec} from "src/beefy/utils/ScaleCodec.sol";
import {IQueueProgressVerifier} from "src/interfaces/IQueueProgressVerifier.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";

/// Authenticates Vara queue snapshots against the client's latest BEEFY root.
contract VaraQueueRootVerifier is IVerifier, IQueueProgressVerifier {
    BeefyClient public immutable beefyClient;
    address public immutable messageQueue;
    uint256 public immutable destinationChainId;

    constructor(BeefyClient client_, address messageQueue_, uint256 destinationChainId_) {
        require(address(client_).code.length != 0, "BEEFY client has no code");
        require(
            client_.minNumRequiredSignatures() == 86 && client_.fiatShamirRequiredSignatures() == 86
                && client_.MAX_VALIDATORS() == 256 && client_.randaoCommitDelay() == 128
                && client_.randaoCommitExpiration() == 24,
            "client policy mismatch"
        );
        require(messageQueue_ != address(0), "message queue is zero");
        require(destinationChainId_ == block.chainid, "wrong deployment chain");
        require(client_.destinationChainId() == destinationChainId_, "client chain mismatch");
        require(client_.destinationQueue() == messageQueue_, "client queue mismatch");
        require(
            client_.bridgeDomain() == _bridgeDomain(client_.sourceDomain(), destinationChainId_, messageQueue_),
            "client bridge domain mismatch"
        );
        beefyClient = client_;
        messageQueue = messageQueue_;
        destinationChainId = destinationChainId_;
    }

    function safeVerifyProof(bytes calldata proof, uint256[] calldata publicInputs) external view returns (bool) {
        if (msg.sender != messageQueue || block.chainid != destinationChainId) {
            return false;
        }
        try this.verifyProof(proof, publicInputs) returns (bool valid) {
            return valid;
        } catch {
            return false;
        }
    }

    function safeVerifyEmptyQueueProgress(uint256 sourceBlock, bytes calldata proof) external view returns (bool) {
        if (msg.sender != messageQueue || block.chainid != destinationChainId) {
            return false;
        }
        try this.verifyEmptyQueueProgress(sourceBlock, proof) returns (bool valid) {
            return valid;
        } catch {
            return false;
        }
    }

    function verifyEmptyQueueProgress(uint256 sourceBlock, bytes calldata proof) external view returns (bool) {
        if (msg.sender != address(this) || sourceBlock > type(uint32).max) {
            return false;
        }
        return _verifyProof(proof, 0, sourceBlock << 96, true);
    }

    function verifyProof(bytes calldata proof, uint256[] calldata publicInputs) external view returns (bool) {
        if (msg.sender != address(this) || publicInputs.length != 2) {
            return false;
        }
        return _verifyProof(proof, publicInputs[0], publicInputs[1], false);
    }

    function _verifyProof(bytes calldata proof, uint256 input0, uint256 input1, bool emptyProgress)
        private
        view
        returns (bool)
    {
        if (input0 >> 192 != 0 || input1 >> 192 != 0 || uint96(input1) != 0) {
            return false;
        }
        if (proof.length < 576 || proof.length > 576 + 32 * 256 || (proof.length - 576) % 32 != 0) {
            return false;
        }

        (
            uint8 proofVersion,
            uint8 bridgeVersion,
            bool initialized,
            bytes32 bridgeDomain,
            uint64 sourceTimestampMs,
            uint64 queueId,
            uint64 anchorBlock,
            bytes32 anchorRoot,
            BeefyClient.MMRLeaf memory leaf,
            bytes32[] memory items,
            uint256 proofOrder
        ) = abi.decode(
            proof,
            (uint8, uint8, bool, bytes32, uint64, uint64, uint64, bytes32, BeefyClient.MMRLeaf, bytes32[], uint256)
        );
        if (items.length > 256 || (items.length < 256 && proofOrder >> items.length != 0)) {
            return false;
        }
        if (proof.length != 576 + 32 * items.length) {
            return false;
        }
        if (
            keccak256(proof)
                != keccak256(
                    abi.encode(
                        proofVersion,
                        bridgeVersion,
                        initialized,
                        bridgeDomain,
                        sourceTimestampMs,
                        queueId,
                        anchorBlock,
                        anchorRoot,
                        leaf,
                        items,
                        proofOrder
                    )
                )
        ) {
            return false;
        }

        if (!beefyClient.isLive()) {
            return false;
        }
        bytes32 acceptedRoot = beefyClient.latestMMRRoot();
        uint64 acceptedBlock = beefyClient.latestBeefyBlock();
        if (acceptedRoot == bytes32(0) || acceptedBlock == 0) {
            return false;
        }

        bytes32 root = bytes32((input0 << 64) | (input1 >> 128));
        uint32 sourceBlock = uint32(input1 >> 96);
        if ((emptyProgress ? root != bytes32(0) : root == bytes32(0)) || !initialized) {
            return false;
        }
        VaraBridgeMetadata.Snapshot memory snapshot =
            _snapshot(bridgeDomain, sourceTimestampMs, initialized, queueId, root);
        if (
            proofVersion != 2 || bridgeVersion != 2 || bridgeDomain != beefyClient.bridgeDomain()
                || bridgeDomain != _bridgeDomain(beefyClient.sourceDomain(), destinationChainId, messageQueue)
                || snapshot.queueRoot != root
        ) {
            return false;
        }
        if (leaf.version != 0 || leaf.parentNumber != sourceBlock) {
            return false;
        }
        if (uint64(sourceBlock) < beefyClient.mmrStartBlock() || sourceBlock >= anchorBlock) {
            return false;
        }
        if (anchorBlock != acceptedBlock || anchorRoot != acceptedRoot || anchorRoot == bytes32(0)) {
            return false;
        }
        if (VaraBridgeMetadata.hash(snapshot) != leaf.parachainHeadsRoot) {
            return false;
        }

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

    function _bridgeDomain(bytes32 sourceDomain, uint256 chainId, address queue) private pure returns (bytes32) {
        return keccak256(abi.encodePacked("vara/gear-eth-bridge-domain/v2", sourceDomain, bytes32(chainId), queue));
    }

    function _snapshot(bytes32 bridgeDomain, uint64 sourceTimestampMs, bool initialized, uint64 queueId, bytes32 root)
        private
        pure
        returns (VaraBridgeMetadata.Snapshot memory snapshot)
    {
        snapshot = VaraBridgeMetadata.Snapshot({
            version: 2,
            bridgeDomain: bridgeDomain,
            sourceTimestampMs: sourceTimestampMs,
            initialized: initialized,
            queueId: queueId,
            queueRoot: root
        });
    }
}
