// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {TimelockController} from "@openzeppelin/contracts/governance/TimelockController.sol";
import {Test} from "forge-std/Test.sol";
import {BeefyLocal} from "script/BeefyLocal.s.sol";
import {BeefyTokens} from "script/BeefyTokens.s.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {CircleToken} from "src/erc20/CircleToken.sol";
import {TetherToken} from "src/erc20/TetherToken.sol";
import {WrappedBitcoin} from "src/erc20/WrappedBitcoin.sol";
import {WrappedEther} from "src/erc20/WrappedEther.sol";
import {DeploymentBase, DeploymentArguments, Overrides} from "script/DeploymentBase.sol";
import {Base} from "test/Base.sol";

contract DeploymentPreflightHarness is Base {
    function preflight(DeploymentArguments memory args, bool messageOnly, bool generateTokens) external {
        messageOnlyBridge = messageOnly;
        deployTestTokenProfile = generateTokens;
        _validateDeploymentArguments(args);
    }

    function predict(address deployer, Overrides memory tokens) external view returns (address) {
        return _predictBeefyMessageQueue(deployer, tokens);
    }

    function bootstrap(address queue) external view returns (BeefyBootstrap memory) {
        return _loadBeefyBootstrap(queue);
    }

    function deployClient(uint256 privateKey, address queue, BeefyBootstrap memory config) external returns (BeefyClient) {
        _deployBeefyClient(privateKey, queue, config);
        return beefyClient;
    }
}

