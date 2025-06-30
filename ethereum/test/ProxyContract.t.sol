// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test, console} from "forge-std/Test.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {IRelayer} from "../src/interfaces/IRelayer.sol";
import {ERC20Manager} from "../src/ERC20Manager.sol";
import {IERC20Manager} from "../src/interfaces/IERC20Manager.sol";
import {MessageQueue} from "../src/MessageQueue.sol";
import {ProxyContract} from "../src/ProxyContract.sol";
import {VFT_MANAGER_ADDRESS} from "./TestHelper.t.sol";

contract ProxyTest is Test {
    ProxyContract public erc20_manager_proxy;
    ProxyContract public message_queue_proxy;

    function setUp() public {
        message_queue_proxy = new ProxyContract();
        erc20_manager_proxy = new ProxyContract();

        ERC20Manager erc20_manager = new ERC20Manager(address(message_queue_proxy), VFT_MANAGER_ADDRESS);
        MessageQueue message_queue = new MessageQueue(IRelayer(address(erc20_manager_proxy)));

        message_queue_proxy.upgradeToAndCall(address(message_queue), "");
        erc20_manager_proxy.upgradeToAndCall(address(erc20_manager), "");
    }

    function test_renewImplementation() public {
        ERC20Manager new_erc20_manager = new ERC20Manager(address(message_queue_proxy), VFT_MANAGER_ADDRESS);

        // from pranker
        vm.prank(
            address(0x5124fcC2B3F99F571AD67D075643C743F38f1C34), address(0x5124fcC2B3F99F571AD67D075643C743F38f1C34)
        );
        vm.expectRevert(ProxyContract.ProxyDeniedAdminAccess.selector);
        erc20_manager_proxy.upgradeToAndCall(address(new_erc20_manager), bytes(""));

        // from proxyAdmin no init
        erc20_manager_proxy.upgradeToAndCall(address(new_erc20_manager), bytes(""));
        assertEq(
            address(uint160(uint256(vm.load(address(erc20_manager_proxy), ERC1967Utils.IMPLEMENTATION_SLOT)))),
            address(new_erc20_manager)
        );
    }

    function test_changeProxyAdmin() public {
        address not_admin = address(0x5124fcC2B3F99F571AD67D075643C743F38f1C34);

        // from pranker
        vm.prank(not_admin, not_admin);
        vm.expectRevert(ProxyContract.ProxyDeniedAdminAccess.selector);
        erc20_manager_proxy.changeProxyAdmin(not_admin);

        // from proxyAdmin
        erc20_manager_proxy.changeProxyAdmin(not_admin);
        assertEq(address(uint160(uint256(vm.load(address(erc20_manager_proxy), ERC1967Utils.ADMIN_SLOT)))), not_admin);
    }
}
