pragma solidity ^0.8.13;

import {Test} from "forge-std/Test.sol";
import {ERC20Manager} from "../src/ERC20Manager.sol";
import {ERC20ManagerBridgingPayment} from "../src/ERC20Manager.sol";
import {BridgingPayment} from "../src/BridgingPayment.sol";
import {ERC20Mock} from "../src/mocks/ERC20Mock.sol";
import {ProxyContract} from "../src/ProxyContract.sol";

contract BridgingPaymentTest is Test {
    uint256 constant NOT_ENOUGH_FEE = 99;
    uint256 constant FEE = 100;
    uint256 constant USER_BALANCE = 10 ether;

    address constant ADMIN = address(42);
    address constant DEPLOYER = address(911);
    address constant USER = address(69);

    bytes32 constant VFT_MANAGER =
        bytes32(
            0x0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A
        );

    uint256 constant TOKEN_TRANSFER_AMOUNT = 100;

    ERC20ManagerBridgingPayment public bridging_payment;
    ERC20Manager public erc20_manager;
    ERC20Mock public erc20_mock;

    function setUp() public {
        vm.startPrank(DEPLOYER, DEPLOYER);

        erc20_mock = new ERC20Mock("Mock");

        bool transferred = erc20_mock.transfer(USER, TOKEN_TRANSFER_AMOUNT);
        assertEq(transferred, true);

        ERC20Manager erc20_manager_impl = new ERC20Manager(
            address(0),
            VFT_MANAGER
        );
        ProxyContract _erc20_manager = new ProxyContract();
        _erc20_manager.upgradeToAndCall(address(erc20_manager_impl), "");
        erc20_manager = ERC20Manager(address(_erc20_manager));

        bridging_payment = new ERC20ManagerBridgingPayment(
            address(erc20_manager),
            ADMIN,
            FEE
        );
    }

    function test_feeDeducted() public {
        vm.startPrank(USER, USER);
        vm.deal(USER, USER_BALANCE);
        vm.deal(ADMIN, 0);

        approveTransfer();

        vm.expectEmit(address(bridging_payment));
        emit BridgingPayment.FeePaid();

        bridging_payment.requestBridging{value: FEE}(
            address(erc20_mock),
            TOKEN_TRANSFER_AMOUNT,
            bytes32(0)
        );

        assertEq(
            erc20_mock.balanceOf(address(erc20_manager)),
            TOKEN_TRANSFER_AMOUNT
        );
        assertEq(ADMIN.balance, FEE);
    }

    function test_revertWhenNotEnoughFee() public {
        vm.startPrank(USER, USER);
        vm.deal(USER, USER_BALANCE);

        approveTransfer();

        vm.expectRevert();
        bridging_payment.requestBridging{value: NOT_ENOUGH_FEE}(
            address(erc20_mock),
            TOKEN_TRANSFER_AMOUNT,
            bytes32(0)
        );
    }

    function approveTransfer() public {
        bool approved = erc20_mock.approve(
            address(bridging_payment),
            TOKEN_TRANSFER_AMOUNT
        );
        assertEq(approved, true);
    }
}
