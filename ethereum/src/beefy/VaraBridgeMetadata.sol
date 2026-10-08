// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.37;

import {ScaleCodec} from "./utils/ScaleCodec.sol";

library VaraBridgeMetadata {
    error InvalidBridgeMetadata();

    struct Snapshot {
        uint8 version;
        bytes32 bridgeDomain;
        uint64 sourceTimestampMs;
        bool initialized;
        uint64 queueId;
        bytes32 queueRoot;
    }

    uint8 internal constant VERSION = 2;
    bytes4 internal constant MAGIC = bytes4("vara");

    function hash(Snapshot memory snapshot) internal pure returns (bytes32) {
        if (
            snapshot.version != VERSION
                || (!snapshot.initialized && (snapshot.queueId != 0 || snapshot.queueRoot != bytes32(0)))
        ) {
            revert InvalidBridgeMetadata();
        }

        return keccak256(
            bytes.concat(
                ScaleCodec.encodeU8(snapshot.version),
                MAGIC,
                snapshot.bridgeDomain,
                ScaleCodec.encodeU64(snapshot.sourceTimestampMs),
                ScaleCodec.encodeU8(snapshot.initialized ? 1 : 0),
                ScaleCodec.encodeU64(snapshot.queueId),
                snapshot.queueRoot
            )
        );
    }
}
