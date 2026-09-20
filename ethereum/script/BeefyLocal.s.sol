// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Script} from "forge-std/Script.sol";
import {VaraQueueRootVerifier} from "src/VaraQueueRootVerifier.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";
import {MessageHandlerMock} from "src/mocks/MessageHandlerMock.sol";
import {Base, DeploymentArguments, Overrides} from "test/Base.sol";
import {BaseConstants} from "test/BaseConstants.sol";

contract BeefyLocal is Script, Base {
    BeefyClient internal beefyClient;

    function run()
        public
        returns (address clientAddress, address verifierAddress, address queueAddress, address receiverAddress)
    {
        if (block.chainid != 31337) {
            revert("BeefyLocal requires chain id 31337");
        }

        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        bytes32 authorityRoot = vm.envBytes32("BEEFY_AUTHORITY_ROOT");
        address deployerAddress = vm.addr(privateKey);

        BeefyClient.ValidatorSet memory currentSet = BeefyClient.ValidatorSet({id: 0, length: 2, root: authorityRoot});
        BeefyClient.ValidatorSet memory nextSet = BeefyClient.ValidatorSet({id: 1, length: 2, root: authorityRoot});

        vm.startBroadcast(privateKey);
        beefyClient = new BeefyClient(128, 24, 17, 111, 0, currentSet, nextSet);
        vm.stopBroadcast();

        address[] memory emergencyStopObservers = new address[](2);
        emergencyStopObservers[0] = BaseConstants.EMERGENCY_STOP_OBSERVER1;
        emergencyStopObservers[1] = BaseConstants.EMERGENCY_STOP_OBSERVER2;

        deployBridge(
            DeploymentArguments({
                privateKey: privateKey,
                deployerAddress: deployerAddress,
                forkUrlOrAlias: "",
                overrides: Overrides({
                    circleToken: BaseConstants.ZERO_ADDRESS,
                    tetherToken: BaseConstants.ZERO_ADDRESS,
                    wrappedEther: BaseConstants.ZERO_ADDRESS,
                    wrappedBitcoin: BaseConstants.ZERO_ADDRESS
                }),
                vftManager: BaseConstants.VFT_MANAGER,
                governanceAdmin: BaseConstants.GOVERNANCE_ADMIN,
                governancePauser: BaseConstants.GOVERNANCE_PAUSER,
                emergencyStopAdmin: BaseConstants.EMERGENCY_STOP_ADMIN,
                emergencyStopObservers: emergencyStopObservers,
                bridgingPaymentFee: BaseConstants.BRIDGING_PAYMENT_FEE
            })
        );

        vm.startBroadcast(privateKey);
        MessageHandlerMock receiver = new MessageHandlerMock();
        vm.stopBroadcast();

        return (address(beefyClient), address(verifier), address(messageQueue), address(receiver));
    }

    function _deployVerifier(bool isTest, bool isScript, uint256 chainId) internal override returns (IVerifier) {
        if (isScript) {
            return new VaraQueueRootVerifier(beefyClient);
        }
        return super._deployVerifier(isTest, isScript, chainId);
    }
}
