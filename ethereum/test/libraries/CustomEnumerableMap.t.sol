// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {EnumerableMap} from "@openzeppelin/contracts/utils/structs/EnumerableMap.sol";
import {Test} from "forge-std/Test.sol";
import {IERC20Manager} from "src/interfaces/IERC20Manager.sol";
import {CustomEnumerableMapWrapper} from "test/libraries/wrappers/CustomEnumerableMapWrapper.sol";

contract CustomEnumerableMapTest is Test {
    CustomEnumerableMapWrapper public customEnumerableMapWrapper;

    function setUp() public {
        customEnumerableMapWrapper = new CustomEnumerableMapWrapper();
    }

    function test_Complex() public {
        customEnumerableMapWrapper.set(address(0x11), IERC20Manager.TokenType.Ethereum);
        customEnumerableMapWrapper.set(address(0x22), IERC20Manager.TokenType.Ethereum);
        customEnumerableMapWrapper.set(address(0x33), IERC20Manager.TokenType.Gear);

        assertFalse(customEnumerableMapWrapper.contains(address(0)));

        assertTrue(customEnumerableMapWrapper.contains(address(0x11)));
        assertTrue(customEnumerableMapWrapper.contains(address(0x22)));
        assertTrue(customEnumerableMapWrapper.contains(address(0x33)));

        uint256 length = customEnumerableMapWrapper.length();
        assertEq(length, 3);

        vm.expectRevert();
        (address key, IERC20Manager.TokenType value) = customEnumerableMapWrapper.pos(length);

        (key, value) = customEnumerableMapWrapper.pos(0);
        assertEq(key, address(0x11));
        assertTrue(value == IERC20Manager.TokenType.Ethereum);

        (key, value) = customEnumerableMapWrapper.pos(1);
        assertEq(key, address(0x22));
        assertTrue(value == IERC20Manager.TokenType.Ethereum);

        (key, value) = customEnumerableMapWrapper.pos(2);
        assertEq(key, address(0x33));
        assertTrue(value == IERC20Manager.TokenType.Gear);

        bool exists;
        (exists, value) = customEnumerableMapWrapper.tryGet(address(0));
        assertFalse(exists);
        assertTrue(value == IERC20Manager.TokenType.Unknown);

        (exists, value) = customEnumerableMapWrapper.tryGet(address(0x11));
        assertTrue(exists);
        assertTrue(value == IERC20Manager.TokenType.Ethereum);

        (exists, value) = customEnumerableMapWrapper.tryGet(address(0x22));
        assertTrue(exists);
        assertTrue(value == IERC20Manager.TokenType.Ethereum);

        (exists, value) = customEnumerableMapWrapper.tryGet(address(0x33));
        assertTrue(exists);
        assertTrue(value == IERC20Manager.TokenType.Gear);

        vm.expectRevert(abi.encodeWithSelector(EnumerableMap.EnumerableMapNonexistentKey.selector, address(0)));
        // forge-lint: disable-next-item(unused-return)
        customEnumerableMapWrapper.get(address(0));

        assertTrue(customEnumerableMapWrapper.get(address(0x11)) == IERC20Manager.TokenType.Ethereum);
        assertTrue(customEnumerableMapWrapper.get(address(0x22)) == IERC20Manager.TokenType.Ethereum);
        assertTrue(customEnumerableMapWrapper.get(address(0x33)) == IERC20Manager.TokenType.Gear);

        address[] memory keys = customEnumerableMapWrapper.keys();
        assertEq(keys.length, 3);
        assertEq(keys[0], address(0x11));
        assertEq(keys[1], address(0x22));
        assertEq(keys[2], address(0x33));

        customEnumerableMapWrapper.remove(address(0x11));

        assertFalse(customEnumerableMapWrapper.contains(address(0)));

        assertFalse(customEnumerableMapWrapper.contains(address(0x11)));
        assertTrue(customEnumerableMapWrapper.contains(address(0x22)));
        assertTrue(customEnumerableMapWrapper.contains(address(0x33)));

        length = customEnumerableMapWrapper.length();
        assertEq(length, 2);

        vm.expectRevert();
        (key, value) = customEnumerableMapWrapper.pos(length);

        (key, value) = customEnumerableMapWrapper.pos(0);
        assertEq(key, address(0x33));
        assertTrue(value == IERC20Manager.TokenType.Gear);

        (key, value) = customEnumerableMapWrapper.pos(1);
        assertEq(key, address(0x22));
        assertTrue(value == IERC20Manager.TokenType.Ethereum);

        customEnumerableMapWrapper.clear();

        length = customEnumerableMapWrapper.length();
        assertEq(length, 0);
    }
}
