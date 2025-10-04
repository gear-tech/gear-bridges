// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {console, Script} from "forge-std/Script.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";
import {Verifier} from "src/Verifier.sol";

contract VerifierScript is Script {
    IVerifier public verifier;

    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");

        vm.startBroadcast(privateKey);

        verifier = new Verifier();
        console.log("Verifier:", address(verifier));

        vm.stopBroadcast();
    }
}
