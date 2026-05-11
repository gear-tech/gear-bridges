// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.35;

import {Script, console} from "forge-std/Script.sol";
import {VerifierTestnet} from "src/VerifierTestnet.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";

contract VerifierTestnetScript is Script {
    IVerifier public verifier;

    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");

        vm.startBroadcast(privateKey);

        verifier = new VerifierTestnet();
        console.log("VerifierTestnet:", address(verifier));

        vm.stopBroadcast();
    }
}
