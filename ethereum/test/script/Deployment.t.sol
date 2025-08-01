// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test} from "forge-std/Test.sol";
import {BaseConstants} from "test/Base.sol";
import {DeploymentScript} from "script/Deployment.s.sol";

contract DeploymentTest is Test {
    function setUp() public {}

    function test_DeploymentMainnet() public {
        vm.chainId(1);
        vm.setEnv("PRIVATE_KEY", "1");
        vm.setEnv("CIRCLE_TOKEN", vm.toString(0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48));
        vm.setEnv("TETHER_TOKEN", vm.toString(0xdAC17F958D2ee523a2206206994597C13D831ec7));
        vm.setEnv("WRAPPED_ETHER", vm.toString(0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2));
        vm.setEnv("VFT_MANAGER", vm.toString(BaseConstants.VFT_MANAGER));
        vm.setEnv("GOVERNANCE_ADMIN", vm.toString(BaseConstants.GOVERNANCE_ADMIN));
        vm.setEnv("GOVERNANCE_PAUSER", vm.toString(BaseConstants.GOVERNANCE_PAUSER));
        vm.setEnv("EMERGENCY_STOP_ADMIN", vm.toString(BaseConstants.EMERGENCY_STOP_ADMIN));
        vm.setEnv("BRIDGING_PAYMENT_FEE", vm.toString(BaseConstants.BRIDGING_PAYMENT_FEE));
        DeploymentScript deploymentScript = new DeploymentScript();
        deploymentScript.setUp();
        deploymentScript.run();
    }

    function test_DeploymentHoodi() public {
        vm.chainId(560048);
        vm.setEnv("PRIVATE_KEY", "1");
        vm.setEnv("VFT_MANAGER", vm.toString(BaseConstants.VFT_MANAGER));
        vm.setEnv("GOVERNANCE_ADMIN", vm.toString(BaseConstants.GOVERNANCE_ADMIN));
        vm.setEnv("GOVERNANCE_PAUSER", vm.toString(BaseConstants.GOVERNANCE_PAUSER));
        vm.setEnv("EMERGENCY_STOP_ADMIN", vm.toString(BaseConstants.EMERGENCY_STOP_ADMIN));
        vm.setEnv("BRIDGING_PAYMENT_FEE", vm.toString(BaseConstants.BRIDGING_PAYMENT_FEE));
        DeploymentScript deploymentScript = new DeploymentScript();
        deploymentScript.setUp();
        deploymentScript.run();
    }
}
