pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {Address} from "@openzeppelin/contracts/utils/Address.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {Context} from "@openzeppelin/contracts/utils/Context.sol";

import {ERC20GearSupply} from "../src/ERC20GearSupply.sol";

contract Deploy is Script {
    using Address for address;

    function setUp() public {}

    function run() public {
        vm.startBroadcast(vm.envUint("ETHEREUM_DEPLOYMENT_PRIVATE_KEY"));

        address erc20_manager_proxy_address = vm.envAddress(
            "ERC20_MANAGER_PROXY"
        );

        WrappedVara token = new WrappedVara(
            erc20_manager_proxy_address,
            "Wrapped VARA",
            "WVARA"
        );
        console.log("Address:", address(token));

        vm.stopBroadcast();
    }
}

contract WrappedVara is ERC20GearSupply {
    constructor(
        address owner,
        string memory name,
        string memory symbol
    ) ERC20GearSupply(owner, name, symbol) {}

    function decimals() public pure override returns (uint8) {
        return 12;
    }
}
