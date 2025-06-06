// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {EthereumToken} from "../src/erc20/EthereumToken.sol";

contract DeployEthereumTokenScript is Script {
    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(privateKey);

        EthereumToken token = new EthereumToken("Ethereum token", "ETHT");
        console.log("Address:", address(token));

        vm.stopBroadcast();
    }
}
