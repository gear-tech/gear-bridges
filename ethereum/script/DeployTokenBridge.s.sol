// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {ERC20Manager} from "../src/ERC20Manager.sol";
import {BridgingPayment} from "../src/BridgingPayment.sol";
import {ProxyContract} from "../src/ProxyContract.sol";
import {ProxyUpdater} from "../src/ProxyUpdater.sol";

contract DeployTokenBridgeScript is Script {
    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(privateKey);

        bytes32 vft_manager = vm.envBytes32("VFT_MANAGER");
        bytes32 governance_address = vm.envBytes32("GOVERNANCE_ADDRESS");

        address message_queue_proxy_address = vm.envAddress("MQ_PROXY");
        address bridging_payment_admin = vm.envAddress("BRIDGING_PAYMENT_ADMIN");

        uint256 fee = vm.envUint("BRIDGING_PAYMENT_FEE");

        ERC20Manager erc20_manager = new ERC20Manager(message_queue_proxy_address, vft_manager);
        ProxyContract erc20_manager_proxy = new ProxyContract();
        erc20_manager_proxy.upgradeToAndCall(address(erc20_manager), "");

        ProxyUpdater erc20_manager_proxy_updater =
            new ProxyUpdater(payable(address(erc20_manager_proxy)), governance_address, message_queue_proxy_address);
        erc20_manager_proxy.changeProxyAdmin(address(erc20_manager_proxy_updater));

        BridgingPayment bridging_payment =
            new BridgingPayment(address(erc20_manager_proxy), fee, bridging_payment_admin);

        console.log("ERC20Manager:", address(erc20_manager));
        console.log("ERC20Manager Proxy:", address(erc20_manager_proxy));
        console.log("ERC20Manager Proxy Updater:", address(erc20_manager_proxy_updater));
        console.log("Bridging Payment:", address(bridging_payment));

        vm.stopBroadcast();
    }
}
