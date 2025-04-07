pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";

import {EthereumToken} from "../src/erc20/EthereumToken.sol";

contract Deploy is Script {
    function setUp() public {}

    function run() public {
        vm.startBroadcast(vm.envUint("ETHEREUM_DEPLOYMENT_PRIVATE_KEY"));

        EthereumToken token = new EthereumToken("Ethereum token", "ETHT");
        console.log("Address:", address(token));

        vm.stopBroadcast();
    }
}
