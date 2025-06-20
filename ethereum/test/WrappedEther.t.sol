// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test} from "forge-std/Test.sol";
import {WrappedEther} from "../src/erc20/WrappedEther.sol";

contract WrappedEtherTest is Test {
    address constant USER = address(1234);
    address constant FAKE_USER = address(4321);
    uint256 constant ETHER_VALUE = 0.1 ether;
    uint256 constant INITIAL_ETHER_BALANCE = 1 ether;

    WrappedEther public wrappedEther;

    function setUp() public {
        vm.deal(USER, INITIAL_ETHER_BALANCE);
        wrappedEther = new WrappedEther();
    }

    function test_DepositAndWithdraw() public {
        vm.startPrank(USER);

        wrappedEther.deposit{value: ETHER_VALUE}();
        assertEq(wrappedEther.balanceOf(USER), ETHER_VALUE);
        assertEq(USER.balance, INITIAL_ETHER_BALANCE - ETHER_VALUE);
        assertEq(address(wrappedEther).balance, ETHER_VALUE);

        wrappedEther.withdraw(ETHER_VALUE);
        assertEq(wrappedEther.balanceOf(USER), 0);
        assertEq(USER.balance, INITIAL_ETHER_BALANCE);
        assertEq(address(wrappedEther).balance, 0);

        vm.stopPrank();
    }

    function test_WithdrawFailsIfUserDontHaveSufficientBalance() public {
        vm.startPrank(USER);
        wrappedEther.deposit{value: ETHER_VALUE}();
        vm.stopPrank();

        vm.startPrank(FAKE_USER);
        vm.expectRevert();
        wrappedEther.withdraw(ETHER_VALUE);
        vm.stopPrank();
    }
}
