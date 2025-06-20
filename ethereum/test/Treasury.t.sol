// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {Test, console} from "forge-std/Test.sol";
import {Verifier} from "../src/Verifier.sol";
import {Relayer} from "../src/Relayer.sol";

import {ERC20Manager} from "../src/ERC20Manager.sol";
import {IERC20Manager} from "../src/interfaces/IERC20Manager.sol";

import {VaraMessage, IMessageQueue} from "../src/interfaces/IMessageQueue.sol";
import {IMessageQueueReceiver} from "../src/interfaces/IMessageQueueReceiver.sol";
import {MessageQueue} from "../src/MessageQueue.sol";
import {ProxyContract} from "../src/ProxyContract.sol";

import {TestHelper, VARA_ADDRESS_3, VARA_ADDRESS_7, USER, OWNER, VFT_MANAGER_ADDRESS} from "./TestHelper.t.sol";

contract TreasuryTest is TestHelper {
    using Address for address;

    function setUp() public override {
        super.setUp();
        vm.prank(OWNER, OWNER);
        erc20_token.transfer(USER, 100 * (10 ** 18));
    }

    function test_deposit() public {
        vm.startPrank(USER, USER);

        uint256 amount = 100 * (10 ** 18);
        erc20_token.approve(address(erc20_manager), amount);
        erc20_manager.requestBridging(address(erc20_token), amount, VARA_ADDRESS_3);

        vm.stopPrank();
    }

    function test_withdraw() public {
        uint256 amount = 100 * (10 ** 18);
        erc20_token.approve(address(erc20_manager), amount);
        erc20_manager.requestBridging(address(erc20_token), amount, VARA_ADDRESS_3);

        bytes memory call_data = abi.encodePacked(address(this), address(erc20_token), amount, VARA_ADDRESS_3);
        vm.expectRevert();
        IMessageQueueReceiver(erc20_manager).processVaraMessage(VFT_MANAGER_ADDRESS, call_data);

        vm.prank(address(message_queue));
        IMessageQueueReceiver(erc20_manager).processVaraMessage(VFT_MANAGER_ADDRESS, call_data);
    }
}
