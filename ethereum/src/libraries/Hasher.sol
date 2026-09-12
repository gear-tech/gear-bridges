// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {VaraMessage} from "src/interfaces/IMessageQueue.sol";

/**
 * @dev Library for hashing VaraMessage.
 */
library Hasher {
    /// forge-lint: disable-next-item(internal-function-used-once)
    /**
     * @dev Hashes VaraMessage.
     * @param message Message to hash.
     * @return hash Hash of the message.
     */
    function hashCalldata(VaraMessage calldata message) internal pure returns (bytes32) {
        // forge-lint: disable-next-item(asm-keccak256)
        return keccak256(abi.encodePacked(message.nonce, message.source, message.destination, message.payload));
    }

    /**
     * @dev Hashes VaraMessage.
     * @param message Message to hash.
     * @return hash Hash of the message.
     */
    function hash(VaraMessage memory message) internal pure returns (bytes32) {
        // forge-lint: disable-next-item(asm-keccak256)
        return keccak256(abi.encodePacked(message.nonce, message.source, message.destination, message.payload));
    }
}
