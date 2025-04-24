pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";

import {WrappedVara} from "../src/erc20/WrappedVara.sol";

contract Deploy is Script {
    function setUp() public {}

    function run() public {
        vm.startBroadcast(vm.envUint("ETHEREUM_DEPLOYMENT_PRIVATE_KEY"));

        address erc20_manager_proxy_address = vm.envAddress(
            "ERC20_MANAGER_PROXY"
        );

        WrappedVara token = new WrappedVara(
            erc20_manager_proxy_address,
            "Wrapped Vara Network Token",
            "wTVARA"
        );
        console.log("Address:", address(token));

        vm.stopBroadcast();
    }
}
