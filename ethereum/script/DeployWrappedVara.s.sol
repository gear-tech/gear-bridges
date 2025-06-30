// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {WrappedVara} from "../src/erc20/WrappedVara.sol";

contract DeployWrappedVaraScript is Script {
    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(privateKey);

        address erc20ManagerProxy = vm.envAddress("ERC20_MANAGER_PROXY");

        WrappedVara wrappedVara = new WrappedVara(erc20ManagerProxy);
        console.log("WVARA:", address(wrappedVara));

        vm.stopBroadcast();
    }
}
