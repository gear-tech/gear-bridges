// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import "forge-std/Script.sol";
import "src/MessageHandler.sol";

contract Deploy is Script {
    function run() external {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");

        vm.startBroadcast(privateKey);

        MessageHandler messageHandler = new MessageHandler();

        vm.stopBroadcast();

        console.log("MessageHandler deployed at:", address(messageHandler));
        console.log("Address written to: deployed_address.txt");
    }
}
