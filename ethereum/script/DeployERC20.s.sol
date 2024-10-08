pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {Test, console} from "forge-std/Test.sol";

import {ERC20Mock} from "../src/mocks/ERC20Mock.sol";

contract DeployERC20Script is Script {
    using Address for address;

    function setUp() public {}

    function run() public {
        vm.startBroadcast(vm.envUint("ETHEREUM_DEPLOYMENT_PRIVATE_KEY"));

        ERC20Mock token_1 = new ERC20Mock("mockToken1");
        ERC20Mock token_2 = new ERC20Mock("mockToken2");
        ERC20Mock token_3 = new ERC20Mock("mockToken3");
        ERC20Mock token_4 = new ERC20Mock("mockToken4");
        ERC20Mock token_5 = new ERC20Mock("mockToken5");

        console.log("Token1:", address(token_1));
        console.log("Token2:", address(token_2));
        console.log("Token3:", address(token_3));
        console.log("Token4:", address(token_4));
        console.log("Token5:", address(token_5));

        vm.stopBroadcast();
    }
}
