// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Test} from "forge-std/Test.sol";
import {Upgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";
import {BeefyHoodi} from "script/BeefyHoodi.s.sol";
import {BeefyLocal} from "script/BeefyLocal.s.sol";
import {BeefyTokens} from "script/BeefyTokens.s.sol";
import {DeploymentScript} from "script/Deployment.s.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {RecoveryController} from "src/RecoveryController.sol";
import {VaraQueueRootVerifier} from "src/VaraQueueRootVerifier.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {VaraBridgeMetadata} from "src/beefy/VaraBridgeMetadata.sol";
import {IGovernance} from "src/interfaces/IGovernance.sol";
import {IMessageHandlerMock} from "src/interfaces/IMessageHandlerMock.sol";
import {IMessageQueue, VaraMessage} from "src/interfaces/IMessageQueue.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";
import {MessageHandlerMock} from "src/mocks/MessageHandlerMock.sol";
import {ERC20GearSupply} from "src/erc20/managed/ERC20GearSupply.sol";
import {BaseConstants} from "test/BaseConstants.sol";
import {BeefyFixtureTest} from "test/BeefyInterop.t.sol";
import {RecoverySafeTestWallet} from "test/RecoverySafeMock.sol";

contract DeploymentScriptTest is Test {
    function test_DeploymentBeefyLocal() public {
        BeefyLocal deployment = new BeefyLocal();
        vm.chainId(1);
        vm.expectRevert();
        deployment.run();

        vm.chainId(31337);
        vm.warp(vm.unixTime() / 1000);
        vm.setEnv("PRIVATE_KEY", "1");
        RecoverySafeTestWallet recoverySafe = new RecoverySafeTestWallet(3, 5);
        vm.setEnv("BEEFY_RECOVERY_WALLET", vm.toString(address(recoverySafe)));
        bytes32 sourceDomain = bytes32(uint256(0x11));
        bytes32 currentRoot = bytes32(uint256(0x22));
        bytes32 nextRoot = bytes32(uint256(0x33));
        address deployerAddress = vm.addr(1);
        address predictedQueue = vm.computeCreateAddress(deployerAddress, vm.getNonce(deployerAddress) + 12);
        bytes32 bridgeDomain = keccak256(
            abi.encodePacked("vara/gear-eth-bridge-domain/v2", sourceDomain, bytes32(uint256(31337)), predictedQueue)
        );
        vm.setEnv("BEEFY_SOURCE_DOMAIN", vm.toString(sourceDomain));
        vm.setEnv("BEEFY_BRIDGE_DOMAIN", vm.toString(bridgeDomain));
        vm.setEnv("BEEFY_MMR_START_BLOCK", "10");
        vm.setEnv("BEEFY_INITIAL_BLOCK", "12");
        uint256 initialSourceTimestampMs = block.timestamp * 1000;
        vm.setEnv("BEEFY_INITIAL_SOURCE_TIMESTAMP_MS", vm.toString(initialSourceTimestampMs));
        vm.setEnv("BEEFY_CURRENT_ID", "5");
        vm.setEnv("BEEFY_CURRENT_LENGTH", "2");
        vm.setEnv("BEEFY_CURRENT_ROOT", vm.toString(currentRoot));
        vm.setEnv("BEEFY_NEXT_ID", "6");
        vm.setEnv("BEEFY_NEXT_LENGTH", "2");
        vm.setEnv("BEEFY_NEXT_ROOT", vm.toString(nextRoot));

        (address client, address verifier, address queue,) = deployment.run();
        MessageQueue deployedQueue = MessageQueue(queue);
        RecoveryController recoveryController = RecoveryController(deployedQueue.recoveryController());
        assertEq(recoveryController.recoveryWallet(), address(recoverySafe));
        VaraQueueRootVerifier boundVerifier = VaraQueueRootVerifier(verifier);
        BeefyClient deployedClient = BeefyClient(client);
        assertEq(address(boundVerifier.beefyClient()), client);
        assertEq(boundVerifier.messageQueue(), queue);
        assertEq(boundVerifier.destinationChainId(), 31337);
        assertEq(deployedClient.sourceDomain(), sourceDomain);
        assertEq(deployedClient.destinationChainId(), 31337);
        assertEq(deployedClient.destinationQueue(), queue);
        assertEq(deployedClient.bridgeDomain(), bridgeDomain);
        assertEq(deployedClient.mmrStartBlock(), 10);
        assertEq(deployedClient.latestBeefyBlock(), 12);
        assertEq(deployedClient.latestMMRRoot(), bytes32(0));
        assertEq(deployedClient.lastAuthenticatedSourceTimestampMs(), initialSourceTimestampMs);
        assertTrue(deployedClient.isLive());
        assertEq(deployedClient.randaoCommitDelay(), 128);
        assertEq(deployedClient.randaoCommitExpiration(), 24);
        assertEq(deployedClient.minNumRequiredSignatures(), 86);
        assertEq(deployedClient.fiatShamirRequiredSignatures(), 86);
        assertEq(deployedClient.MAX_VALIDATORS(), 256);
        assertEq(deployedClient.MAX_SOURCE_AGE_MS(), 86_400_000);
        assertEq(deployedClient.MAX_FUTURE_SOURCE_SKEW_MS(), 120_000);
        (uint128 currentId, uint128 currentLength, bytes32 currentSetRoot,) = deployedClient.currentValidatorSet();
        (uint128 nextId, uint128 nextLength, bytes32 nextSetRoot,) = deployedClient.nextValidatorSet();
        assertEq(currentId, 5);
        assertEq(currentLength, 2);
        assertEq(currentSetRoot, currentRoot);
        assertEq(nextId, 6);
        assertEq(nextLength, 2);
        assertEq(nextSetRoot, nextRoot);
    }

    function test_DeploymentBeefyTokensQueueDomainDryRun() public {
        vm.chainId(31337);
        vm.warp(vm.unixTime() / 1000);
        uint256 privateKey = 1;
        address deployerAddress = vm.addr(privateKey);
        vm.deal(deployerAddress, 100 ether);
        uint256 startingNonce = vm.getNonce(deployerAddress);
        address predictedQueue = vm.computeCreateAddress(deployerAddress, startingNonce + 12);
        bytes32 sourceDomain = 0x2222222222222222222222222222222222222222222222222222222222222222;
        bytes32 bridgeDomain = keccak256(
            abi.encodePacked("vara/gear-eth-bridge-domain/v2", sourceDomain, bytes32(block.chainid), predictedQueue)
        );
        vm.setEnv("PRIVATE_KEY", vm.toString(privateKey));
        RecoverySafeTestWallet recoverySafe = new RecoverySafeTestWallet(3, 5);
        vm.setEnv("BEEFY_RECOVERY_WALLET", vm.toString(address(recoverySafe)));
        vm.setEnv("EXPECTED_DEPLOYER_NONCE", vm.toString(startingNonce));
        vm.setEnv("BEEFY_SOURCE_DOMAIN", vm.toString(sourceDomain));
        vm.setEnv("BEEFY_BRIDGE_DOMAIN", vm.toString(bridgeDomain));
        vm.setEnv("BEEFY_MMR_START_BLOCK", "10");
        vm.setEnv("BEEFY_INITIAL_BLOCK", "12");
        vm.setEnv("BEEFY_INITIAL_SOURCE_TIMESTAMP_MS", vm.toString(block.timestamp * 1000));
        vm.setEnv("BEEFY_CURRENT_ID", "5");
        vm.setEnv("BEEFY_CURRENT_LENGTH", "2");
        vm.setEnv("BEEFY_CURRENT_ROOT", vm.toString(bytes32(uint256(0x22))));
        vm.setEnv("BEEFY_NEXT_ID", "6");
        vm.setEnv("BEEFY_NEXT_LENGTH", "2");
        vm.setEnv("BEEFY_NEXT_ROOT", vm.toString(bytes32(uint256(0x33))));
        vm.setEnv("GEAR_VFT_MANAGER", vm.toString(bytes32(uint256(0x44))));
        vm.setEnv("GEAR_GOVERNANCE_ADMIN", vm.toString(bytes32(uint256(0x55))));
        vm.setEnv("GEAR_GOVERNANCE_PAUSER", vm.toString(bytes32(uint256(0x66))));
        vm.setEnv("EMERGENCY_STOP_ADMIN", vm.toString(address(0xA11CE)));
        vm.setEnv("EMERGENCY_STOP_OBSERVER", vm.toString(address(0xB0B)));
        vm.setEnv("BRIDGING_PAYMENT_FEE", "1");

        BeefyTokens deployment = new BeefyTokens();
        (address clientAddress, address verifierAddress, address queueAddress,) = deployment.run();
        BeefyClient client = BeefyClient(clientAddress);
        VaraQueueRootVerifier verifier = VaraQueueRootVerifier(verifierAddress);
        assertEq(queueAddress, predictedQueue);
        RecoveryController recoveryController = RecoveryController(MessageQueue(queueAddress).recoveryController());
        assertEq(recoveryController.recoveryWallet(), address(recoverySafe));
        assertEq(address(verifier.beefyClient()), clientAddress);
        assertEq(verifier.messageQueue(), predictedQueue);
        assertEq(client.sourceDomain(), sourceDomain);
        assertEq(client.destinationChainId(), block.chainid);
        assertEq(client.destinationQueue(), predictedQueue);
        assertEq(client.bridgeDomain(), bridgeDomain);
        assertFalse(deployment.shouldUseOverrides());
        ERC20GearSupply ordinary = ERC20GearSupply(address(deployment.erc20GearSupply()));
        vm.expectRevert();
        ordinary.mint(address(this), 7);
        vm.prank(address(deployment.erc20Manager()));
        ordinary.mint(address(this), 7);
        assertEq(ordinary.balanceOf(address(this)), 7);
    }

    function test_DeploymentMainnet() public {
        vm.chainId(1);
        vm.warp(vm.unixTime() / 1000);
        // forge-lint: disable-start(unsafe-cheatcode)
        vm.setEnv("PRIVATE_KEY", "1");
        vm.setEnv("CIRCLE_TOKEN", vm.toString(0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48));
        vm.setEnv("TETHER_TOKEN", vm.toString(0xdAC17F958D2ee523a2206206994597C13D831ec7));
        vm.setEnv("WRAPPED_ETHER", vm.toString(0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2));
        vm.setEnv("VFT_MANAGER", vm.toString(BaseConstants.VFT_MANAGER));
        vm.setEnv("GOVERNANCE_ADMIN", vm.toString(BaseConstants.GOVERNANCE_ADMIN));
        vm.setEnv("GOVERNANCE_PAUSER", vm.toString(BaseConstants.GOVERNANCE_PAUSER));
        vm.setEnv("EMERGENCY_STOP_ADMIN", vm.toString(BaseConstants.EMERGENCY_STOP_ADMIN));
        vm.setEnv(
            "EMERGENCY_STOP_OBSERVERS",
            string.concat(
                vm.toString(BaseConstants.EMERGENCY_STOP_OBSERVER1),
                ",",
                vm.toString(BaseConstants.EMERGENCY_STOP_OBSERVER2)
            )
        );
        vm.setEnv("BRIDGING_PAYMENT_FEE", vm.toString(BaseConstants.BRIDGING_PAYMENT_FEE));
        // forge-lint: disable-end(unsafe-cheatcode)
        DeploymentScript deploymentScript = new DeploymentScript();
        deploymentScript.run();
    }

    function test_DeploymentHoodi() public {
        vm.chainId(560048);
        vm.warp(vm.unixTime() / 1000);
        // forge-lint: disable-start(unsafe-cheatcode)
        vm.setEnv("PRIVATE_KEY", "1");
        vm.setEnv("VFT_MANAGER", vm.toString(BaseConstants.VFT_MANAGER));
        vm.setEnv("GOVERNANCE_ADMIN", vm.toString(BaseConstants.GOVERNANCE_ADMIN));
        vm.setEnv("GOVERNANCE_PAUSER", vm.toString(BaseConstants.GOVERNANCE_PAUSER));
        vm.setEnv("EMERGENCY_STOP_ADMIN", vm.toString(BaseConstants.EMERGENCY_STOP_ADMIN));
        vm.setEnv(
            "EMERGENCY_STOP_OBSERVERS",
            string.concat(
                vm.toString(BaseConstants.EMERGENCY_STOP_OBSERVER1),
                ",",
                vm.toString(BaseConstants.EMERGENCY_STOP_OBSERVER2)
            )
        );
        vm.setEnv("BRIDGING_PAYMENT_FEE", vm.toString(BaseConstants.BRIDGING_PAYMENT_FEE));
        // forge-lint: disable-end(unsafe-cheatcode)
        DeploymentScript deploymentScript = new DeploymentScript();
        deploymentScript.run();
    }
}

contract BeefyHoodiDeploymentTest is BeefyFixtureTest {
    uint256 private constant DEPLOYER_KEY = 1;

    function setUp() public {
        fixtures = vm.readFile("test/fixtures/beefy-interop.json");
    }

    function test_HoodiRejectsWrongNetwork() public {
        BeefyHoodi deployment = new BeefyHoodi();
        vm.chainId(1);
        vm.expectRevert();
        deployment.run();
    }

    function test_HoodiBindsClientQueueAndDeployerEmergencyRoles() public {
        (BeefyClient client, VaraQueueRootVerifier verifier, MessageQueue queue,) = _deploy();
        address deployer = vm.addr(DEPLOYER_KEY);
        bytes32 deployerSource = bytes32(uint256(uint160(deployer)));

        assertEq(address(verifier.beefyClient()), address(client));
        assertEq(verifier.messageQueue(), address(queue));
        assertEq(verifier.destinationChainId(), 560048);
        assertEq(client.sourceDomain(), fixtureHash(0, "sourceDomain"));
        assertEq(client.destinationChainId(), 560048);
        assertEq(client.destinationQueue(), address(queue));
        assertEq(
            client.bridgeDomain(),
            keccak256(
                abi.encodePacked(
                    "vara/gear-eth-bridge-domain/v2", client.sourceDomain(), bytes32(uint256(560048)), address(queue)
                )
            )
        );
        assertEq(queue.verifier(), address(verifier));
        RecoveryController recoveryController = RecoveryController(queue.recoveryController());
        assertEq(recoveryController.recoveryWallet(), vm.envAddress("BEEFY_RECOVERY_WALLET"));
        assertEq(queue.emergencyStopAdmin(), deployer);
        address[] memory observers = queue.emergencyStopObservers();
        assertEq(observers.length, 1);
        assertEq(observers[0], deployer);
        assertEq(IGovernance(queue.governanceAdmin()).governance(), deployerSource);
        assertEq(IGovernance(queue.governancePauser()).governance(), deployerSource);
        assertTrue(queue.hasRole(queue.DEFAULT_ADMIN_ROLE(), queue.governanceAdmin()));
        assertTrue(queue.hasRole(queue.PAUSER_ROLE(), queue.governancePauser()));

        vm.prank(deployer);
        queue.challengeRoot();
        assertTrue(queue.isChallengingRoot());
        vm.prank(deployer);
        queue.disableChallengeRoot();
        assertFalse(queue.isChallengingRoot());
    }

    function test_HoodiRejectsUnauthorizedInitializationAndRoles() public {
        (,, MessageQueue queue,) = _deploy();
        address unauthorized = address(0xBEEF);
        address implementation = Upgrades.getImplementationAddress(address(queue));
        address[] memory noObservers = new address[](0);

        (bool implementationInitialized,) = implementation.call(
            abi.encodeCall(
                MessageQueue.initialize,
                (IGovernance(address(0)), IGovernance(address(0)), unauthorized, noObservers, IVerifier(address(0)))
            )
        );
        assertFalse(implementationInitialized);
        (bool proxyInitialized,) = address(queue)
            .call(
                abi.encodeCall(
                    MessageQueue.initialize,
                    (IGovernance(address(0)), IGovernance(address(0)), unauthorized, noObservers, IVerifier(address(0)))
                )
            );
        assertFalse(proxyInitialized);
        vm.startPrank(unauthorized);
        (bool paused,) = address(queue).call(abi.encodeWithSelector(MessageQueue.pause.selector));
        (bool granted,) = address(queue)
            .call(abi.encodeWithSignature("grantRole(bytes32,address)", queue.PAUSER_ROLE(), unauthorized));
        vm.stopPrank();
        assertFalse(paused);
        assertFalse(granted);
    }

    function test_HoodiAuthenticatesV2ProofAndHonorsCharlieUserDelay() public {
        (BeefyClient client, VaraQueueRootVerifier verifier, MessageQueue queue, MessageHandlerMock receiver) =
            _deploy();
        VaraMessage memory message = VaraMessage({
            nonce: 7,
            source: bytes32(uint256(0xCAFE)),
            destination: address(receiver),
            payload: bytes("charlie user message")
        });
        bytes32 messageHash =
            keccak256(abi.encodePacked(message.nonce, message.source, message.destination, message.payload));
        QueueProof memory proof = _customQueueProof(client, messageHash);
        uint256 sourceBlock = proof.leaf.parentNumber;
        uint256[] memory publicInputs = _publicInputs(proof.leaf.parentNumber, messageHash);
        vm.prank(address(queue));
        assertTrue(verifier.safeVerifyProof(encodeProof(proof), publicInputs));
        queue.submitMerkleRoot(sourceBlock, messageHash, encodeProof(proof));

        vm.expectRevert(IMessageQueue.MerkleRootDelayNotPassed.selector);
        queue.processMessage(sourceBlock, 1, 0, message, new bytes32[](0));
        vm.warp(block.timestamp + queue.PROCESS_USER_MESSAGE_DELAY());
        vm.expectEmit(true, false, false, true, address(receiver));
        emit IMessageHandlerMock.MessageHandled(message.source, message.payload);
        queue.processMessage(sourceBlock, 1, 0, message, new bytes32[](0));
        assertTrue(queue.isProcessed(message.nonce));
    }

    function _customQueueProof(BeefyClient client, bytes32 messageHash) internal returns (QueueProof memory p) {
        uint64 historicalTimestamp = uint64(fixtureUint(0, "sourceTimestampMs"));
        uint64 freshnessTimestamp = uint64(fixtureUint(0, "freshnessProof.sourceTimestampMs"));
        p = queueProof(0);
        bytes32 bridgeDomain = client.bridgeDomain();
        uint64 queueId = uint64(fixtureUint(0, "queueId"));
        uint64 freshnessQueueId = uint64(fixtureUint(0, "freshnessProof.queueId"));
        bytes32 freshnessRoot = fixtureHash(0, "freshnessProof.queueRoot");
        VaraBridgeMetadata.Snapshot memory historicalSnapshot = VaraBridgeMetadata.Snapshot({
            version: 2,
            bridgeDomain: bridgeDomain,
            sourceTimestampMs: historicalTimestamp,
            initialized: true,
            queueId: queueId,
            queueRoot: messageHash
        });
        VaraBridgeMetadata.Snapshot memory freshnessSnapshot = VaraBridgeMetadata.Snapshot({
            version: 2,
            bridgeDomain: bridgeDomain,
            sourceTimestampMs: freshnessTimestamp,
            initialized: true,
            queueId: freshnessQueueId,
            queueRoot: freshnessRoot
        });
        p.proofVersion = 2;
        p.bridgeVersion = 2;
        p.initialized = true;
        p.bridgeDomain = bridgeDomain;
        p.sourceTimestampMs = historicalTimestamp;
        p.queueId = queueId;
        p.leaf.parachainHeadsRoot = VaraBridgeMetadata.hash(historicalSnapshot);
        BeefyClient.MMRLeaf memory freshnessLeaf = BeefyClient.MMRLeaf({
            version: p.leaf.version,
            parentNumber: 102,
            parentHash: p.leaf.parentHash ^ bytes32(uint256(103)),
            nextAuthoritySetID: p.leaf.nextAuthoritySetID,
            nextAuthoritySetLen: p.leaf.nextAuthoritySetLen,
            nextAuthoritySetRoot: p.leaf.nextAuthoritySetRoot,
            parachainHeadsRoot: VaraBridgeMetadata.hash(freshnessSnapshot)
        });
        bytes32 historicalLeafHash = keccak256(leafBytes(p.leaf));
        bytes32 freshnessLeafHash = keccak256(leafBytes(freshnessLeaf));
        p.anchorBlock = 103;
        p.anchorRoot = keccak256(abi.encodePacked(historicalLeafHash, freshnessLeafHash));
        p.items = new bytes32[](1);
        p.items[0] = freshnessLeafHash;
        p.order = 0;
        BeefyClient.Commitment memory c = commitment(uint32(p.anchorBlock), 0, p.anchorRoot);
        for (uint256 i; i < c.payload.length; i++) {
            if (c.payload[i].payloadID == bytes2("mh")) c.payload[i].data = abi.encodePacked(p.anchorRoot);
        }
        bytes32[] memory freshnessProof = new bytes32[](1);
        freshnessProof[0] = historicalLeafHash;
        client.submitFiatShamir(
            c, allSigners(), signedProofs(client, c, ""), freshnessLeaf, freshnessSnapshot, freshnessProof, 1
        );
    }

    function _deploy()
        internal
        returns (BeefyClient client, VaraQueueRootVerifier verifier, MessageQueue queue, MessageHandlerMock receiver)
    {
        vm.chainId(560048);
        _setBootstrapEnv();
        vm.warp(uint256(bootstrapTimestamp(0)) / 1000);
        BeefyHoodi deployment = new BeefyHoodi();
        (address clientAddress, address verifierAddress, address queueAddress, address receiverAddress) =
            deployment.run();
        client = BeefyClient(clientAddress);
        verifier = VaraQueueRootVerifier(verifierAddress);
        queue = MessageQueue(queueAddress);
        receiver = MessageHandlerMock(receiverAddress);
    }

    function _setBootstrapEnv() internal {
        uint256 index;
        vm.setEnv("PRIVATE_KEY", vm.toString(DEPLOYER_KEY));
        RecoverySafeTestWallet recoverySafe = new RecoverySafeTestWallet(3, 5);
        vm.setEnv("BEEFY_RECOVERY_WALLET", vm.toString(address(recoverySafe)));
        bytes32 sourceDomain = fixtureHash(index, "sourceDomain");
        address deployer = vm.addr(DEPLOYER_KEY);
        address predictedQueue = vm.computeCreateAddress(deployer, vm.getNonce(deployer) + 5);
        bytes32 bridgeDomain = keccak256(
            abi.encodePacked("vara/gear-eth-bridge-domain/v2", sourceDomain, bytes32(uint256(560048)), predictedQueue)
        );
        vm.setEnv("BEEFY_SOURCE_DOMAIN", vm.toString(sourceDomain));
        vm.setEnv("BEEFY_BRIDGE_DOMAIN", vm.toString(bridgeDomain));
        vm.setEnv("BEEFY_MMR_START_BLOCK", vm.toString(fixtureUint(index, "mmrStartBlock")));
        vm.setEnv("BEEFY_INITIAL_BLOCK", vm.toString(fixtureUint(index, "mmrStartBlock") + 1));
        vm.setEnv("BEEFY_INITIAL_SOURCE_TIMESTAMP_MS", vm.toString(bootstrapTimestamp(index)));
        vm.setEnv("BEEFY_CURRENT_ID", "0");
        vm.setEnv("BEEFY_CURRENT_LENGTH", vm.toString(fixtureUint(index, "validatorCount")));
        vm.setEnv("BEEFY_CURRENT_ROOT", vm.toString(fixtureHash(index, "authorityRoot")));
        vm.setEnv("BEEFY_NEXT_ID", "1");
        vm.setEnv("BEEFY_NEXT_LENGTH", vm.toString(fixtureUint(index, "validatorCount")));
        vm.setEnv("BEEFY_NEXT_ROOT", vm.toString(fixtureHash(index, "authorityRoot")));
    }

    function _publicInputs(uint32 source, bytes32 root) internal pure returns (uint256[] memory inputs) {
        inputs = new uint256[](2);
        inputs[0] = uint256(root) >> 64;
        inputs[1] = (uint256(uint64(uint256(root))) << 128) | (uint256(source) << 96);
    }
}