contract DeploymentPreflightTest is Test {
    DeploymentPreflightHarness private deployment;
    uint256 private constant DEPLOYER_KEY = 1;

    function setUp() public {
        vm.chainId(31337);
        deployment = new DeploymentPreflightHarness();
        for (uint160 i = 1; i <= 4; ++i) {
            vm.etch(address(0x1000 + i), hex"00");
        }
        _clearTokens();
    }

    function test_BeefyPreflightRejectsEveryZeroIdentityBeforeCreate() public {
        for (uint256 i; i < 6; ++i) {
            DeploymentArguments memory args = _arguments();
            if (i == 0) args.deployerAddress = address(0);
            if (i == 1) args.governanceAdmin = bytes32(0);
            if (i == 2) args.governancePauser = bytes32(0);
            if (i == 3) args.vftManager = bytes32(0);
            if (i == 4) args.emergencyStopAdmin = address(0);
            if (i == 5) args.emergencyStopObservers[0] = address(0);
            _rejectBeforeCreate(args);
        }
    }

    function test_BeefyPreflightRejectsDeployerKeyMismatch() public {
        DeploymentArguments memory args = _arguments();
        args.deployerAddress = address(0xBEEF);
        _rejectBeforeCreate(args);
    }

    function test_BeefyPreflightMessageOnlyAllowsNoManagerButRequiresGovernance() public {
        DeploymentArguments memory args = _arguments();
        args.vftManager = bytes32(0);
        deployment.preflight(args, true, false);
        args.governanceAdmin = bytes32(0);
        vm.expectRevert();
        deployment.preflight(args, true, false);
        args.governanceAdmin = bytes32(uint256(1));
        args.governancePauser = bytes32(0);
        vm.expectRevert();
        deployment.preflight(args, true, false);
    }

    function test_BeefyPreflightRejectsEveryPartialSubsetOnEverySupportedChain() public {
        uint256[4] memory chains = [uint256(1), 31337, 11155111, 560048];
        for (uint256 c; c < chains.length; ++c) {
            vm.chainId(chains[c]);
            for (uint256 mask = 1; mask < 15; ++mask) {
                DeploymentArguments memory args = _arguments();
                args.overrides = _tokens(mask);
                uint64 nonce = vm.getNonce(vm.addr(DEPLOYER_KEY));
                vm.expectRevert();
                deployment.deployBridge(args);
                assertEq(vm.getNonce(vm.addr(DEPLOYER_KEY)), nonce);
            }
        }
    }

    function test_BeefyPreflightRejectsEveryDuplicatePairAndCodeLessToken() public {
        for (uint256 i; i < 4; ++i) {
            for (uint256 j; j < i; ++j) {
                address[4] memory duplicateTokens = _addresses();
                duplicateTokens[i] = duplicateTokens[j];
                DeploymentArguments memory duplicateArgs = _arguments();
                duplicateArgs.overrides =
                    Overrides(duplicateTokens[0], duplicateTokens[1], duplicateTokens[2], duplicateTokens[3]);
                _rejectBeforeCreate(duplicateArgs);
            }
            address[4] memory tokens = _addresses();
            tokens[i] = address(0xBEEF);
            DeploymentArguments memory args = _arguments();
            args.overrides = Overrides(tokens[0], tokens[1], tokens[2], tokens[3]);
            _rejectBeforeCreate(args);
        }
    }

    function test_BeefyPreflightMainnetForbidsEvenCanonicalOverrides() public {
        vm.chainId(1);
        _rejectBeforeCreate(_arguments());
        DeploymentArguments memory args = _arguments();
        args.overrides = _canonicalTokens();
        vm.etch(args.overrides.circleToken, hex"00");
        vm.etch(args.overrides.tetherToken, hex"00");
        vm.etch(args.overrides.wrappedEther, hex"00");
        vm.etch(args.overrides.wrappedBitcoin, hex"00");
        _rejectBeforeCreate(args);
        vm.expectRevert(DeploymentBase.FreshMainnetDeploymentForbidden.selector);
        deployment.preflight(args, true, false);
    }

    function test_BeefyPreflightMainnetEnvironmentAndSharedClientRejectBeforeCreate() public {
        _configureFreshEnvironment();
        vm.chainId(1);
        uint64 nonce = vm.getNonce(vm.addr(DEPLOYER_KEY));
        uint64 harnessNonce = vm.getNonce(address(deployment));
        vm.expectRevert(DeploymentBase.FreshMainnetDeploymentForbidden.selector);
        deployment.deployBridgeFromEnvironment();
        assertEq(vm.getNonce(vm.addr(DEPLOYER_KEY)), nonce);
        assertEq(vm.getNonce(address(deployment)), harnessNonce);
        DeploymentBase.BeefyBootstrap memory bootstrap;
        vm.expectRevert(DeploymentBase.FreshMainnetDeploymentForbidden.selector);
        deployment.deployClient(DEPLOYER_KEY, address(0x1234), bootstrap);
        assertEq(vm.getNonce(vm.addr(DEPLOYER_KEY)), nonce);
        assertEq(vm.getNonce(address(deployment)), harnessNonce);
    }

    function test_BeefyPreflightMissingApprovalRejectsBeforeAnyCreate() public {
        _configureFreshEnvironment();
        vm.setEnv("BEEFY_LOCAL_FIXTURE", "false");
        vm.setEnv("BEEFY_DEPLOYMENT_MANIFEST", "");
        vm.setEnv("BEEFY_APPROVED_MANIFEST_SHA256", vm.toString(bytes32(0)));
        uint64 nonce = vm.getNonce(vm.addr(DEPLOYER_KEY));
        uint64 harnessNonce = vm.getNonce(address(deployment));
        vm.expectRevert(DeploymentBase.DeploymentNotAuthorized.selector);
        deployment.deployBridgeFromEnvironment();
        assertEq(vm.getNonce(vm.addr(DEPLOYER_KEY)), nonce);
        assertEq(vm.getNonce(address(deployment)), harnessNonce);
    }

    function test_BeefyPreflightFalseAndConflictingApprovalRejectBeforeCreate() public {
        _configureFreshEnvironment();
        vm.setEnv("BEEFY_LOCAL_FIXTURE", "false");
        vm.createDir("broadcast", true);
        string memory path = "broadcast/deployment-preflight-approval.json";
        string memory manifest = '{"schemaVersion":2,"executionAuthorized":false}';
        vm.writeFile(path, manifest);
        vm.setEnv("BEEFY_DEPLOYMENT_MANIFEST", path);
        uint64 nonce = vm.getNonce(vm.addr(DEPLOYER_KEY));
        uint64 harnessNonce = vm.getNonce(address(deployment));
        vm.setEnv("BEEFY_APPROVED_MANIFEST_SHA256", vm.toString(bytes32(uint256(1))));
        vm.expectRevert(DeploymentBase.DeploymentNotAuthorized.selector);
        deployment.deployBridgeFromEnvironment();
        vm.setEnv("BEEFY_APPROVED_MANIFEST_SHA256", vm.toString(sha256(bytes(manifest))));
        vm.expectRevert(DeploymentBase.DeploymentNotAuthorized.selector);
        deployment.deployBridgeFromEnvironment();
        assertEq(vm.getNonce(vm.addr(DEPLOYER_KEY)), nonce);
        assertEq(vm.getNonce(address(deployment)), harnessNonce);
        vm.removeFile(path);
    }

    function test_BeefyPreflightDeploysTheValidatedBootstrapWithoutReloadingEnvironment() public {
        _configureFreshEnvironment();
        address queue = deployment.predict(vm.addr(DEPLOYER_KEY), _tokens(0));
        DeploymentBase.BeefyBootstrap memory config = deployment.bootstrap(queue);
        vm.setEnv("BEEFY_CURRENT_ROOT", vm.toString(bytes32(0)));
        vm.setEnv("BEEFY_SOURCE_DOMAIN", vm.toString(bytes32(0)));
        BeefyClient client = deployment.deployClient(DEPLOYER_KEY, queue, config);
        assertEq(client.sourceDomain(), config.sourceDomain);
        assertEq(client.bridgeDomain(), config.bridgeDomain);
        assertEq(client.destinationQueue(), queue);
        (uint128 id, uint128 length, bytes32 root) = client.currentValidatorSet();
        assertEq(id, config.currentSet.id);
        assertEq(length, config.currentSet.length);
        assertEq(root, config.currentSet.root);
    }

    function test_BeefyPreflightExplicitGenerationIsTestChainOnly() public {
        DeploymentArguments memory args = _arguments();
        vm.expectRevert();
        deployment.preflight(args, false, false);
        uint256[3] memory chains = [uint256(31337), 11155111, 560048];
        for (uint256 i; i < chains.length; ++i) {
            vm.chainId(chains[i]);
            deployment.preflight(args, false, true);
        }
        vm.chainId(1);
        vm.expectRevert();
        deployment.preflight(args, false, true);
    }

    function test_BeefyPreflightUnitFixturesRetainGeneratedTokens() public {
        DeploymentArguments memory args = _arguments();
        args.privateKey = 0;
        deployment.preflight(args, false, false);
    }

    function test_BeefyPreflightRejectsUnsupportedChain() public {
        vm.chainId(42);
        _rejectBeforeCreate(_arguments());
    }

    function test_BeefyPreflightCompleteTokensAcceptedWithoutGeneration() public {
        DeploymentArguments memory args = _arguments();
        args.overrides = _tokens(15);
        uint256[3] memory chains = [uint256(31337), 11155111, 560048];
        for (uint256 i; i < chains.length; ++i) {
            vm.chainId(chains[i]);
            deployment.preflight(args, false, false);
        }
        vm.expectRevert();
        deployment.preflight(args, true, false);
    }

    function test_BeefyPreflightCompleteOverridesAreActuallySelectedInUnitFixture() public {
        DeploymentArguments memory args = _arguments();
        args.privateKey = 0;
        args.overrides = Overrides(
            address(new CircleToken(args.deployerAddress)),
            address(new TetherToken(args.deployerAddress)),
            address(new WrappedEther()),
            address(new WrappedBitcoin(args.deployerAddress))
        );
        deployment.deployBridge(args);
        assertEq(address(deployment.circleToken()), args.overrides.circleToken);
        assertEq(address(deployment.tetherToken()), args.overrides.tetherToken);
        assertEq(address(deployment.wrappedEther()), args.overrides.wrappedEther);
        assertEq(address(deployment.wrappedBitcoin()), args.overrides.wrappedBitcoin);
    }

    function test_BeefyPreflightQueuePredictionAccountsForExistingTokens() public view {
        address deployer = vm.addr(DEPLOYER_KEY);
        assertEq(
            deployment.predict(deployer, _tokens(0)), vm.computeCreateAddress(deployer, vm.getNonce(deployer) + 12)
        );
        assertEq(
            deployment.predict(deployer, _tokens(15)), vm.computeCreateAddress(deployer, vm.getNonce(deployer) + 8)
        );
    }

    function test_BeefyPreflightRejectsNonFreshModesBeforeCreate() public {
        string[3] memory modes = [string(""), string("retained-beefy-migration"), string("unsupported")];
        for (uint256 i; i < modes.length; ++i) {
            vm.setEnv("DEPLOYMENT_MODE", modes[i]);
            uint64 nonce = vm.getNonce(vm.addr(DEPLOYER_KEY));
            vm.expectRevert();
            deployment.deployBridgeFromEnvironment();
            assertEq(vm.getNonce(vm.addr(DEPLOYER_KEY)), nonce);
        }
    }

    function test_BeefyPreflightGenericRequiresEveryBootstrapFieldBeforeCreate() public {
        _configureFreshEnvironment();
        string[11] memory keys = _bootstrapKeys();
        address queue = vm.computeCreateAddress(vm.addr(DEPLOYER_KEY), vm.getNonce(vm.addr(DEPLOYER_KEY)) + 12);
        for (uint256 i; i < keys.length; ++i) {
            _configureBootstrap(queue);
            vm.setEnv(keys[i], "");
            uint64 nonce = vm.getNonce(vm.addr(DEPLOYER_KEY));
            vm.expectRevert();
            deployment.deployBridgeFromEnvironment();
            assertEq(vm.getNonce(vm.addr(DEPLOYER_KEY)), nonce);
        }
    }

    function test_BeefyPreflightGenericInvalidBootstrapRejectsBeforeCreate() public {
        _configureFreshEnvironment();
        string[11] memory keys = _bootstrapKeys();
        string memory zero = vm.toString(bytes32(0));
        string[11] memory invalid = [
            zero,
            zero,
            string("0"),
            string("10"),
            string("0"),
            string("18446744073709551615"),
            string("0"),
            zero,
            string("5"),
            string("257"),
            zero
        ];
        uint64 nonce = vm.getNonce(vm.addr(DEPLOYER_KEY));
        address queue = vm.computeCreateAddress(vm.addr(DEPLOYER_KEY), nonce + 12);
        for (uint256 i; i < keys.length; ++i) {
            _configureBootstrap(queue);
            vm.setEnv(keys[i], invalid[i]);
            vm.expectRevert();
            deployment.deployBridgeFromEnvironment();
            assertEq(vm.getNonce(vm.addr(DEPLOYER_KEY)), nonce);
        }
    }

    function test_BeefyPreflightGenericRequiresPositiveDelayRecoveryBeforeCreate() public {
        _configureFreshEnvironment();
        address[] memory proposers = new address[](1);
        proposers[0] = address(this);
        address zeroDelay = address(new TimelockController(0, proposers, proposers, address(0)));
        address[3] memory invalid = [address(0), address(0xBEEF), zeroDelay];
        uint64 nonce = vm.getNonce(vm.addr(DEPLOYER_KEY));
        for (uint256 i; i < invalid.length; ++i) {
            vm.setEnv("BEEFY_RECOVERY_TIMELOCK", vm.toString(invalid[i]));
            vm.expectRevert();
            deployment.deployBridgeFromEnvironment();
            assertEq(vm.getNonce(vm.addr(DEPLOYER_KEY)), nonce);
        }
    }

    function test_BeefyPreflightGenericRejectsEveryPartialSubsetBeforeClientCreate() public {
        uint256[4] memory chains = [uint256(1), 31337, 11155111, 560048];
        for (uint256 c; c < chains.length; ++c) {
            vm.chainId(chains[c]);
            _configureFreshEnvironment();
            vm.setEnv("DEPLOY_TEST_TOKENS", "false");
            for (uint256 mask = 1; mask < 15; ++mask) {
                Overrides memory tokens = _tokens(mask);
                vm.setEnv("CIRCLE_TOKEN", vm.toString(tokens.circleToken));
                vm.setEnv("TETHER_TOKEN", vm.toString(tokens.tetherToken));
                vm.setEnv("WRAPPED_ETHER", vm.toString(tokens.wrappedEther));
                vm.setEnv("WRAPPED_BITCOIN", vm.toString(tokens.wrappedBitcoin));
                uint64 nonce = vm.getNonce(vm.addr(DEPLOYER_KEY));
                vm.expectRevert();
                deployment.deployBridgeFromEnvironment();
                assertEq(vm.getNonce(vm.addr(DEPLOYER_KEY)), nonce);
            }
        }
    }

    function test_BeefyPreflightGenericMissingOrZeroIdentityNeverDeploysClient() public {
        string[4] memory keys = ["VFT_MANAGER", "GOVERNANCE_ADMIN", "GOVERNANCE_PAUSER", "EMERGENCY_STOP_ADMIN"];
        for (uint256 missing; missing < keys.length; ++missing) {
            for (uint256 empty; empty < 2; ++empty) {
                _configureFreshEnvironment();
                vm.setEnv(
                    keys[missing], empty == 0 ? "" : (missing < 3 ? vm.toString(bytes32(0)) : vm.toString(address(0)))
                );
                uint64 identityNonce = vm.getNonce(vm.addr(DEPLOYER_KEY));
                vm.expectRevert();
                deployment.deployBridgeFromEnvironment();
                assertEq(vm.getNonce(vm.addr(DEPLOYER_KEY)), identityNonce);
            }
        }
        // An empty observer list is allowed; a selected zero-address observer is not.
        _configureFreshEnvironment();
        vm.setEnv("EMERGENCY_STOP_OBSERVERS", vm.toString(address(0)));
        uint64 nonce = vm.getNonce(vm.addr(DEPLOYER_KEY));
        vm.expectRevert();
        deployment.deployBridgeFromEnvironment();
        assertEq(vm.getNonce(vm.addr(DEPLOYER_KEY)), nonce);
    }

    function test_BeefyPreflightTokensMissingOrZeroIdentityNeverDeploysClient() public {
        string[5] memory keys = [
            "GEAR_VFT_MANAGER",
            "GEAR_GOVERNANCE_ADMIN",
            "GEAR_GOVERNANCE_PAUSER",
            "EMERGENCY_STOP_ADMIN",
            "EMERGENCY_STOP_OBSERVER"
        ];
        vm.setEnv("PRIVATE_KEY", "1");
        vm.setEnv("EXPECTED_DEPLOYER_NONCE", vm.toString(vm.getNonce(vm.addr(DEPLOYER_KEY))));
        vm.setEnv("BRIDGING_PAYMENT_FEE", "1");
        address[] memory proposers = new address[](1);
        proposers[0] = address(this);
        TimelockController recovery = new TimelockController(1 hours, proposers, proposers, address(0));
        vm.setEnv("BEEFY_RECOVERY_TIMELOCK", vm.toString(address(recovery)));
        _configureBootstrap(vm.computeCreateAddress(vm.addr(DEPLOYER_KEY), vm.getNonce(vm.addr(DEPLOYER_KEY)) + 12));
        for (uint256 missing; missing < keys.length; ++missing) {
            for (uint256 empty; empty < 2; ++empty) {
                for (uint256 i; i < keys.length; ++i) {
                    vm.setEnv(keys[i], i < 3 ? vm.toString(bytes32(uint256(1))) : vm.toString(address(1)));
                }
                vm.setEnv(
                    keys[missing], empty == 0 ? "" : (missing < 3 ? vm.toString(bytes32(0)) : vm.toString(address(0)))
                );
                BeefyTokens script = new BeefyTokens();
                uint64 nonce = vm.getNonce(vm.addr(DEPLOYER_KEY));
                vm.expectRevert();
                script.run();
                assertEq(vm.getNonce(vm.addr(DEPLOYER_KEY)), nonce);
            }
        }
    }

    function test_BeefyPreflightInvalidBootstrapNeverCreatesLocalRecovery() public {
        vm.setEnv("PRIVATE_KEY", "1");
        vm.setEnv("LOCAL_RECOVERY_PRIVATE_KEY", "2");
        string[11] memory keys = _bootstrapKeys();
        string memory zero = vm.toString(bytes32(0));
        string[11] memory invalid = [
            zero,
            zero,
            string("0"),
            string("10"),
            string("0"),
            string("18446744073709551615"),
            string("0"),
            zero,
            string("5"),
            string("257"),
            zero
        ];
        BeefyLocal script = new BeefyLocal();
        uint64 deployerNonce = vm.getNonce(vm.addr(1));
        uint64 recoveryNonce = vm.getNonce(vm.addr(2));
        address queue = vm.computeCreateAddress(vm.addr(1), deployerNonce + 12);
        for (uint256 i; i < keys.length; ++i) {
            _configureBootstrap(queue);
            vm.setEnv(keys[i], invalid[i]);
            vm.expectRevert();
            script.run();
            assertEq(vm.getNonce(vm.addr(1)), deployerNonce);
            assertEq(vm.getNonce(vm.addr(2)), recoveryNonce);
        }
    }

    function _rejectBeforeCreate(DeploymentArguments memory args) private {
        uint64 nonce = vm.getNonce(vm.addr(DEPLOYER_KEY));
        vm.expectRevert();
        deployment.deployBridge(args);
        assertEq(vm.getNonce(vm.addr(DEPLOYER_KEY)), nonce);
    }

    function _arguments() private returns (DeploymentArguments memory) {
        address[] memory observers = new address[](1);
        observers[0] = address(0x1234);
        return DeploymentArguments({
            privateKey: DEPLOYER_KEY,
            deployerAddress: vm.addr(DEPLOYER_KEY),
            forkUrlOrAlias: "",
            overrides: Overrides(address(0), address(0), address(0), address(0)),
            vftManager: bytes32(uint256(1)),
            governanceAdmin: bytes32(uint256(2)),
            governancePauser: bytes32(uint256(3)),
            emergencyStopAdmin: address(0x1234),
            emergencyStopObservers: observers,
            bridgingPaymentFee: 1
        });
    }

    function _addresses() private pure returns (address[4] memory) {
        return [address(0x1001), address(0x1002), address(0x1003), address(0x1004)];
    }

    function _tokens(uint256 mask) private pure returns (Overrides memory) {
        return Overrides(
            (mask & 1) != 0 ? address(0x1001) : address(0),
            (mask & 2) != 0 ? address(0x1002) : address(0),
            (mask & 4) != 0 ? address(0x1003) : address(0),
            (mask & 8) != 0 ? address(0x1004) : address(0)
        );
    }

    function _canonicalTokens() private pure returns (Overrides memory) {
        return Overrides(
            0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48,
            0xdAC17F958D2ee523a2206206994597C13D831ec7,
            0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2,
            0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599
        );
    }

    function _bootstrapKeys() private pure returns (string[11] memory) {
        return [
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
    }

    function _configureBootstrap(address queue) private {
        vm.warp(1_000_000);
        bytes32 source = bytes32(uint256(0x11));
        vm.setEnv("BEEFY_SOURCE_DOMAIN", vm.toString(source));
        vm.setEnv(
            "BEEFY_BRIDGE_DOMAIN",
            vm.toString(
                keccak256(abi.encodePacked("vara/gear-eth-bridge-domain/v2", source, bytes32(block.chainid), queue))
            )
        );
        vm.setEnv("BEEFY_MMR_START_BLOCK", "10");
        vm.setEnv("BEEFY_INITIAL_BLOCK", "12");
        vm.setEnv("BEEFY_INITIAL_SOURCE_TIMESTAMP_MS", vm.toString(block.timestamp * 1000));
        vm.setEnv("BEEFY_CURRENT_ID", "5");
        vm.setEnv("BEEFY_CURRENT_LENGTH", "2");
        vm.setEnv("BEEFY_CURRENT_ROOT", vm.toString(bytes32(uint256(0x22))));
        vm.setEnv("BEEFY_NEXT_ID", "6");
        vm.setEnv("BEEFY_NEXT_LENGTH", "2");
        vm.setEnv("BEEFY_NEXT_ROOT", vm.toString(bytes32(uint256(0x33))));
    }

    function _configureFreshEnvironment() private {
        vm.setEnv("BEEFY_LOCAL_FIXTURE", "true");
        _clearTokens();
        vm.setEnv("DEPLOYMENT_MODE", "fresh-beefy");
        vm.setEnv("DEPLOY_TEST_TOKENS", "true");
        vm.setEnv("PRIVATE_KEY", vm.toString(DEPLOYER_KEY));
        vm.setEnv("VFT_MANAGER", vm.toString(bytes32(uint256(1))));
        vm.setEnv("GOVERNANCE_ADMIN", vm.toString(bytes32(uint256(2))));
        vm.setEnv("GOVERNANCE_PAUSER", vm.toString(bytes32(uint256(3))));
        vm.setEnv("EMERGENCY_STOP_ADMIN", vm.toString(address(0x1234)));
        vm.setEnv("EMERGENCY_STOP_OBSERVERS", vm.toString(address(0x5678)));
        vm.setEnv("BRIDGING_PAYMENT_FEE", "1");
        address[] memory proposers = new address[](1);
        proposers[0] = address(this);
        TimelockController recovery = new TimelockController(1 hours, proposers, proposers, address(0));
        vm.setEnv("BEEFY_RECOVERY_TIMELOCK", vm.toString(address(recovery)));
        _configureBootstrap(vm.computeCreateAddress(vm.addr(DEPLOYER_KEY), vm.getNonce(vm.addr(DEPLOYER_KEY)) + 12));
    }

    function _clearTokens() private {
        vm.setEnv("CIRCLE_TOKEN", vm.toString(address(0)));
        vm.setEnv("TETHER_TOKEN", vm.toString(address(0)));
        vm.setEnv("WRAPPED_ETHER", vm.toString(address(0)));
        vm.setEnv("WRAPPED_BITCOIN", vm.toString(address(0)));
    }
}
