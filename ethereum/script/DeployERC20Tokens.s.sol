// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {CircleToken} from "../src/erc20/CircleToken.sol";
import {TetherToken} from "../src/erc20/TetherToken.sol";
import {WrappedEther} from "../src/erc20/WrappedEther.sol";

contract DeployERC20TokensScript is Script {
    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        address deployerAddress = vm.addr(privateKey);
        vm.startBroadcast(privateKey);

        CircleToken circleToken = new CircleToken(deployerAddress);
        circleToken.mint(deployerAddress, type(uint256).max);
        console.log("USDC:", address(circleToken));

        TetherToken tetherToken = new TetherToken(deployerAddress);
        tetherToken.mint(deployerAddress, type(uint256).max);
        console.log("USDT:", address(tetherToken));

        WrappedEther wrappedEther = new WrappedEther();
        console.log("WETH:", address(wrappedEther));

        vm.stopBroadcast();
    }
}
