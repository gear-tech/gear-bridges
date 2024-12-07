pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {ProxyContract} from "../src/ProxyContract.sol";

import {ERC20Manager} from "../src/ERC20Manager.sol";
import {IERC20Manager} from "../src/interfaces/IERC20Manager.sol";

contract Update is Script {
    using Address for address;

    function setUp() public {}

    function run() public {
        vm.startBroadcast(vm.envUint("ETHEREUM_DEPLOYMENT_PRIVATE_KEY"));

        address message_queue_proxy_address = vm.envAddress("MQ_PROXY");
        address payable erc20_manager_proxy_address = payable(
            vm.envAddress("ERC20_MANAGER_PROXY")
        );
        bytes32 vft_manager = vm.envBytes32("VFT_MANAGER");

        ProxyContract erc20_manager_proxy = ProxyContract(
            erc20_manager_proxy_address
        );

        ERC20Manager erc20_manager = new ERC20Manager(
            address(message_queue_proxy_address),
            vft_manager
        );

        erc20_manager_proxy.upgradeToAndCall(address(erc20_manager), "");

        console.log("New ERC20Manager:", address(erc20_manager));

        vm.stopBroadcast();
    }
}
