pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {ERC20Mock} from "../src/mocks/ERC20Mock.sol";

contract Deploy is Script {
    using Address for address;

    function setUp() public {}

    function run() public {
        vm.startBroadcast(vm.envUint("ETHEREUM_DEPLOYMENT_PRIVATE_KEY"));

        ERC20Mock token_1 = new ERC20Mock("USDC");
        ERC20Mock token_2 = new ERC20Mock("USDT");

        console.log("USDC:", address(token_1));
        console.log("USDT:", address(token_2));

        vm.stopBroadcast();
    }
}
