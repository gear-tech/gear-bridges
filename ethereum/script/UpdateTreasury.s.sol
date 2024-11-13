pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {Test, console} from "forge-std/Test.sol";

import {ProxyContract} from "../src/ProxyContract.sol";

import {ERC20Manager} from "../src/ERC20Manager.sol";
import {IERC20Manager} from "../src/interfaces/IERC20Manager.sol";

contract UpdateTreasuryScript is Script {
    using Address for address;

    function setUp() public {}

    function run() public {
        vm.startBroadcast(vm.envUint("ETHEREUM_DEPLOYMENT_PRIVATE_KEY"));

        address message_queue_proxy_address = vm.envAddress("MQ_PROXY");
        address payable treasury_proxy_address = payable(
            vm.envAddress("TREASURY_PROXY")
        );
        bytes32 vft_manager = vm.envBytes32("VFT_MANAGER");

        ProxyContract treasury_proxy = ProxyContract(treasury_proxy_address);

        ERC20Manager treasury = new ERC20Manager(
            address(message_queue_proxy_address),
            vft_manager
        );

        treasury_proxy.upgradeToAndCall(address(treasury), "");

        console.log("New treasury:", address(treasury));
        console.log("New treasury proxy:", address(treasury_proxy));

        vm.stopBroadcast();
    }
}
