// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {TimelockController} from "@openzeppelin/contracts/governance/TimelockController.sol";
import {Script} from "forge-std/Script.sol";
import {MessageHandlerMock} from "src/mocks/MessageHandlerMock.sol";
import {Base, DeploymentArguments} from "test/Base.sol";
import {BaseConstants} from "test/BaseConstants.sol";

contract BeefyLocal is Script, Base {
    function run()
        public
        returns (address clientAddress, address verifierAddress, address queueAddress, address receiverAddress)
    {
        require(block.chainid == 31337, "BeefyLocal requires chain id 31337");
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        address deployerAddress = vm.addr(privateKey);
        address[] memory observers = new address[](2);
        observers[0] = BaseConstants.EMERGENCY_STOP_OBSERVER1;
        observers[1] = BaseConstants.EMERGENCY_STOP_OBSERVER2;
        DeploymentArguments memory args = DeploymentArguments({
            privateKey: privateKey,
            deployerAddress: deployerAddress,
            forkUrlOrAlias: "",
            overrides: _tokenOverridesFromEnvironment(),
            vftManager: BaseConstants.VFT_MANAGER,
            governanceAdmin: BaseConstants.GOVERNANCE_ADMIN,
            governancePauser: BaseConstants.GOVERNANCE_PAUSER,
            emergencyStopAdmin: BaseConstants.EMERGENCY_STOP_ADMIN,
            emergencyStopObservers: observers,
            bridgingPaymentFee: BaseConstants.BRIDGING_PAYMENT_FEE
        });
        deployTestTokenProfile = true; // Explicit local test profile, never a production token fallback.
        _validateDeploymentArguments(args);
        address predictedQueue = _predictBeefyMessageQueue(deployerAddress, args.overrides);
        _validateBeefyBootstrap(predictedQueue);
        uint256 recoveryKey = vm.envOr("LOCAL_RECOVERY_PRIVATE_KEY", uint256(0));
        if (recoveryKey != 0) {
            // Explicit local fixture only; a development proposer is not production governance.
            address recoveryProposer = vm.addr(recoveryKey);
            require(recoveryProposer != deployerAddress, "fixture recovery must use a separate signer");
            address[] memory proposers = new address[](1);
            proposers[0] = recoveryProposer;
            vm.startBroadcast(recoveryKey);
            TimelockController recovery = new TimelockController(1 hours, proposers, proposers, address(0));
            vm.stopBroadcast();
            vm.setEnv("BEEFY_RECOVERY_TIMELOCK", vm.toString(address(recovery)));
        }
        _deployBeefyClient(privateKey, predictedQueue);
        deployBridge(args);
        require(address(messageQueue) == predictedQueue, "deployed queue differs from prediction");

        vm.startBroadcast(privateKey);
        MessageHandlerMock receiver = new MessageHandlerMock();
        vm.stopBroadcast();
        return (address(beefyClient), address(verifier), address(messageQueue), address(receiver));
    }
}
