// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Script, console} from "forge-std/Script.sol";

abstract contract DeployCommonScript is Script {
    function printContractInfo(string memory contractName, address contractAddress, address expectedImplementation)
        public
        view
    {
        console.log("================================================================================================");
        console.log("[ CONTRACT  ]", contractName);
        console.log("[ ADDRESS   ]", contractAddress);
        if (expectedImplementation != address(0)) {
            console.log("[ IMPL ADDR ]", expectedImplementation);
            console.log(
                "[ PROXY VERIFICATION ] Click \"Is this a proxy?\" on Etherscan to be able read and write as proxy."
            );
            console.log("                       Alternatively, run the following curl request.");
            console.log("```");
            uint256 chainId = block.chainid;
            if (chainId == 1) {
                console.log("curl --request POST 'https://api.etherscan.io/api' \\");
            } else {
                console.log(
                    string.concat(
                        "curl --request POST 'https://api-", vm.getChain(chainId).chainAlias, ".etherscan.io/api' \\"
                    )
                );
            }
            console.log("   --header 'Content-Type: application/x-www-form-urlencoded' \\");
            console.log("   --data-urlencode 'module=contract' \\");
            console.log("   --data-urlencode 'action=verifyproxycontract' \\");
            console.log(string.concat("   --data-urlencode 'address=", vm.toString(contractAddress), "' \\"));
            console.log(
                string.concat(
                    "   --data-urlencode 'expectedimplementation=", vm.toString(expectedImplementation), "' \\"
                )
            );
            console.log("   --data-urlencode \"apikey=$ETHERSCAN_API_KEY\"");
            console.log("```");
        }
        console.log("================================================================================================");
    }
}
