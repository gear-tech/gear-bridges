// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Script} from "forge-std/Script.sol";
import {VaraQueueRootVerifier} from "src/VaraQueueRootVerifier.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";
import {Base, DeploymentArguments, Overrides} from "test/Base.sol";

/**
 * @dev Fresh, queue-bound token bridge. Never points at the message-only deployment.
 */
contract BeefyTokens is Script, Base {
    BeefyClient internal beefyClient;

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
        expectedMessageQueueAddress = predictedQueue;
        includeOrdinaryGearToken = true;
        bytes32 sourceDomain = vm.envBytes32("BEEFY_SOURCE_DOMAIN");
        require(sourceDomain != bytes32(0), "missing source domain");
        bytes32 configuredBridgeDomain = vm.envBytes32("BEEFY_BRIDGE_DOMAIN");
        bytes32 predictedBridgeDomain = keccak256(
            abi.encodePacked("vara/gear-eth-bridge-domain/v2", sourceDomain, bytes32(block.chainid), predictedQueue)
        );
        require(
            configuredBridgeDomain != bytes32(0) && configuredBridgeDomain == predictedBridgeDomain,
            "configured bridge domain mismatch"
        );
        uint256 fee = vm.envUint("BRIDGING_PAYMENT_FEE");
        require(fee > 0, "missing payment fee");

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

        vm.startBroadcast(privateKey);
        beefyClient = new BeefyClient(
            sourceDomain,
            block.chainid,
            predictedQueue,
            _envUint64("BEEFY_MMR_START_BLOCK"),
            _envUint64("BEEFY_INITIAL_BLOCK"),
            _envUint64("BEEFY_INITIAL_SOURCE_TIMESTAMP_MS"),
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

    function _deployVerifier(bool isTest, bool isScript, uint256 chainId, address queue)
        internal
        override
        returns (IVerifier)
    {
        if (isScript) {
            return new VaraQueueRootVerifier(beefyClient, queue, chainId);
        }
        return super._deployVerifier(isTest, isScript, chainId, queue);
    }
}
