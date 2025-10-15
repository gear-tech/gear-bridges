// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test} from "forge-std/Test.sol";
import {Base} from "./Base.sol";
import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import {IBridgingPayment} from "src/interfaces/IBridgingPayment.sol";
import {IERC20Mintable} from "src/interfaces/IERC20Mintable.sol";
import {ERC20Manager} from "src/ERC20Manager.sol";

contract BridgingPaymentOwner {
    ERC20Manager public erc20Manager;

    constructor(ERC20Manager _erc20Manager) {
        erc20Manager = _erc20Manager;
    }

    function createBridgingPayment(uint256 fee) external returns (address) {
        return erc20Manager.createBridgingPayment(fee);
    }
}

contract BridgingPaymentTest is Test, Base {
    function setUp() public {
        deployBridgeFromConstants();
    }

    function test_SetFee() public {
        vm.startPrank(deploymentArguments.deployerAddress);

        bridgingPayment.setFee(2 wei);
        assertEq(bridgingPayment.fee(), 2 wei);

        vm.stopPrank();
    }

    function test_SetFeeUnauthorized() public {
        vm.startPrank(address(0x11));

        vm.expectRevert(abi.encodeWithSelector(OwnableUpgradeable.OwnableUnauthorizedAccount.selector, address(0x11)));
        bridgingPayment.setFee(2 wei);

        vm.stopPrank();
    }

    function test_PayFeeUnauthorized() public {
        vm.startPrank(address(0x11));

        vm.expectRevert(IBridgingPayment.OnlyErc20Manager.selector);
        bridgingPayment.payFee();

        vm.stopPrank();
    }

    function test_PayFeeWithInvalidOwner() public {
        vm.startPrank(deploymentArguments.deployerAddress);

        address token = address(tetherToken);
        uint256 amount = 100 * (10 ** tetherToken.decimals());
        bytes32 to = 0;

        IERC20Mintable(address(tetherToken)).mint(deploymentArguments.deployerAddress, amount);
        tetherToken.approve(address(erc20Manager), amount);

        BridgingPaymentOwner bridgingPaymentOwner = new BridgingPaymentOwner(erc20Manager);
        address bridgingPayment_ = bridgingPaymentOwner.createBridgingPayment(deploymentArguments.bridgingPaymentFee);

        vm.expectRevert(IBridgingPayment.PayFeeFailed.selector);
        erc20Manager.requestBridgingPayingFee{
            value: deploymentArguments.bridgingPaymentFee
        }(token, amount, to, bridgingPayment_);

        vm.stopPrank();
    }
}
