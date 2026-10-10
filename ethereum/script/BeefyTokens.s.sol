// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Script} from "forge-std/Script.sol";
import {Base, DeploymentArguments, Overrides} from "test/Base.sol";

/**
 * @dev Fresh, queue-bound token bridge. Never points at the message-only deployment.
 */
contract BeefyTokens is Script, Base {
    function run()
        public
        returns (address clientAddress, address verifierAddress, address queueAddress, address managerAddress)
    {
        require(block.chainid == 31337 || block.chainid == 560048, "unsupported token test chain");
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(privateKey);
        bytes32 manager = vm.envBytes32("GEAR_VFT_MANAGER");
        bytes32 admin = vm.envBytes32("GEAR_GOVERNANCE_ADMIN");
        bytes32 pauser = vm.envBytes32("GEAR_GOVERNANCE_PAUSER");
        require(manager != bytes32(0) && admin != bytes32(0) && pauser != bytes32(0), "missing Gear identities");
        // A partial broadcast changes Base.deployBridge's CREATE-address predictions.
        // Inspect broadcast receipts and deployed contracts; never simply rerun.
        uint256 startingNonce = vm.getNonce(deployer);
        require(
            startingNonce == _envUint64("EXPECTED_DEPLOYER_NONCE"),
            "deployer nonce changed; reconcile partial broadcast"
        );
        address predictedQueue = vm.computeCreateAddress(deployer, startingNonce + 12);
        includeOrdinaryGearToken = true;
        uint256 fee = vm.envUint("BRIDGING_PAYMENT_FEE");
        require(fee > 0, "missing payment fee");
        _deployBeefyClient(privateKey, predictedQueue);

        address[] memory observers = new address[](1);
        observers[0] = vm.envAddress("EMERGENCY_STOP_OBSERVER");
        deployBridge(
            DeploymentArguments({
                privateKey: privateKey,
                deployerAddress: deployer,
                forkUrlOrAlias: "",
                overrides: Overrides({
                    circleToken: address(0),
                    tetherToken: address(0),
                    wrappedEther: address(0),
                    wrappedBitcoin: address(0)
                }),
                vftManager: manager,
                governanceAdmin: admin,
                governancePauser: pauser,
                emergencyStopAdmin: vm.envAddress("EMERGENCY_STOP_ADMIN"),
                emergencyStopObservers: observers,
                bridgingPaymentFee: fee
            })
        );
        require(address(messageQueue) == predictedQueue, "deployed queue differs from prediction");
        return (address(beefyClient), address(verifier), address(messageQueue), address(erc20Manager));
    }
}
