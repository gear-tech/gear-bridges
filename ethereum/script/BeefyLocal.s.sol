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
        bytes32 sourceDomain = vm.envBytes32("BEEFY_SOURCE_DOMAIN");
        require(sourceDomain != bytes32(0), "missing source domain");
        uint64 mmrStartBlock = _envUint64("BEEFY_MMR_START_BLOCK");
        uint64 initialBeefyBlock = _envUint64("BEEFY_INITIAL_BLOCK");
        uint64 initialSourceTimestampMs = _envUint64("BEEFY_INITIAL_SOURCE_TIMESTAMP_MS");
        BeefyClient.ValidatorSet memory currentSet = BeefyClient.ValidatorSet({
            id: _envUint128("BEEFY_CURRENT_ID"),
            length: _envUint128("BEEFY_CURRENT_LENGTH"),
            root: vm.envBytes32("BEEFY_CURRENT_ROOT")
        });
        BeefyClient.ValidatorSet memory nextSet = BeefyClient.ValidatorSet({
            id: _envUint128("BEEFY_NEXT_ID"),
            length: _envUint128("BEEFY_NEXT_LENGTH"),
            root: vm.envBytes32("BEEFY_NEXT_ROOT")
        });
        address deployerAddress = vm.addr(privateKey);
        uint256 startingNonce = vm.getNonce(deployerAddress);
        address predictedQueue = vm.computeCreateAddress(deployerAddress, startingNonce + 12);
        expectedMessageQueueAddress = predictedQueue;
        bytes32 configuredBridgeDomain = vm.envBytes32("BEEFY_BRIDGE_DOMAIN");
        bytes32 predictedBridgeDomain = keccak256(
            abi.encodePacked("vara/gear-eth-bridge-domain/v2", sourceDomain, bytes32(block.chainid), predictedQueue)
        );
        require(
            configuredBridgeDomain != bytes32(0) && configuredBridgeDomain == predictedBridgeDomain,
            "configured bridge domain mismatch"
        );

        vm.startBroadcast(privateKey);
        beefyClient = new BeefyClient(
            sourceDomain,
            block.chainid,
            predictedQueue,
            mmrStartBlock,
            initialBeefyBlock,
            initialSourceTimestampMs,
            currentSet,
            nextSet
        );
        require(
            beefyClient.minNumRequiredSignatures() == 86 && beefyClient.fiatShamirRequiredSignatures() == 86
                && beefyClient.MAX_VALIDATORS() == 256 && beefyClient.randaoCommitDelay() == 128
                && beefyClient.randaoCommitExpiration() == 24,
            "client policy mismatch"
        );
        require(
            beefyClient.destinationChainId() == block.chainid && beefyClient.destinationQueue() == predictedQueue
                && beefyClient.bridgeDomain() == configuredBridgeDomain,
            "client destination authentication mismatch"
        );
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
        require(address(messageQueue) == predictedQueue, "deployed queue differs from prediction");

        vm.startBroadcast(privateKey);
        MessageHandlerMock receiver = new MessageHandlerMock();
        vm.stopBroadcast();

        return (address(beefyClient), address(verifier), address(messageQueue), address(receiver));
    }

    function _envUint64(string memory key) internal view returns (uint64 value) {
        uint256 raw = vm.envUint(key);
        require(raw <= type(uint64).max, string.concat(key, " exceeds uint64"));
        return uint64(raw);
    }

    function _envUint128(string memory key) internal view returns (uint128 value) {
        uint256 raw = vm.envUint(key);
        require(raw <= type(uint128).max, string.concat(key, " exceeds uint128"));
        return uint128(raw);
    }

    function _deployVerifier(bool isTest, bool isScript, uint256 chainId, address messageQueueAddress)
        internal
        override
        returns (IVerifier)
    {
        if (isScript) {
            return new VaraQueueRootVerifier(beefyClient, messageQueueAddress, chainId);
        }
        return super._deployVerifier(isTest, isScript, chainId, messageQueueAddress);
    }
}
