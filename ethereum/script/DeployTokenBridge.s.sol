// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {DeployCommonScript} from "./DeployCommon.s.sol";
import {BridgingPayment} from "../src/BridgingPayment.sol";
import {ERC20Manager} from "../src/ERC20Manager.sol";
import {ProxyContract} from "../src/ProxyContract.sol";
import {ProxyUpdater} from "../src/ProxyUpdater.sol";

contract DeployTokenBridgeScript is DeployCommonScript {
    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(privateKey);

        bytes32 vftManager = vm.envBytes32("VFT_MANAGER");
        bytes32 governance = vm.envBytes32("GOVERNANCE_ADDRESS");

        address messageQueueProxyAddress = vm.envAddress("MQ_PROXY");
        address bridgingPaymentAdmin = vm.envAddress("BRIDGING_PAYMENT_ADMIN");

        uint256 fee = vm.envUint("BRIDGING_PAYMENT_FEE");

        ERC20Manager erc20Manager = new ERC20Manager(messageQueueProxyAddress, vftManager);
        ProxyContract erc20ManagerProxy = new ProxyContract();
        erc20ManagerProxy.upgradeToAndCall(address(erc20Manager), "");

        ProxyUpdater erc20ManagerProxyUpdater =
            new ProxyUpdater(erc20ManagerProxy, governance, messageQueueProxyAddress);
        erc20ManagerProxy.changeProxyAdmin(address(erc20ManagerProxyUpdater));

        printContractInfo("ERC20Manager", address(erc20ManagerProxy), address(erc20Manager));

        BridgingPayment bridgingPayment = new BridgingPayment(address(erc20ManagerProxy), fee, bridgingPaymentAdmin);

        console.log("ERC20Manager:", address(erc20Manager));
        console.log("ERC20Manager Proxy:", address(erc20ManagerProxy));
        console.log("ERC20Manager Proxy Updater:", address(erc20ManagerProxyUpdater));
        console.log("Bridging Payment:", address(bridgingPayment));

        vm.stopBroadcast();
    }
}
