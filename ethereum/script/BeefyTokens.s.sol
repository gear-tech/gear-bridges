// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Script} from "forge-std/Script.sol";
import {Base, DeploymentArguments} from "test/Base.sol";

/**
 * @dev Fresh, queue-bound token test bridge. Never points at the message-only deployment.
 */
contract BeefyTokens is Script, Base {
    function run()
        public
        returns (address clientAddress, address verifierAddress, address queueAddress, address managerAddress)
    {
        require(block.chainid == 31337 || block.chainid == 560048, "unsupported token test chain");
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(privateKey);
        // A partial broadcast changes CREATE predictions. Reconcile it; never simply rerun.
        require(
            vm.getNonce(deployer) == _envUint64("EXPECTED_DEPLOYER_NONCE"),
            "deployer nonce changed; reconcile partial broadcast"
        );
        address[] memory observers = new address[](1);
        observers[0] = vm.envAddress("EMERGENCY_STOP_OBSERVER");
        DeploymentArguments memory args = DeploymentArguments({
            privateKey: privateKey,
            deployerAddress: deployer,
            forkUrlOrAlias: "",
            overrides: _tokenOverridesFromEnvironment(),
            vftManager: vm.envBytes32("GEAR_VFT_MANAGER"),
            governanceAdmin: vm.envBytes32("GEAR_GOVERNANCE_ADMIN"),
            governancePauser: vm.envBytes32("GEAR_GOVERNANCE_PAUSER"),
            emergencyStopAdmin: vm.envAddress("EMERGENCY_STOP_ADMIN"),
            emergencyStopObservers: observers,
            bridgingPaymentFee: vm.envUint("BRIDGING_PAYMENT_FEE")
        });
        require(args.bridgingPaymentFee > 0, "missing payment fee");
        includeOrdinaryGearToken = true;
        deployTestTokenProfile = true; // Explicit test-token profile on the two allowed test chains.
        _validateDeploymentArguments(args);
        address predictedQueue = _predictBeefyMessageQueue(deployer, args.overrides);
        _deployBeefyClient(privateKey, predictedQueue);
        deployBridge(args);
        require(address(messageQueue) == predictedQueue, "deployed queue differs from prediction");
        return (address(beefyClient), address(verifier), address(messageQueue), address(erc20Manager));
    }
}
