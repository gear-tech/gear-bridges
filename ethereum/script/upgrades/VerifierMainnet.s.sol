// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

import {console, Script} from "forge-std/Script.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";
import {VerifierMainnet} from "src/VerifierMainnet.sol";

contract VerifierMainnetScript is Script {
    IVerifier public verifier;

    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");

        vm.startBroadcast(privateKey);

        verifier = new VerifierMainnet();
        console.log("VerifierMainnet:", address(verifier));

        vm.stopBroadcast();
    }
}
