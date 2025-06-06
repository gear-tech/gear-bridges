// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {Verifier} from "../src/Verifier.sol";
import {Relayer} from "../src/Relayer.sol";
import {MessageQueue} from "../src/MessageQueue.sol";
import {ProxyContract} from "../src/ProxyContract.sol";
import {ProxyUpdater} from "../src/ProxyUpdater.sol";

contract DeployCoreScript is Script {
    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(privateKey);

        bytes32 governance = vm.envBytes32("GOVERNANCE_ADDRESS");

        Verifier verifier = new Verifier();

        ProxyContract messageQueueProxy = new ProxyContract();
        ProxyUpdater messageQueueProxyUpdater =
            new ProxyUpdater(payable(address(messageQueueProxy)), governance, address(messageQueueProxy));

        ProxyContract relayerProxy = new ProxyContract();
        ProxyUpdater relayerProxyUpdater =
            new ProxyUpdater(payable(address(relayerProxy)), governance, address(messageQueueProxy));

        MessageQueue messageQueue = new MessageQueue(address(relayerProxy));
        Relayer relayer = new Relayer(address(verifier));

        messageQueueProxy.upgradeToAndCall(address(messageQueue), "");
        relayerProxy.upgradeToAndCall(address(relayer), "");

        messageQueueProxy.changeProxyAdmin(address(messageQueueProxyUpdater));
        relayerProxy.changeProxyAdmin(address(relayerProxyUpdater));

        console.log("Verifier:", address(verifier));
        console.log("Relayer:", address(relayer));
        console.log("MessageQueue:", address(messageQueue));

        console.log("Relayer Proxy:", address(relayerProxy));
        console.log("MessageQueue Proxy:", address(messageQueueProxy));

        console.log("Relayer Proxy Updater:", address(relayerProxyUpdater));
        console.log("MessageQueue Proxy Updater:", address(messageQueueProxyUpdater));
    }
}
