// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {IERC20Manager} from "src/interfaces/IERC20Manager.sol";
import {CustomEnumerableMap} from "src/libraries/CustomEnumerableMap.sol";

contract CustomEnumerableMapWrapper {
    using CustomEnumerableMap for CustomEnumerableMap.AddressToTokenTypeMap;

    CustomEnumerableMap.AddressToTokenTypeMap private map;

    function set(address key, IERC20Manager.TokenType value) external {
        map.set(key, value);
    }

    function remove(address key) external {
        map.remove(key);
    }

    function clear() external {
        map.clear();
    }

    function contains(address key) external view returns (bool) {
        return map.contains(key);
    }

    function length() external view returns (uint256) {
        return map.length();
    }

    function pos(uint256 index) external view returns (address key, IERC20Manager.TokenType value) {
        return map.pos(index);
    }

    function tryGet(address key) external view returns (bool exists, IERC20Manager.TokenType value) {
        return map.tryGet(key);
    }

    function get(address key) external view returns (IERC20Manager.TokenType) {
        return map.get(key);
    }

    function keys() external view returns (address[] memory) {
        return map.keys();
    }
}
