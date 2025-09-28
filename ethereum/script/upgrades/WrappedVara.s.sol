// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {console, Script} from "forge-std/Script.sol";
import {WrappedVara} from "src/erc20/WrappedVara.sol";

contract WrappedVaraScript is Script {
    WrappedVara public wrappedVara;

    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");

        vm.startBroadcast(privateKey);

        wrappedVara = new WrappedVara();
        console.log("WrappedVara:", address(wrappedVara));

        vm.stopBroadcast();
    }
}
