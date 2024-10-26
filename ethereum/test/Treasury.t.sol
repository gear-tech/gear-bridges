pragma solidity ^0.8.20;

import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {Test, console} from "forge-std/Test.sol";
import {Verifier} from "../src/Verifier.sol";
import {Relayer} from "../src/Relayer.sol";

import {ERC20Treasury} from "../src/ERC20Treasury.sol";
import {IERC20Treasury} from "../src/interfaces/IERC20Treasury.sol";

import {IMessageQueue, VaraMessage, IMessageQueueReceiver} from "../src/interfaces/IMessageQueue.sol";
import {MessageQueue} from "../src/MessageQueue.sol";
import {ProxyContract} from "../src/ProxyContract.sol";

import {ERC20Mock} from "../src/mocks/ERC20Mock.sol";

import {TestHelper, VARA_ADDRESS_3, VARA_ADDRESS_7, USER, OWNER} from "./TestHelper.t.sol";
import {VFT_GATEWAY_ADDRESS} from "../src/libraries/Environment.sol";

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
        erc20_token.approve(address(treasury), amount);
        treasury.deposit(address(erc20_token), amount, VARA_ADDRESS_3);

        vm.stopPrank();
    }

    // TODO: Test skipped, to enable it remove the skip_ prefix
    function skip_test_withdraw() public {
        uint128 amount = 100 * (10 ** 18);
        erc20_token.approve(address(treasury), amount);
        treasury.deposit(address(erc20_token), amount, VARA_ADDRESS_3);

        bytes memory call_data = abi.encodePacked(
            address(this),
            address(erc20_token),
            amount
        );
        console.log(amount);
        console.logBytes(call_data);

        vm.expectRevert();

        IMessageQueueReceiver(treasury).processVaraMessage(
            VFT_GATEWAY_ADDRESS,
            call_data
        );

        vm.prank(address(message_queue));
        IMessageQueueReceiver(treasury).processVaraMessage(VFT_GATEWAY_ADDRESS, call_data);
    }
}
