// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import {Test} from "forge-std/Test.sol";
import {ITetherToken} from "src/erc20/interfaces/ITetherToken.sol";
import {IBridgingPayment} from "src/interfaces/IBridgingPayment.sol";
import {IERC20Mintable} from "src/interfaces/IERC20Mintable.sol";
import {Base} from "test/Base.sol";
import {BridgingPaymentOwner} from "test/BridgingPaymentOwner.sol";

contract BridgingPaymentTest is Test, Base {
    function setUp() public {
        deployBridgeDependsOnEnvironment();
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

        if (isFork()) {
            bool isMainnet = block.chainid == 1;
            if (isMainnet) {
                ITetherToken(address(tetherToken)).issue(amount);
                ITetherToken(address(tetherToken)).approve(address(erc20Manager), amount);
            } else {
                IERC20Mintable(address(tetherToken)).mint(deploymentArguments.deployerAddress, amount);
                bool success = tetherToken.approve(address(erc20Manager), amount);
                assertTrue(success);
            }
        } else {
            IERC20Mintable(address(tetherToken)).mint(deploymentArguments.deployerAddress, amount);
            bool success = tetherToken.approve(address(erc20Manager), amount);
            assertTrue(success);
        }

        BridgingPaymentOwner bridgingPaymentOwner = new BridgingPaymentOwner(erc20Manager);
        address bridgingPayment_ = bridgingPaymentOwner.createBridgingPayment(deploymentArguments.bridgingPaymentFee);

        vm.expectRevert(IBridgingPayment.PayFeeFailed.selector);
        // forge-lint: disable-next-item(arbitrary-send-eth)
        erc20Manager.requestBridgingPayingFee{value: deploymentArguments.bridgingPaymentFee}(
            token, amount, to, bridgingPayment_
        );

        vm.stopPrank();
    }
}
