pragma solidity ^0.8.13;

import {Test} from "forge-std/Test.sol";
import {ERC20Treasury} from "../src/ERC20Treasury.sol";
import {ERC20TreasuryBridgingPayment} from "../src/ERC20Treasury.sol";
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

    uint256 constant TOKEN_TRANSFER_AMOUNT = 100;

    ERC20TreasuryBridgingPayment public bridging_payment;
    ERC20Treasury public erc20_treasury;
    ERC20Mock public erc20_mock;

    function setUp() public {
        vm.startPrank(DEPLOYER, DEPLOYER);

        erc20_mock = new ERC20Mock("Mock");

        bool transferred = erc20_mock.transfer(USER, TOKEN_TRANSFER_AMOUNT);
        assertEq(transferred, true);

        ERC20Treasury erc20_treasury_impl = new ERC20Treasury(address(0));
        ProxyContract _erc20_treasury = new ProxyContract();
        _erc20_treasury.upgradeToAndCall(address(erc20_treasury_impl), "");
        erc20_treasury = ERC20Treasury(address(_erc20_treasury));

        bridging_payment = new ERC20TreasuryBridgingPayment(
            address(erc20_treasury),
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

        bridging_payment.deposit{value: FEE}(
            address(erc20_mock),
            TOKEN_TRANSFER_AMOUNT,
            bytes32(0)
        );

        assertEq(
            erc20_mock.balanceOf(address(erc20_treasury)),
            TOKEN_TRANSFER_AMOUNT
        );
        assertEq(ADMIN.balance, FEE);
    }

    function test_revertWhenNotEnoughFee() public {
        vm.startPrank(USER, USER);
        vm.deal(USER, USER_BALANCE);

        approveTransfer();

        vm.expectRevert(BridgingPayment.NotEnoughFunds.selector);
        bridging_payment.deposit{value: NOT_ENOUGH_FEE}(
            address(erc20_mock),
            TOKEN_TRANSFER_AMOUNT,
            bytes32(0)
        );
    }

    function approveTransfer() public {
        bool approved = erc20_mock.approve(
            address(erc20_treasury),
            TOKEN_TRANSFER_AMOUNT
        );
        assertEq(approved, true);
    }
}
