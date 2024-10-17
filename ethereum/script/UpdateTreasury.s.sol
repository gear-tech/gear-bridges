pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {Test, console} from "forge-std/Test.sol";

import {ProxyContract} from "../src/ProxyContract.sol";

import {ERC20Treasury} from "../src/ERC20Treasury.sol";
import {IERC20Treasury} from "../src/interfaces/IERC20Treasury.sol";

contract UpdateTreasuryScript is Script {
    using Address for address;

    function setUp() public {}

    function run() public {
        vm.startBroadcast(vm.envUint("ETHEREUM_DEPLOYMENT_PRIVATE_KEY"));

        address message_queue_proxy_address = vm.envAddress("MQ_PROXY");
        address payable treasury_proxy_address = payable(
            vm.envAddress("TREASURY_PROXY")
        );

        ProxyContract treasury_proxy = ProxyContract(treasury_proxy_address);

        ERC20Treasury treasury = new ERC20Treasury(
            address(message_queue_proxy_address)
        );

        treasury_proxy.upgradeToAndCall(address(treasury), "");

        console.log("New treasury:", address(treasury));
        console.log("New treasury proxy:", address(treasury_proxy));

        vm.stopBroadcast();
    }
}
