pragma solidity ^0.8.13;

import {Test, console} from "forge-std/Test.sol";
import {ERC20Treasury} from "../src/ERC20Treasury.sol";
import {IERC20Treasury} from "../src/interfaces/IERC20Treasury.sol";
import {MessageQueue} from "../src/MessageQueue.sol";
import {ProxyContract} from "../src/ProxyContract.sol";

contract ProxyTest is Test {
    ProxyContract public treasury_proxy;
    ProxyContract public message_queue_proxy;

    function setUp() public {
        message_queue_proxy = new ProxyContract();
        treasury_proxy = new ProxyContract();

        ERC20Treasury treasury = new ERC20Treasury(
            address(message_queue_proxy)
        );
        MessageQueue message_queue = new MessageQueue(address(treasury_proxy));

        message_queue_proxy.upgradeToAndCall(address(message_queue), "");
        treasury_proxy.upgradeToAndCall(address(treasury), "");
    }

    function test_renewImplementation() public {
        ERC20Treasury new_treasury = new ERC20Treasury(
            address(message_queue_proxy)
        );

        // from pranker
        vm.prank(
            address(0x5124fcC2B3F99F571AD67D075643C743F38f1C34),
            address(0x5124fcC2B3F99F571AD67D075643C743F38f1C34)
        );
        vm.expectRevert(ProxyContract.ProxyDeniedAdminAccess.selector);
        treasury_proxy.upgradeToAndCall(address(new_treasury), bytes(""));

        // from proxyAdmin no init
        treasury_proxy.upgradeToAndCall(address(new_treasury), bytes(""));
        assertEq(treasury_proxy.implementation(), address(new_treasury));
    }

    function test_changeProxyAdmin() public {
        address not_admin = address(0x5124fcC2B3F99F571AD67D075643C743F38f1C34);

        // from pranker
        vm.prank(not_admin, not_admin);
        vm.expectRevert(ProxyContract.ProxyDeniedAdminAccess.selector);
        treasury_proxy.changeProxyAdmin(not_admin);

        // from proxyAdmin
        treasury_proxy.changeProxyAdmin(not_admin);
        assertEq(treasury_proxy.proxyAdmin(), not_admin);
    }
}
