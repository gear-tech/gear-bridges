pragma solidity ^0.8.13;

import {Test, console} from "forge-std/Test.sol";
import {Treasury} from "../src/Treasury.sol";
import {ITreasury} from "../src/interfaces/ITreasury.sol";
import {MessageQueue} from "../src/MessageQueue.sol";
import {ProxyContract} from "../src/ProxyContract.sol";


contract ProxyTest is Test {
    //Treasury public treasury;
    //MessageQueue public message_queue;
    ProxyContract public treasury_proxy;
    ProxyContract public message_queue_proxy;

    function setUp() public  {
        Treasury treasury = new Treasury();
        MessageQueue message_queue = new MessageQueue();
        message_queue_proxy = new ProxyContract( address(message_queue), bytes("") ); 
        treasury_proxy = new ProxyContract(address(treasury), bytes("")  );
    }


    function test_setUp() public {
        assertNotEq(address(message_queue_proxy), address(0) );
        assertNotEq(address(treasury_proxy), address(0) );
    }

    function test_renewImplementation() public {
        Treasury new_treasury = new Treasury();


        // from pranker
        vm.prank(address(0x5124fcC2B3F99F571AD67D075643C743F38f1C34), address(0x5124fcC2B3F99F571AD67D075643C743F38f1C34) );
        vm.expectRevert(ProxyContract.ProxyDeniedAdminAccess.selector);
        treasury_proxy.upgradeToAndCall(address(new_treasury), bytes(""));

        // from proxyAdmin no init
        treasury_proxy.upgradeToAndCall(address(new_treasury), bytes(""));
        assertEq(treasury_proxy.implementation(), address(new_treasury));
    }

    function test_changeProxyAdmin() public {
        address not_admin = address(0x5124fcC2B3F99F571AD67D075643C743F38f1C34);

        // from pranker
        vm.prank(not_admin, not_admin );
        vm.expectRevert(ProxyContract.ProxyDeniedAdminAccess.selector);
        treasury_proxy.changeProxyAdmin(not_admin);

        // from proxyAdmin
        treasury_proxy.changeProxyAdmin(not_admin);
        assertEq(treasury_proxy.proxyAdmin(), not_admin);
    }




}