// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Script} from "forge-std/Script.sol";
import {Base, DeploymentArguments, Overrides} from "test/Base.sol";

/**
 * @dev Isolated, message-only BEEFY deployment for Hoodi.
 */
contract BeefyHoodi is Script, Base {
    uint256 internal constant HOODI_CHAIN_ID = 560048;

    function run()
        public
        returns (address clientAddress, address verifierAddress, address queueAddress, address receiverAddress)
    {
        if (block.chainid != HOODI_CHAIN_ID) {
            revert("BeefyHoodi requires chain id 560048");
        }

        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(privateKey);
        bytes32 deployerSource = bytes32(uint256(uint160(deployer)));
        // The message-only profile omits the token deployments before governance.
        address predictedQueue = vm.computeCreateAddress(deployer, vm.getNonce(deployer) + 5);
        messageOnlyBridge = true;
        _deployBeefyClient(privateKey, predictedQueue);

        address[] memory emergencyStopObservers = new address[](1);
        emergencyStopObservers[0] = deployer;
        deployBridge(
            DeploymentArguments({
                privateKey: privateKey,
                deployerAddress: deployer,
                forkUrlOrAlias: "",
                overrides: Overrides(address(0), address(0), address(0), address(0)),
                vftManager: bytes32(0),
                governanceAdmin: deployerSource,
                governancePauser: deployerSource,
                emergencyStopAdmin: deployer,
                emergencyStopObservers: emergencyStopObservers,
                bridgingPaymentFee: 0
            })
        );
        return (address(beefyClient), address(verifier), address(messageQueue), address(messageHandlerMock));
    }
}
