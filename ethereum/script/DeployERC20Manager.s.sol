// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {ERC20Manager} from "../src/ERC20Manager.sol";

contract Deploy is Script {
    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(privateKey);

        bytes32 vft_manager = vm.envBytes32("VFT_MANAGER");
        address message_queue_proxy_address = vm.envAddress("MQ_PROXY");

        ERC20Manager erc20_manager = new ERC20Manager(message_queue_proxy_address, vft_manager);

        console.log("ERC20Manager:", address(erc20_manager));

        vm.stopBroadcast();
    }
}
