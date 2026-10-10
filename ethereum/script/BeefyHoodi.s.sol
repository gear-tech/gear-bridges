// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Script} from "forge-std/Script.sol";
import {Upgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";
import {ERC20Manager} from "src/ERC20Manager.sol";
import {GovernanceAdmin} from "src/GovernanceAdmin.sol";
import {GovernancePauser} from "src/GovernancePauser.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {VaraQueueRootVerifier} from "src/VaraQueueRootVerifier.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {WrappedVara} from "src/erc20/WrappedVara.sol";
import {IGovernance} from "src/interfaces/IGovernance.sol";
import {MessageHandlerMock} from "src/mocks/MessageHandlerMock.sol";

/**
 * @dev Isolated, message-only BEEFY deployment for Hoodi.
 */
contract BeefyHoodi is Script {
    uint256 internal constant HOODI_CHAIN_ID = 560048;

    BeefyClient internal beefyClient;
    VaraQueueRootVerifier internal verifier;
    MessageQueue internal messageQueue;

    function run()
        public
        returns (address clientAddress, address verifierAddress, address queueAddress, address receiverAddress)
    {
        if (block.chainid != HOODI_CHAIN_ID) {
            revert("BeefyHoodi requires chain id 560048");
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
        address deployer = vm.addr(privateKey);
        bytes32 deployerSource = bytes32(uint256(uint160(deployer)));
        address predictedQueue = vm.computeCreateAddress(deployer, vm.getNonce(deployer) + 5);
        bytes32 configuredBridgeDomain = vm.envBytes32("BEEFY_BRIDGE_DOMAIN");
        bytes32 predictedBridgeDomain = keccak256(
            abi.encodePacked("vara/gear-eth-bridge-domain/v2", sourceDomain, bytes32(HOODI_CHAIN_ID), predictedQueue)
        );
        require(
            configuredBridgeDomain != bytes32(0) && configuredBridgeDomain == predictedBridgeDomain,
            "configured bridge domain mismatch"
        );

        vm.startBroadcast(privateKey);
        beefyClient = new BeefyClient(
            sourceDomain,
            HOODI_CHAIN_ID,
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
            beefyClient.destinationChainId() == HOODI_CHAIN_ID && beefyClient.destinationQueue() == predictedQueue
                && beefyClient.bridgeDomain() == configuredBridgeDomain,
            "client destination authentication mismatch"
        );

        // GovernanceAdmin, GovernancePauser, the verifier and UUPS implementation
        // precede the proxy, so the n+5 queue remains bound atomically.
        require(
            vm.computeCreateAddress(deployer, vm.getNonce(deployer) + 4) == predictedQueue, "queue prediction changed"
        );
        GovernanceAdmin governanceAdmin = new GovernanceAdmin(
            deployerSource, WrappedVara(address(0)), MessageQueue(predictedQueue), ERC20Manager(address(0))
        );
        GovernancePauser governancePauser = new GovernancePauser(
            deployerSource, WrappedVara(address(0)), MessageQueue(predictedQueue), ERC20Manager(address(0))
        );
        verifier = new VaraQueueRootVerifier(beefyClient, predictedQueue, HOODI_CHAIN_ID);

        address[] memory emergencyStopObservers = new address[](1);
        emergencyStopObservers[0] = deployer;
        messageQueue = MessageQueue(
            Upgrades.deployUUPSProxy(
                "MessageQueue.sol",
                abi.encodeCall(
                    MessageQueue.initializeBeefy,
                    (
                        IGovernance(address(governanceAdmin)),
                        IGovernance(address(governancePauser)),
                        deployer,
                        emergencyStopObservers,
                        verifier
                    )
                )
            )
        );
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
}
