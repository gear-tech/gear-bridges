// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test} from "forge-std/Test.sol";

import {EthereumToken} from "../src/erc20/EthereumToken.sol";

contract EthereumTokenTest is Test {
    address constant USER = address(1234);
    address constant FAKE_USER = address(4321);
    uint256 constant TOKEN_AMOUNT = 1000;
    uint256 constant INITIAL_ETHER_BALANCE = 1 ether;

    EthereumToken public token;

    function setUp() public {
        vm.deal(USER, INITIAL_ETHER_BALANCE);

        token = new EthereumToken("", "");
    }

    function test_tokenizeAndRelease() public {
        vm.startPrank(USER);

        token.tokenize{value: TOKEN_AMOUNT}();
        assertEq(token.balanceOf(USER), TOKEN_AMOUNT);
        assertEq(USER.balance, INITIAL_ETHER_BALANCE - TOKEN_AMOUNT);
        assertEq(address(token).balance, TOKEN_AMOUNT);

        token.release(TOKEN_AMOUNT);
        assertEq(token.balanceOf(USER), 0);
        assertEq(USER.balance, INITIAL_ETHER_BALANCE);
        assertEq(address(token).balance, 0);

        vm.stopPrank();
    }

    function test_releaseFailsIfUserDontHaveSufficientBalance() public {
        vm.startPrank(USER);
        token.tokenize{value: TOKEN_AMOUNT}();
        vm.stopPrank();

        vm.startPrank(FAKE_USER);
        vm.expectRevert();
        token.release(TOKEN_AMOUNT);
        vm.stopPrank();
    }
}
