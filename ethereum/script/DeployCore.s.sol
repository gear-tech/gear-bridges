// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {DeployCommonScript} from "./DeployCommon.s.sol";
import {IRelayer} from "../src/interfaces/IRelayer.sol";
import {MessageQueue} from "../src/MessageQueue.sol";
import {ProxyContract} from "../src/ProxyContract.sol";
import {ProxyUpdater} from "../src/ProxyUpdater.sol";
import {Relayer} from "../src/Relayer.sol";
import {Verifier} from "../src/Verifier.sol";

contract DeployCoreScript is DeployCommonScript {
    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(privateKey);

        bytes32 governance = vm.envBytes32("GOVERNANCE_ADDRESS");

        Verifier verifier = new Verifier();

        ProxyContract messageQueueProxy = new ProxyContract();
        ProxyUpdater messageQueueProxyUpdater =
            new ProxyUpdater(messageQueueProxy, governance, address(messageQueueProxy));

        ProxyContract relayerProxy = new ProxyContract();
        ProxyUpdater relayerProxyUpdater = new ProxyUpdater(relayerProxy, governance, address(messageQueueProxy));

        MessageQueue messageQueue = new MessageQueue(IRelayer(address(relayerProxy)));
        Relayer relayer = new Relayer(verifier);

        messageQueueProxy.upgradeToAndCall(address(messageQueue), "");
        relayerProxy.upgradeToAndCall(address(relayer), "");

        printContractInfo("MessageQueue", address(messageQueueProxy), address(messageQueue));
        printContractInfo("Relayer", address(relayerProxy), address(relayer));

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
