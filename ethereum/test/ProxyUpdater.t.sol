pragma solidity ^0.8.13;

import {Test} from "forge-std/Test.sol";
import {ProxyContract} from "../src/ProxyContract.sol";
import {ProxyUpdater} from "../src/ProxyUpdater.sol";

contract Empty {}

contract ProxyUpdaterTest is Test {
    address constant MESSAGE_QUEUE = address(500);
    address constant NEW_ADMIN = address(1000);

    address initialImpl;
    address changedImpl;

    bytes32 constant GOVERNANCE = bytes32("governance_governance_governance");
    bytes32 constant NEW_GOVERNANCE =
        bytes32("new_governance_governance_govern");

    ProxyContract public proxy;
    ProxyUpdater public updater;

    function setUp() public {
        initialImpl = address(new Empty());
        changedImpl = address(new Empty());

        proxy = new ProxyContract();
        proxy.upgradeToAndCall(initialImpl, "");

        updater = new ProxyUpdater(
            payable(address(proxy)),
            GOVERNANCE,
            MESSAGE_QUEUE
        );

        proxy.changeProxyAdmin(address(updater));
    }

    function test_updateImpl() public {
        vm.startPrank(MESSAGE_QUEUE);

        assertEq(proxy.implementation(), initialImpl);

        updater.processVaraMessage(
            GOVERNANCE,
            abi.encodePacked(uint8(0), changedImpl, "")
        );

        assertEq(proxy.implementation(), changedImpl);
    }

    function test_updateAdmin() public {
        vm.startPrank(MESSAGE_QUEUE);

        assertEq(proxy.proxyAdmin(), address(updater));

        updater.processVaraMessage(
            GOVERNANCE,
            abi.encodePacked(uint8(1), NEW_ADMIN)
        );

        assertEq(proxy.proxyAdmin(), NEW_ADMIN);
    }

    function test_updateGovernance() public {
        vm.startPrank(MESSAGE_QUEUE);

        assertEq(updater.getGovernance(), GOVERNANCE);

        updater.processVaraMessage(
            GOVERNANCE,
            abi.encodePacked(uint8(2), NEW_GOVERNANCE)
        );

        assertEq(updater.getGovernance(), NEW_GOVERNANCE);
    }
}
