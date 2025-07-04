// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test} from "forge-std/Test.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {ProxyContract} from "../src/ProxyContract.sol";
import {ProxyUpdater} from "../src/ProxyUpdater.sol";

contract Empty {}

contract ProxyUpdaterTest is Test {
    address constant MESSAGE_QUEUE = address(500);
    address constant NEW_ADMIN = address(1000);

    address initialImpl;
    address changedImpl;

    bytes32 constant GOVERNANCE = bytes32("governance_governance_governance");
    bytes32 constant NEW_GOVERNANCE = bytes32("new_governance_governance_govern");

    ProxyContract public proxy;
    ProxyUpdater public updater;

    function setUp() public {
        initialImpl = address(new Empty());
        changedImpl = address(new Empty());

        proxy = new ProxyContract();
        proxy.upgradeToAndCall(initialImpl, "");

        updater = new ProxyUpdater(proxy, GOVERNANCE, MESSAGE_QUEUE);

        proxy.changeProxyAdmin(address(updater));
    }

    function test_updateImpl() public {
        vm.startPrank(MESSAGE_QUEUE);

        assertEq(
            address(uint160(uint256(vm.load(address(proxy), ERC1967Utils.IMPLEMENTATION_SLOT)))), address(initialImpl)
        );

        updater.processVaraMessage(GOVERNANCE, abi.encodePacked(uint8(0), changedImpl, ""));

        assertEq(
            address(uint160(uint256(vm.load(address(proxy), ERC1967Utils.IMPLEMENTATION_SLOT)))), address(changedImpl)
        );
    }

    function test_updateAdmin() public {
        vm.startPrank(MESSAGE_QUEUE);

        assertEq(address(uint160(uint256(vm.load(address(proxy), ERC1967Utils.ADMIN_SLOT)))), address(updater));

        updater.processVaraMessage(GOVERNANCE, abi.encodePacked(uint8(1), NEW_ADMIN));

        assertEq(address(uint160(uint256(vm.load(address(proxy), ERC1967Utils.ADMIN_SLOT)))), NEW_ADMIN);
    }

    function test_updateGovernance() public {
        vm.startPrank(MESSAGE_QUEUE);

        assertEq(updater.getGovernance(), GOVERNANCE);

        updater.processVaraMessage(GOVERNANCE, abi.encodePacked(uint8(2), NEW_GOVERNANCE));

        assertEq(updater.getGovernance(), NEW_GOVERNANCE);
    }
}
