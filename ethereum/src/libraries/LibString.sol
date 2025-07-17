// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

/**
 * @dev Library for converting numbers into strings and other string operations.
 * @author Solady (https://github.com/vectorized/solady/blob/main/src/utils/LibString.sol)
 */
library LibString {
    /**
     * @dev Packs a single string with its length into a single word.
     * @return result Returns `bytes32(0)` if the length is zero or greater than 31.
     */
    function packOne(string memory a) internal pure returns (bytes32 result) {
        assembly ("memory-safe") {
            // We don't need to zero right pad the string,
            // since this is our own custom non-standard packing scheme.
            result :=
                mul(
                    // Load the length and the bytes.
                    mload(add(a, 0x1f)),
                    // `length != 0 && length < 32`. Abuses underflow.
                    // Assumes that the length is valid and within the block gas limit.
                    lt(sub(mload(a), 1), 0x1f)
                )
        }
    }

    /**
     * @dev Unpacks a string packed using {packOne}.
     * @return result Returns the empty string if `packed` is `bytes32(0)`.
     * @dev If `packed` is not an output of {packOne}, the output behavior is undefined.
     */
    function unpackOne(bytes32 packed) internal pure returns (string memory result) {
        assembly ("memory-safe") {
            result := mload(0x40) // Grab the free memory pointer.
            mstore(0x40, add(result, 0x40)) // Allocate 2 words (1 for the length, 1 for the bytes).
            mstore(result, 0) // Zeroize the length slot.
            mstore(add(result, 0x1f), packed) // Store the length and bytes.
            mstore(add(add(result, 0x20), mload(result)), 0) // Right pad with zeroes.
        }
    }
}
