pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";

import {IERC20Manager} from "../src/interfaces/IERC20Manager.sol";
import {ERC20Manager} from "../src/ERC20Manager.sol";

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {ERC20Mock} from "../src/mocks/ERC20Mock.sol";
import {ERC20GearSupply} from "../src/erc20/ERC20GearSupply.sol";

contract ERC20ManagerTest is Test {
    address constant MESSAGE_QUEUE = address(500);
    address constant FAKE_MESSAGE_QUEUE = address(501);
    bytes32 constant VFT_MANAGER = bytes32("vft_manager_vft_manager_vft_mana");
    bytes32 constant FAKE_VFT_MANAGER =
        bytes32("fake_vft_manager_vft_manager_vft");

    address constant ETH_TOKEN_HOLDER = address(6969);
    bytes32 constant GEAR_TOKEN_RECEIVER =
        bytes32("token_receiver_token_receiver_to");
    address constant ETH_TOKEN_RECEIVER = address(1234);
    uint256 constant TRANSFER_AMOUNT = 1000;

    ERC20Manager public erc20Manager;

    ERC20GearSupply public erc20GearSupply;
    ERC20Mock public erc20EthSupply;

    function setUp() public {
        erc20Manager = new ERC20Manager(MESSAGE_QUEUE, VFT_MANAGER);

        erc20GearSupply = new ERC20GearSupply(address(erc20Manager), "", "");

        vm.startPrank(ETH_TOKEN_HOLDER, ETH_TOKEN_HOLDER);
        erc20EthSupply = new ERC20Mock("");
        vm.stopPrank();
    }

    function test_ethereumSupply() public {
        // Lock ethereum-supply tokens.

        assertEq(
            uint8(erc20Manager.getTokenSupplyType(address(erc20EthSupply))),
            uint8(IERC20Manager.SupplyType.Unknown)
        );

        vm.startPrank(ETH_TOKEN_HOLDER, ETH_TOKEN_HOLDER);
        erc20EthSupply.approve(address(erc20Manager), TRANSFER_AMOUNT);
        erc20Manager.requestBridging(
            address(erc20EthSupply),
            TRANSFER_AMOUNT,
            GEAR_TOKEN_RECEIVER
        );
        vm.stopPrank();

        assertEq(
            uint8(erc20Manager.getTokenSupplyType(address(erc20EthSupply))),
            uint8(IERC20Manager.SupplyType.Ethereum)
        );

        assertEq(
            erc20EthSupply.balanceOf(address(erc20Manager)),
            TRANSFER_AMOUNT
        );

        // Unlock ethereum-supply tokens.

        vm.startPrank(MESSAGE_QUEUE, MESSAGE_QUEUE);
        erc20Manager.processVaraMessage(
            VFT_MANAGER,
            abi.encodePacked(
                ETH_TOKEN_RECEIVER,
                address(erc20EthSupply),
                TRANSFER_AMOUNT
            )
        );
        vm.stopPrank();

        assertEq(erc20EthSupply.balanceOf(address(erc20Manager)), 0);
        assertEq(erc20EthSupply.balanceOf(ETH_TOKEN_RECEIVER), TRANSFER_AMOUNT);
    }

    function test_gearSupply() public {
        // Mint gear-supply tokens.

        assertEq(
            uint8(erc20Manager.getTokenSupplyType(address(erc20GearSupply))),
            uint8(IERC20Manager.SupplyType.Unknown)
        );

        vm.startPrank(MESSAGE_QUEUE, MESSAGE_QUEUE);
        erc20Manager.processVaraMessage(
            VFT_MANAGER,
            abi.encodePacked(
                ETH_TOKEN_RECEIVER,
                address(erc20GearSupply),
                TRANSFER_AMOUNT
            )
        );
        vm.stopPrank();

        assertEq(
            uint8(erc20Manager.getTokenSupplyType(address(erc20GearSupply))),
            uint8(IERC20Manager.SupplyType.Gear)
        );

        assertEq(erc20GearSupply.balanceOf(address(erc20Manager)), 0);
        assertEq(
            erc20GearSupply.balanceOf(ETH_TOKEN_RECEIVER),
            TRANSFER_AMOUNT
        );

        // Send gear-supply tokens back to gear.

        vm.startPrank(ETH_TOKEN_RECEIVER, ETH_TOKEN_RECEIVER);
        erc20GearSupply.approve(address(erc20Manager), TRANSFER_AMOUNT);
        erc20Manager.requestBridging(
            address(erc20GearSupply),
            TRANSFER_AMOUNT,
            GEAR_TOKEN_RECEIVER
        );
        vm.stopPrank();

        assertEq(erc20GearSupply.balanceOf(ETH_TOKEN_RECEIVER), 0);
        assertEq(erc20GearSupply.balanceOf(address(erc20Manager)), 0);
    }

    function test_unauthorizedFails() public {
        vm.startPrank(MESSAGE_QUEUE, MESSAGE_QUEUE);
        vm.expectRevert(IERC20Manager.BadVftManagerAddress.selector);
        erc20Manager.processVaraMessage(
            FAKE_VFT_MANAGER,
            abi.encodePacked(
                ETH_TOKEN_RECEIVER,
                address(erc20GearSupply),
                TRANSFER_AMOUNT
            )
        );
        vm.stopPrank();

        vm.startPrank(FAKE_MESSAGE_QUEUE, FAKE_MESSAGE_QUEUE);
        vm.expectRevert(IERC20Manager.NotAuthorized.selector);
        erc20Manager.processVaraMessage(
            VFT_MANAGER,
            abi.encodePacked(
                ETH_TOKEN_RECEIVER,
                address(erc20GearSupply),
                TRANSFER_AMOUNT
            )
        );
        vm.stopPrank();
    }
}
