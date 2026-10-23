// Copyright (C) Gear Technologies Inc.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {IAccessControl} from "@openzeppelin/contracts/access/IAccessControl.sol";
import {TimelockController} from "@openzeppelin/contracts/governance/TimelockController.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {IERC20Metadata} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";
import {Test} from "forge-std/Test.sol";
import {Upgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";
import {BeefyHoodi} from "script/BeefyHoodi.s.sol";
import {BeefyLocal} from "script/BeefyLocal.s.sol";
import {BeefyTokens} from "script/BeefyTokens.s.sol";
import {DeploymentScript} from "script/Deployment.s.sol";
import {ERC20Manager} from "src/ERC20Manager.sol";
import {GovernanceAdmin} from "src/GovernanceAdmin.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {VaraQueueRootVerifier} from "src/VaraQueueRootVerifier.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {VaraBridgeMetadata} from "src/beefy/VaraBridgeMetadata.sol";
import {ERC20GearSupply} from "src/erc20/managed/ERC20GearSupply.sol";
import {IGovernance} from "src/interfaces/IGovernance.sol";
import {IMessageHandlerMock} from "src/interfaces/IMessageHandlerMock.sol";
import {IMessageQueue, VaraMessage} from "src/interfaces/IMessageQueue.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";
import {Hasher} from "src/libraries/Hasher.sol";
import {
    GovernancePacker,
    PauseProxyMessage,
    UnpauseProxyMessage,
    UpgradeProxyMessage
} from "src/libraries/packing/GovernancePacker.sol";
import {MessageHandlerMock} from "src/mocks/MessageHandlerMock.sol";
import {Base} from "test/Base.sol";
import {BaseConstants} from "test/BaseConstants.sol";
import {BeefyFixtureTest} from "test/BeefyInterop.t.sol";

// Deployment mechanics fixtures only. Production authorization has its own
// proof-backed gate and pre-CREATE negative tests; these never qualify a live deployment.
contract DeploymentMechanicsFixture is DeploymentScript {
    function _localDeploymentFixture() internal view override returns (bool) {
        return block.chainid == 31337 || block.chainid == 560048;
    }
}

contract HoodiMechanicsFixture is BeefyHoodi {
    function _localDeploymentFixture() internal view override returns (bool) {
        return block.chainid == 560048;
    }
}

contract DeploymentScriptTest is BeefyFixtureTest {
    function setUp() public {
        _resetDeploymentEnvironment();
        fixtures = vm.readFile("test/fixtures/beefy-interop.json");
        address[] memory proposers = new address[](1);
        proposers[0] = address(this);
        TimelockController recovery = new TimelockController(1 hours, proposers, proposers, address(0));
        vm.setEnv("BEEFY_RECOVERY_TIMELOCK", vm.toString(address(recovery)));
    }

    function _resetDeploymentEnvironment() private {
        string[11] memory keys = [
            "BEEFY_SOURCE_DOMAIN",
            "BEEFY_BRIDGE_DOMAIN",
            "BEEFY_MMR_START_BLOCK",
            "BEEFY_INITIAL_BLOCK",
            "BEEFY_INITIAL_SOURCE_TIMESTAMP_MS",
            "BEEFY_CURRENT_ID",
            "BEEFY_CURRENT_LENGTH",
            "BEEFY_CURRENT_ROOT",
            "BEEFY_NEXT_ID",
            "BEEFY_NEXT_LENGTH",
            "BEEFY_NEXT_ROOT"
        ];
        for (uint256 i; i < keys.length; ++i) {
            vm.setEnv(keys[i], "");
        }
        vm.setEnv("LOCAL_RECOVERY_PRIVATE_KEY", "0");
        vm.setEnv("CIRCLE_TOKEN", vm.toString(address(0)));
        vm.setEnv("TETHER_TOKEN", vm.toString(address(0)));
        vm.setEnv("WRAPPED_ETHER", vm.toString(address(0)));
        vm.setEnv("WRAPPED_BITCOIN", vm.toString(address(0)));
    }

    function test_DeploymentBeefyLocal() public {
        BeefyLocal deployment = new BeefyLocal();
        vm.chainId(1);
        vm.expectRevert();
        deployment.run();

        vm.chainId(31337);
        vm.warp(vm.unixTime() / 1000);
        vm.setEnv("PRIVATE_KEY", "1");
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
        assertEq(deployedQueue.beefyRootMinimum(), 10);
        assertEq(deployedQueue.beefyRecoveryTimelock(), vm.envAddress("BEEFY_RECOVERY_TIMELOCK"));
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
        (uint128 currentId, uint128 currentLength, bytes32 currentSetRoot) = deployedClient.currentValidatorSet();
        (uint128 nextId, uint128 nextLength, bytes32 nextSetRoot) = deployedClient.nextValidatorSet();
        assertEq(currentId, 5);
        assertEq(currentLength, 2);
        assertEq(currentSetRoot, currentRoot);
        assertEq(nextId, 6);
        assertEq(nextLength, 2);
        assertEq(nextSetRoot, nextRoot);
    }

    function test_DeploymentBeefyTokensQueueDomainDryRun() public {
        vm.chainId(31337);
        vm.setEnv("BEEFY_LOCAL_FIXTURE", "true");
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
        assertEq(MessageQueue(queueAddress).beefyRootMinimum(), 10);
        assertEq(MessageQueue(queueAddress).beefyRecoveryTimelock(), vm.envAddress("BEEFY_RECOVERY_TIMELOCK"));
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

    function test_DeploymentBeefyLocalAtNonzeroNonce() public {
        vm.setNonce(vm.addr(1), 17);
        test_DeploymentBeefyLocal();
    }

    function test_DeploymentBeefyTokensAtNonzeroNonce() public {
        vm.setNonce(vm.addr(1), 23);
        test_DeploymentBeefyTokensQueueDomainDryRun();
    }

    function test_DeploymentMainnetRejectsPartialTokens() public {
        _resetDeploymentEnvironment();
        vm.chainId(1);
        vm.warp(vm.unixTime() / 1000);
        // forge-lint: disable-start(unsafe-cheatcode)
        vm.setEnv("PRIVATE_KEY", "1");
        vm.setEnv("DEPLOYMENT_MODE", "fresh-beefy");
        vm.setEnv("DEPLOY_TEST_TOKENS", "false");
        vm.setEnv("WRAPPED_BITCOIN", vm.toString(address(0)));
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
        uint64 nonce = vm.getNonce(vm.addr(1));
        vm.expectRevert();
        deploymentScript.run();
        assertEq(vm.getNonce(vm.addr(1)), nonce, "preflight must not broadcast");
    }

    function test_DeploymentHoodi() public {
        _resetDeploymentEnvironment();
        vm.chainId(560048);
        vm.warp(vm.unixTime() / 1000);
        // forge-lint: disable-start(unsafe-cheatcode)
        vm.setEnv("PRIVATE_KEY", "1");
        vm.setEnv("DEPLOYMENT_MODE", "fresh-beefy");
        vm.setEnv("DEPLOY_TEST_TOKENS", "true");
        vm.setEnv("CIRCLE_TOKEN", vm.toString(address(0)));
        vm.setEnv("TETHER_TOKEN", vm.toString(address(0)));
        vm.setEnv("WRAPPED_ETHER", vm.toString(address(0)));
        vm.setEnv("WRAPPED_BITCOIN", vm.toString(address(0)));
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
        _genericBootstrap(false);
        DeploymentScript deploymentScript = new DeploymentMechanicsFixture();
        deploymentScript.run();
        _assertGenericDeployment(deploymentScript);
    }

    function test_DeploymentHoodiAtNonzeroNonce() public {
        vm.setNonce(vm.addr(1), 37);
        test_DeploymentHoodi();
    }

    function test_DeploymentMainnetWithCanonicalTokensRejectsBeforeCreate() public {
        vm.chainId(1);
        vm.warp(vm.unixTime() / 1000);
        _genericEnvironment(false);
        address[4] memory canonical = [
            0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48,
            0xdAC17F958D2ee523a2206206994597C13D831ec7,
            0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2,
            0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599
        ];
        address token = address(new ERC20GearSupply(address(this), "Existing token", "EXT", 18));
        string[4] memory keys = ["CIRCLE_TOKEN", "TETHER_TOKEN", "WRAPPED_ETHER", "WRAPPED_BITCOIN"];
        for (uint256 i; i < canonical.length; ++i) {
            vm.etch(canonical[i], token.code);
            vm.setEnv(keys[i], vm.toString(canonical[i]));
        }
        _genericBootstrap(true);
        DeploymentScript deployment = new DeploymentScript();
        uint64 nonce = vm.getNonce(vm.addr(1));
        vm.expectRevert(bytes4(keccak256("FreshMainnetDeploymentForbidden()")));
        deployment.run();
        assertEq(vm.getNonce(vm.addr(1)), nonce);
    }

    function test_DeploymentMainnetAtNonzeroNonceRejectsBeforeCreate() public {
        vm.setNonce(vm.addr(1), 41);
        test_DeploymentMainnetWithCanonicalTokensRejectsBeforeCreate();
    }

    function test_DeploymentHoodiWithCompleteOverrides() public {
        vm.chainId(560048);
        vm.warp(vm.unixTime() / 1000);
        _genericEnvironment(false);
        string[4] memory keys = ["CIRCLE_TOKEN", "TETHER_TOKEN", "WRAPPED_ETHER", "WRAPPED_BITCOIN"];
        for (uint256 i; i < keys.length; ++i) {
            address token = address(new ERC20GearSupply(address(this), "Existing token", "EXT", 18));
            vm.setEnv(keys[i], vm.toString(token));
        }
        _genericBootstrap(true);
        DeploymentScript deployment = new DeploymentMechanicsFixture();
        deployment.run();
        _assertGenericDeployment(deployment);
        assertTrue(deployment.shouldUseOverrides());
    }

    function _genericEnvironment(bool generateTokens) private {
        vm.setEnv("BEEFY_LOCAL_FIXTURE", "true");
        vm.setEnv("PRIVATE_KEY", "1");
        vm.setEnv("DEPLOYMENT_MODE", "fresh-beefy");
        vm.setEnv("DEPLOY_TEST_TOKENS", generateTokens ? "true" : "false");
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
    }

    function _genericBootstrap(bool existingTokens) private {
        address deployer = vm.addr(1);
        vm.deal(deployer, 100 ether);
        address predictedQueue = vm.computeCreateAddress(deployer, vm.getNonce(deployer) + (existingTokens ? 8 : 12));
        bytes32 source = fixtureHash(0, "sourceDomain");
        vm.setEnv("BEEFY_SOURCE_DOMAIN", vm.toString(source));
        vm.setEnv(
            "BEEFY_BRIDGE_DOMAIN",
            vm.toString(
                keccak256(
                    abi.encodePacked("vara/gear-eth-bridge-domain/v2", source, bytes32(block.chainid), predictedQueue)
                )
            )
        );
        vm.setEnv("BEEFY_MMR_START_BLOCK", "1");
        vm.setEnv("BEEFY_INITIAL_BLOCK", "2");
        vm.setEnv("BEEFY_INITIAL_SOURCE_TIMESTAMP_MS", vm.toString(block.timestamp * 1000));
        vm.setEnv("BEEFY_CURRENT_ID", "0");
        vm.setEnv("BEEFY_CURRENT_LENGTH", vm.toString(fixtureUint(0, "validatorCount")));
        vm.setEnv("BEEFY_CURRENT_ROOT", vm.toString(fixtureHash(0, "authorityRoot")));
        vm.setEnv("BEEFY_NEXT_ID", "1");
        vm.setEnv("BEEFY_NEXT_LENGTH", vm.toString(fixtureUint(0, "validatorCount")));
        vm.setEnv("BEEFY_NEXT_ROOT", vm.toString(fixtureHash(0, "authorityRoot")));
    }

    function _assertGenericDeployment(DeploymentScript deployment) private {
        MessageQueue queue = deployment.messageQueue();
        VaraQueueRootVerifier verifier = VaraQueueRootVerifier(address(deployment.verifier()));
        BeefyClient client = verifier.beefyClient();
        assertEq(client.destinationQueue(), address(queue));
        assertEq(client.destinationChainId(), block.chainid);
        assertEq(client.sourceDomain(), fixtureHash(0, "sourceDomain"));
        assertEq(client.bridgeDomain(), vm.envBytes32("BEEFY_BRIDGE_DOMAIN"));
        assertEq(verifier.messageQueue(), address(queue));
        assertEq(queue.verifier(), address(verifier));
        assertEq(queue.beefyRootMinimum(), 1);
        assertEq(queue.beefyRecoveryTimelock(), vm.envAddress("BEEFY_RECOVERY_TIMELOCK"));
        assertEq(queue.governanceAdmin(), address(deployment.governanceAdmin()));
        assertEq(queue.governancePauser(), address(deployment.governancePauser()));
        assertEq(deployment.governanceAdmin().governance(), BaseConstants.GOVERNANCE_ADMIN);
        assertEq(deployment.governancePauser().governance(), BaseConstants.GOVERNANCE_PAUSER);
        assertEq(deployment.erc20Manager().messageQueue(), address(queue));
        assertEq(deployment.erc20Manager().totalTokens(), 5);
        assertEq(deployment.bridgingPayment().fee(), BaseConstants.BRIDGING_PAYMENT_FEE);
        bytes32 root = keccak256("generic real-client deployment");
        bytes memory proof = encodeProof(
            customQueueProofFor(client, 100, 103, root, uint64(block.timestamp * 1000), uint64(block.timestamp * 1000))
        );
        queue.submitMerkleRoot(100, root, proof);
        assertEq(queue.getMerkleRoot(100), root);
    }
}

contract BeefyHoodiDeploymentTest is BeefyFixtureTest {
    uint256 private constant DEPLOYER_KEY = 1;

    function setUp() public {
        fixtures = vm.readFile("test/fixtures/beefy-interop.json");
        vm.setEnv("CIRCLE_TOKEN", vm.toString(address(0)));
        vm.setEnv("TETHER_TOKEN", vm.toString(address(0)));
        vm.setEnv("WRAPPED_ETHER", vm.toString(address(0)));
        vm.setEnv("WRAPPED_BITCOIN", vm.toString(address(0)));
        address[] memory proposers = new address[](1);
        proposers[0] = address(this);
        TimelockController recovery = new TimelockController(1 hours, proposers, proposers, address(0));
        vm.setEnv("BEEFY_RECOVERY_TIMELOCK", vm.toString(address(recovery)));
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
        assertEq(queue.beefyRootMinimum(), client.mmrStartBlock());
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

    function test_HoodiQueuePredictionAtNonzeroNonce() public {
        address deployer = vm.addr(DEPLOYER_KEY);
        vm.setNonce(deployer, 31);
        address expectedQueue = vm.computeCreateAddress(deployer, 31 + 5);
        (BeefyClient client,, MessageQueue queue,) = _deploy();
        assertEq(address(queue), expectedQueue);
        assertEq(client.destinationQueue(), expectedQueue);
    }

    function test_HoodiRejectsMismatchedConfiguredDomain() public {
        vm.chainId(560048);
        _setBootstrapEnv();
        vm.setEnv("BEEFY_BRIDGE_DOMAIN", vm.toString(bytes32(uint256(1))));
        vm.warp(uint256(bootstrapTimestamp(0)) / 1000);
        BeefyHoodi deployment = new BeefyHoodi();
        vm.expectRevert();
        deployment.run();
    }

    function test_HoodiRejectsUnauthorizedInitializationAndRoles() public {
        (,, MessageQueue queue,) = _deploy();
        address unauthorized = address(0xBEEF);
        address implementation = Upgrades.getImplementationAddress(address(queue));
        address[] memory noObservers = new address[](0);

        (bool implementationInitialized,) = implementation.call(
            abi.encodeCall(
                MessageQueue.initializeBeefy,
                (
                    IGovernance(address(0)),
                    IGovernance(address(0)),
                    unauthorized,
                    noObservers,
                    IVerifier(address(0)),
                    address(0)
                )
            )
        );
        assertFalse(implementationInitialized);
        (bool proxyInitialized,) = address(queue)
            .call(
                abi.encodeCall(
                    MessageQueue.initializeBeefy,
                    (
                        IGovernance(address(0)),
                        IGovernance(address(0)),
                        unauthorized,
                        noObservers,
                        IVerifier(address(0)),
                        address(0)
                    )
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
        vm.prank(address(queue));
        assertTrue(verifier.safeVerifyProof(sourceBlock, messageHash, encodeProof(proof)));
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
        BeefyHoodi deployment = new HoodiMechanicsFixture();
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
}

contract MainnetBridgeMigrationTest is Test, Base {
    using Hasher for VaraMessage;
    using GovernancePacker for UpgradeProxyMessage;
    using GovernancePacker for PauseProxyMessage;
    using GovernancePacker for UnpauseProxyMessage;

    // Fixture JSON materialization and the multi-transaction scenario are not a gas benchmark.
    function test_DeploymentMainnetBeefyMigration() public noGasMetering {
        string memory rpc = vm.envOr("MAINNET_MIGRATION_RPC_URL", string(""));
        require(
            bytes(rpc).length != 0 || !vm.envOr("REQUIRE_MAINNET_MIGRATION", false), "migration archive RPC required"
        );
        if (bytes(rpc).length == 0) {
            vm.skip(true);
            return;
        }
        string memory artifact = vm.readFile("test/fixtures/mainnet-legacy-queue.json");
        vm.createSelectFork(rpc, vm.parseJsonUint(artifact, ".ethereumBlock"));
        MessageQueue legacy = MessageQueue(vm.parseJsonAddress(artifact, ".queue"));
        address manager = address(GovernanceAdmin(legacy.governanceAdmin()).erc20Manager());
        address oldVerifier = legacy.verifier();
        bytes32 oldVerifierCodeHash = oldVerifier.codehash;
        uint256 oldMax = legacy.maxBlockNumber();
        uint256 genesis = legacy.genesisBlock();
        address[] memory tokens = ERC20Manager(manager).tokens();
        uint256[] memory balances = new uint256[](tokens.length);
        uint256[] memory supplies = new uint256[](tokens.length);
        for (uint256 i; i < tokens.length; ++i) {
            balances[i] = IERC20Metadata(tokens[i]).balanceOf(manager);
            supplies[i] = IERC20Metadata(tokens[i]).totalSupply();
        }
        VaraMessage memory claim = VaraMessage(
            vm.parseJsonUint(artifact, ".claims.pending21.nonce"),
            vm.parseJsonBytes32(artifact, ".claims.pending21.source"),
            vm.parseJsonAddress(artifact, ".claims.pending21.destination"),
            vm.parseJsonBytes(artifact, ".claims.pending21.payload")
        );
        uint256 claimBlock = vm.parseJsonUint(artifact, ".claims.pending21.sourceBlock");
        bytes32 claimRoot = legacy.getMerkleRoot(claimBlock);
        uint256 timestamp = uint256(vm.load(address(legacy), keccak256(abi.encode(claimRoot, uint256(12)))));
        assertFalse(legacy.isProcessed(claim.nonce));
        assertTrue(legacy.isProcessed(836));
        assertTrue(legacy.isProcessed(864));

        // Exercises the same deployBridge branch as the fork deployment, not an impersonated upgrade.
        deployBridgeFromConstants(BaseConstants.DEPLOYER_ADDRESS, rpc);
        assertEq(address(messageQueue), address(legacy));
        assertEq(address(erc20Manager), manager);
        assertEq(oldVerifier.codehash, oldVerifierCodeHash);
        assertEq(messageQueue.genesisBlock(), genesis);
        assertEq(messageQueue.maxBlockNumber(), oldMax + 4);
        assertEq(messageQueue.beefyRootMinimum(), oldMax + 3);
        assertEq(messageQueue.getMerkleRoot(claimBlock), claimRoot);
        assertEq(messageQueue.getMerkleRootTimestampForBlock(claimBlock), timestamp);
        assertTrue(messageQueue.isProcessed(836));
        assertTrue(messageQueue.isProcessed(864));
        assertFalse(messageQueue.paused());
        for (uint256 i; i < tokens.length; ++i) {
            assertEq(erc20Manager.tokens(i, 1)[0], tokens[i]);
            assertEq(IERC20Metadata(tokens[i]).balanceOf(manager), balances[i]);
            assertEq(IERC20Metadata(tokens[i]).totalSupply(), supplies[i]);
        }
        assertEq(
            uint64(uint256(vm.load(address(messageQueue), vm.parseJsonBytes32(artifact, ".initializableSlot")))), 8
        );
        address placeholder = 0x1111111111111111111111111111111111111111;
        address replacement = address(new MessageQueue());
        bytes32 adminRole = messageQueue.DEFAULT_ADMIN_ROLE();
        vm.expectRevert(
            abi.encodeWithSelector(IAccessControl.AccessControlUnauthorizedAccount.selector, placeholder, adminRole)
        );
        vm.prank(placeholder);
        messageQueue.upgradeToAndCall(replacement, "");

        IERC20Metadata token = IERC20Metadata(vm.parseJsonAddress(artifact, ".claims.pending21.token"));
        address recipient = vm.parseJsonAddress(artifact, ".claims.pending21.recipient");
        uint256 amount = vm.parseUint(vm.parseJsonString(artifact, ".claims.pending21.amount"));
        uint256 beforeBalance = token.balanceOf(recipient);
        uint256 beforeSupply = token.totalSupply();
        messageQueue.processMessage(claimBlock, 1, 0, claim, new bytes32[](0));
        assertEq(token.balanceOf(recipient), beforeBalance + amount);
        assertEq(token.totalSupply(), beforeSupply + amount);
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MessageAlreadyProcessed.selector, claim.nonce));
        messageQueue.processMessage(claimBlock, 1, 0, claim, new bytes32[](0));

        // The first new claim requires a genuine fixture-signed BEEFY proof and its own maturity.
        VaraMessage memory nextClaim =
            VaraMessage(messageNonce++, deploymentArguments.vftManager, claim.destination, claim.payload);
        uint256 nextBlock = currentBlockNumber++;
        bytes32 nextRoot = nextClaim.hash();
        bytes memory proof = forkBeefy.rootProof(uint32(nextBlock), nextRoot);
        vm.expectRevert(IMessageQueue.InvalidQueueRootProof.selector);
        messageQueue.submitMerkleRoot(nextBlock, nextRoot, "");
        messageQueue.submitMerkleRoot(nextBlock, nextRoot, proof);
        vm.expectRevert(IMessageQueue.MerkleRootDelayNotPassed.selector);
        messageQueue.processMessage(nextBlock, 1, 0, nextClaim, new bytes32[](0));
        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());
        messageQueue.processMessage(nextBlock, 1, 0, nextClaim, new bytes32[](0));
        assertEq(token.balanceOf(recipient), beforeBalance + 2 * amount);
        assertEq(token.totalSupply(), beforeSupply + 2 * amount);
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MessageAlreadyProcessed.selector, nextClaim.nonce));
        messageQueue.processMessage(nextBlock, 1, 0, nextClaim, new bytes32[](0));
        _assertPostMigrationGovernance(replacement);
    }

    function _assertPostMigrationGovernance(address replacement) private {
        bytes32 implementation = vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT);
        uint256 floor = messageQueue.beefyRootMinimum();
        VaraMessage[4] memory messages;
        messages[0] = VaraMessage(
            messageNonce++,
            governancePauser.governance(),
            address(governancePauser),
            PauseProxyMessage(address(messageQueue)).pack()
        );
        messages[1] = VaraMessage(
            messageNonce++,
            governanceAdmin.governance(),
            address(governanceAdmin),
            UpgradeProxyMessage(
                    address(messageQueue),
                    replacement,
                    abi.encodeCall(
                        MessageQueue.migrateToBeefy,
                        (verifier, verifier, messageQueue.maxBlockNumber(), floor, beefyRecoveryTimelock)
                    )
                )
                .pack()
        );
        messages[2] = VaraMessage(
            messageNonce++,
            governanceAdmin.governance(),
            address(governanceAdmin),
            UpgradeProxyMessage(address(messageQueue), replacement, "").pack()
        );
        messages[3] = VaraMessage(
            messageNonce++,
            governancePauser.governance(),
            address(governancePauser),
            UnpauseProxyMessage(address(messageQueue)).pack()
        );
        uint256 firstBlock = currentBlockNumber;
        for (uint256 i; i < messages.length; ++i) {
            uint256 height = currentBlockNumber++;
            bytes32 root = messages[i].hash();
            bytes memory proof = forkBeefy.rootProof(uint32(height), root);
            messageQueue.submitMerkleRoot(height, root, proof);
        }
        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());
        messageQueue.processMessage(firstBlock, 1, 0, messages[0], new bytes32[](0));
        assertTrue(messageQueue.paused());
        // A failed governance upgrade must roll back BOTH the implementation and its consumed nonce.
        vm.expectRevert(bytes4(keccak256("InvalidInitialization()")));
        messageQueue.processMessage(firstBlock + 1, 1, 0, messages[1], new bytes32[](0));
        assertEq(vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT), implementation);
        assertFalse(messageQueue.isProcessed(messages[1].nonce));
        assertEq(messageQueue.verifier(), address(verifier));
        assertEq(messageQueue.beefyRootMinimum(), floor);
        messageQueue.processMessage(firstBlock + 2, 1, 0, messages[2], new bytes32[](0));
        assertEq(
            vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT), bytes32(uint256(uint160(replacement)))
        );
        assertTrue(messageQueue.hasRole(messageQueue.DEFAULT_ADMIN_ROLE(), address(governanceAdmin)));
        assertEq(messageQueue.verifier(), address(verifier));
        assertEq(messageQueue.beefyRootMinimum(), floor);
        messageQueue.processMessage(firstBlock + 3, 1, 0, messages[3], new bytes32[](0));
        assertFalse(messageQueue.paused());
    }
}
