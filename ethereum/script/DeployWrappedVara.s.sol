// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {WrappedVara} from "../src/erc20/WrappedVara.sol";

contract DeployWrappedVaraScript is Script {
    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(privateKey);

        address erc20_manager_proxy_address = vm.envAddress("ERC20_MANAGER_PROXY");

        WrappedVara token = new WrappedVara(erc20_manager_proxy_address, "Wrapped VARA", "WVARA");
        console.log("Address:", address(token));

        vm.stopBroadcast();
    }
}
