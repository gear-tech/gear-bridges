// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test} from "forge-std/Test.sol";
import {WrappedEther} from "../src/erc20/WrappedEther.sol";
import {WrappedVara} from "../src/erc20/WrappedVara.sol";
import {IERC20Manager} from "../src/interfaces/IERC20Manager.sol";
import {ERC20Manager} from "../src/ERC20Manager.sol";

contract ERC20ManagerTest is Test {
    address constant MESSAGE_QUEUE = address(500);
    address constant FAKE_MESSAGE_QUEUE = address(501);
    bytes32 constant VFT_MANAGER = bytes32("vft_manager_vft_manager_vft_mana");
    bytes32 constant FAKE_VFT_MANAGER = bytes32("fake_vft_manager_vft_manager_vft");

    address constant ETH_TOKEN_HOLDER = address(6969);
    bytes32 constant GEAR_TOKEN_RECEIVER = bytes32("token_receiver_token_receiver_to");
    address constant ETH_TOKEN_RECEIVER = address(1234);
    bytes32 constant VARA_ADDRESS = bytes32(uint256(7856));
    uint256 constant TRANSFER_AMOUNT = 1000;

    ERC20Manager public erc20Manager;

    WrappedVara public wrappedVara;
    WrappedEther public wrappedEther;

    function setUp() public {
        erc20Manager = new ERC20Manager(MESSAGE_QUEUE, VFT_MANAGER);

        wrappedVara = new WrappedVara(address(erc20Manager));

        vm.startPrank(ETH_TOKEN_HOLDER, ETH_TOKEN_HOLDER);
        vm.deal(ETH_TOKEN_HOLDER, type(uint256).max);
        wrappedEther = new WrappedEther();
        wrappedEther.deposit{value: type(uint256).max}();
        vm.stopPrank();
    }

    function test_ethereumSupply() public {
        // Lock ethereum-supply tokens.

        assertEq(uint8(erc20Manager.getTokenSupplyType(address(wrappedEther))), uint8(IERC20Manager.SupplyType.Unknown));

        vm.startPrank(ETH_TOKEN_HOLDER, ETH_TOKEN_HOLDER);
        wrappedEther.approve(address(erc20Manager), TRANSFER_AMOUNT);
        erc20Manager.requestBridging(address(wrappedEther), TRANSFER_AMOUNT, GEAR_TOKEN_RECEIVER);
        vm.stopPrank();

        assertEq(
            uint8(erc20Manager.getTokenSupplyType(address(wrappedEther))), uint8(IERC20Manager.SupplyType.Ethereum)
        );

        assertEq(wrappedEther.balanceOf(address(erc20Manager)), TRANSFER_AMOUNT);

        // Unlock ethereum-supply tokens.

        vm.startPrank(MESSAGE_QUEUE, MESSAGE_QUEUE);
        erc20Manager.processVaraMessage(
            VFT_MANAGER, abi.encodePacked(ETH_TOKEN_RECEIVER, address(wrappedEther), TRANSFER_AMOUNT, VARA_ADDRESS)
        );
        vm.stopPrank();

        assertEq(wrappedEther.balanceOf(address(erc20Manager)), 0);
        assertEq(wrappedEther.balanceOf(ETH_TOKEN_RECEIVER), TRANSFER_AMOUNT);
    }

    function test_gearSupply() public {
        // Mint gear-supply tokens.

        assertEq(uint8(erc20Manager.getTokenSupplyType(address(wrappedVara))), uint8(IERC20Manager.SupplyType.Unknown));

        vm.startPrank(MESSAGE_QUEUE, MESSAGE_QUEUE);
        erc20Manager.processVaraMessage(
            VFT_MANAGER, abi.encodePacked(ETH_TOKEN_RECEIVER, address(wrappedVara), TRANSFER_AMOUNT, VARA_ADDRESS)
        );
        vm.stopPrank();

        assertEq(uint8(erc20Manager.getTokenSupplyType(address(wrappedVara))), uint8(IERC20Manager.SupplyType.Gear));

        assertEq(wrappedVara.balanceOf(address(erc20Manager)), 0);
        assertEq(wrappedVara.balanceOf(ETH_TOKEN_RECEIVER), TRANSFER_AMOUNT);

        // Send gear-supply tokens back to gear.

        vm.startPrank(ETH_TOKEN_RECEIVER, ETH_TOKEN_RECEIVER);
        wrappedVara.approve(address(erc20Manager), TRANSFER_AMOUNT);
        erc20Manager.requestBridging(address(wrappedVara), TRANSFER_AMOUNT, GEAR_TOKEN_RECEIVER);
        vm.stopPrank();

        assertEq(wrappedVara.balanceOf(ETH_TOKEN_RECEIVER), 0);
        assertEq(wrappedVara.balanceOf(address(erc20Manager)), 0);
    }

    function test_unauthorizedFails() public {
        vm.startPrank(MESSAGE_QUEUE, MESSAGE_QUEUE);
        vm.expectRevert(IERC20Manager.BadSender.selector);
        erc20Manager.processVaraMessage(
            FAKE_VFT_MANAGER, abi.encodePacked(ETH_TOKEN_RECEIVER, address(wrappedVara), TRANSFER_AMOUNT, VARA_ADDRESS)
        );
        vm.stopPrank();

        vm.startPrank(FAKE_MESSAGE_QUEUE, FAKE_MESSAGE_QUEUE);
        vm.expectRevert(IERC20Manager.NotAuthorized.selector);
        erc20Manager.processVaraMessage(
            VFT_MANAGER, abi.encodePacked(ETH_TOKEN_RECEIVER, address(wrappedVara), TRANSFER_AMOUNT, VARA_ADDRESS)
        );
        vm.stopPrank();
    }
}
