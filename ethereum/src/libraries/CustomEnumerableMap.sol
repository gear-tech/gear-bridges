// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

import {EnumerableMap} from "@openzeppelin/contracts/utils/structs/EnumerableMap.sol";
import {EnumerableSet} from "@openzeppelin/contracts/utils/structs/EnumerableSet.sol";
import {IERC20Manager} from "src/interfaces/IERC20Manager.sol";

library CustomEnumerableMap {
    using EnumerableSet for EnumerableSet.Bytes32Set;

    // AddressToTokenTypeMap

    struct AddressToTokenTypeMap {
        EnumerableMap.Bytes32ToBytes32Map _inner;
    }

    /**
     * @dev Adds a key-value pair to a map, or updates the value for an existing
     * key. O(1).
     *
     * Returns true if the key was added to the map, that is if it was not
     * already present.
     */
    function set(AddressToTokenTypeMap storage map, address key, IERC20Manager.TokenType value)
        internal
        returns (bool)
    {
        return EnumerableMap.set(map._inner, bytes32(uint256(uint160(key))), bytes32(uint256(value)));
    }

    /**
     * @dev Removes a value from a map. O(1).
     *
     * Returns true if the key was removed from the map, that is if it was present.
     */
    function remove(AddressToTokenTypeMap storage map, address key) internal returns (bool) {
        return EnumerableMap.remove(map._inner, bytes32(uint256(uint160(key))));
    }

    /**
     * @dev Removes all the entries from a map. O(n).
     *
     * WARNING: Developers should keep in mind that this function has an unbounded cost and using it may render the
     * function uncallable if the map grows to the point where clearing it consumes too much gas to fit in a block.
     */
    function clear(AddressToTokenTypeMap storage map) internal {
        EnumerableMap.clear(map._inner);
    }

    /**
     * @dev Returns true if the key is in the map. O(1).
     */
    function contains(AddressToTokenTypeMap storage map, address key) internal view returns (bool) {
        return EnumerableMap.contains(map._inner, bytes32(uint256(uint160(key))));
    }

    /**
     * @dev Returns the number of elements in the map. O(1).
     */
    function length(AddressToTokenTypeMap storage map) internal view returns (uint256) {
        return EnumerableMap.length(map._inner);
    }

    /**
     * @dev Returns the element stored at position `index` in the map. O(1).
     * Note that there are no guarantees on the ordering of values inside the
     * array, and it may change when more values are added or removed.
     *
     * Requirements:
     *
     * - `index` must be strictly less than {length}.
     */
    function at(AddressToTokenTypeMap storage map, uint256 index)
        internal
        view
        returns (address key, IERC20Manager.TokenType value)
    {
        (bytes32 atKey, bytes32 val) = EnumerableMap.at(map._inner, index);
        return (address(uint160(uint256(atKey))), IERC20Manager.TokenType(uint256(val)));
    }

    /**
     * @dev Tries to returns the value associated with `key`. O(1).
     * Does not revert if `key` is not in the map.
     */
    function tryGet(AddressToTokenTypeMap storage map, address key)
        internal
        view
        returns (bool exists, IERC20Manager.TokenType value)
    {
        (bool success, bytes32 val) = EnumerableMap.tryGet(map._inner, bytes32(uint256(uint160(key))));
        return (success, IERC20Manager.TokenType(uint256(val)));
    }

    /**
     * @dev Returns the value associated with `key`. O(1).
     *
     * Requirements:
     *
     * - `key` must be in the map.
     */
    function get(AddressToTokenTypeMap storage map, address key) internal view returns (IERC20Manager.TokenType) {
        return IERC20Manager.TokenType(uint256(EnumerableMap.get(map._inner, bytes32(uint256(uint160(key))))));
    }

    /**
     * @dev Return the an array containing all the keys
     *
     * WARNING: This operation will copy the entire storage to memory, which can be quite expensive. This is designed
     * to mostly be used by view accessors that are queried without any gas fees. Developers should keep in mind that
     * this function has an unbounded cost, and using it as part of a state-changing function may render the function
     * uncallable if the map grows to a point where copying to memory consumes too much gas to fit in a block.
     */
    function keys(AddressToTokenTypeMap storage map) internal view returns (address[] memory) {
        bytes32[] memory store = EnumerableMap.keys(map._inner);
        address[] memory result;

        assembly ("memory-safe") {
            result := store
        }

        return result;
    }
}
