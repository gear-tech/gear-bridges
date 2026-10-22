// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.38;

import {Script} from "forge-std/Script.sol";
import {Base, DeploymentArguments} from "test/Base.sol";

/**
 * @dev Isolated, message-only BEEFY deployment for Hoodi.
 */
contract BeefyHoodi is Script, Base {
    uint256 internal constant HOODI_CHAIN_ID = 560048;

    function run()
        public
        returns (address clientAddress, address verifierAddress, address queueAddress, address receiverAddress)
    {
        require(block.chainid == HOODI_CHAIN_ID, "BeefyHoodi requires chain id 560048");
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(privateKey);
        bytes32 deployerSource = bytes32(uint256(uint160(deployer)));
        messageOnlyBridge = true;
        address[] memory observers = new address[](1);
        observers[0] = deployer;
        DeploymentArguments memory args = DeploymentArguments({
            privateKey: privateKey,
            deployerAddress: deployer,
            forkUrlOrAlias: "",
            overrides: _tokenOverridesFromEnvironment(),
            vftManager: bytes32(0),
            governanceAdmin: deployerSource,
            governancePauser: deployerSource,
            emergencyStopAdmin: deployer,
            emergencyStopObservers: observers,
            bridgingPaymentFee: 0
        });
        _validateDeploymentArguments(args);
        address predictedQueue = _predictBeefyMessageQueue(deployer, args.overrides);
        _deployBeefyClient(privateKey, predictedQueue);
        deployBridge(args);
        return (address(beefyClient), address(verifier), address(messageQueue), address(messageHandlerMock));
    }
}
