pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {Verifier} from "../src/Verifier.sol";
import {Relayer} from "../src/Relayer.sol";
import {MessageQueue} from "../src/MessageQueue.sol";
import {ProxyContract} from "../src/ProxyContract.sol";
import {ProxyUpdater} from "../src/ProxyUpdater.sol";

contract Deploy is Script {
    using Address for address;

    function setUp() public {}

    function run() public {
        vm.startBroadcast(vm.envUint("ETHEREUM_DEPLOYMENT_PRIVATE_KEY"));

        bytes32 governance_address = vm.envBytes32("GOVERNANCE_ADDRESS");

        Verifier verifier = new Verifier();

        ProxyContract message_queue_proxy = new ProxyContract();
        ProxyUpdater message_queue_proxy_updater = new ProxyUpdater(
            payable(address(message_queue_proxy)),
            governance_address,
            address(message_queue_proxy)
        );

        ProxyContract relayer_proxy = new ProxyContract();
        ProxyUpdater relayer_proxy_updater = new ProxyUpdater(
            payable(address(relayer_proxy)),
            governance_address,
            address(message_queue_proxy)
        );

        MessageQueue message_queue = new MessageQueue(address(relayer_proxy));
        Relayer relayer = new Relayer(address(verifier));

        message_queue_proxy.upgradeToAndCall(address(message_queue), "");
        relayer_proxy.upgradeToAndCall(address(relayer), "");

        message_queue_proxy.changeProxyAdmin(
            address(message_queue_proxy_updater)
        );
        relayer_proxy.changeProxyAdmin(address(relayer_proxy_updater));

        console.log("Verifier:", address(verifier));
        console.log("Relayer:", address(relayer));
        console.log("MessageQueue:", address(message_queue));

        console.log("Relayer Proxy:", address(relayer_proxy));
        console.log("MessageQueue Proxy:", address(message_queue_proxy));

        console.log("Relayer Proxy Updater:", address(relayer_proxy_updater));
        console.log(
            "MessageQueue Proxy Updater:",
            address(message_queue_proxy_updater)
        );

        vm.stopBroadcast();
    }
}
