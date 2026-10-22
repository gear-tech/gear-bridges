// Copyright (C) Gear Technologies Inc.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {TimelockController} from "@openzeppelin/contracts/governance/TimelockController.sol";
import {IERC20Metadata} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";
import {CommonBase} from "forge-std/Base.sol";
import {console} from "forge-std/console.sol";
import {Upgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";
import {BridgingPayment} from "src/BridgingPayment.sol";
import {ERC20Manager} from "src/ERC20Manager.sol";
import {GovernanceAdmin} from "src/GovernanceAdmin.sol";
import {GovernancePauser} from "src/GovernancePauser.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {VaraQueueRootVerifier} from "src/VaraQueueRootVerifier.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {CircleToken} from "src/erc20/CircleToken.sol";
import {TetherToken} from "src/erc20/TetherToken.sol";
import {WrappedBitcoin} from "src/erc20/WrappedBitcoin.sol";
import {WrappedEther} from "src/erc20/WrappedEther.sol";
import {WrappedVara} from "src/erc20/WrappedVara.sol";
import {ERC20GearSupply} from "src/erc20/managed/ERC20GearSupply.sol";
import {IERC20Manager} from "src/interfaces/IERC20Manager.sol";
import {IGovernance} from "src/interfaces/IGovernance.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";

struct Overrides {
    address circleToken;
    address tetherToken;
    address wrappedEther;
    address wrappedBitcoin;
}

struct DeploymentArguments {
    uint256 privateKey;
    address deployerAddress;
    string forkUrlOrAlias;
    Overrides overrides;
    bytes32 vftManager;
    bytes32 governanceAdmin;
    bytes32 governancePauser;
    address emergencyStopAdmin;
    address[] emergencyStopObservers;
    uint256 bridgingPaymentFee;
}

abstract contract DeploymentBase is CommonBase {
    error FreshMainnetDeploymentForbidden();
    error DeploymentNotAuthorized();
    DeploymentArguments public deploymentArguments;

    IERC20Metadata public erc20GearSupply;

    IERC20Metadata public circleToken;
    IERC20Metadata public tetherToken;
    IERC20Metadata public wrappedEther;
    IERC20Metadata public wrappedBitcoin;

    WrappedVara public wrappedVara;

    GovernanceAdmin public governanceAdmin;
    GovernancePauser public governancePauser;

    IVerifier public verifier;
    MessageQueue public messageQueue;
    BeefyClient internal beefyClient;
    address internal beefyRecoveryTimelock;
    address internal expectedMessageQueueAddress;
    bool internal messageOnlyBridge;
    bool internal includeOrdinaryGearToken;
    bool internal deployTestTokenProfile;
    bytes32 private approvedDeploymentManifest;
    bytes32 private approvedDeploymentArguments;

    // CREATEs before the queue proxy: both governance contracts, verifier, queue implementation.
    uint256 internal constant GOVERNANCE_TO_QUEUE_PROXY_CREATES = 4;
    // Token profiles add five tokens and the WrappedVara implementation/proxy before governance.
    uint256 internal constant TOKEN_PROFILE_CREATES = 7;

    ERC20Manager public erc20Manager;

    BridgingPayment public bridgingPayment;

    function deployBridgeFromEnvironment() public {
        _requireFreshDeploymentChain();
        require(
            keccak256(bytes(vm.envOr("DEPLOYMENT_MODE", string("")))) == keccak256("fresh-beefy"),
            "DEPLOYMENT_MODE must be fresh-beefy"
        );
        deployTestTokenProfile = vm.envOr("DEPLOY_TEST_TOKENS", false);
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        address deployerAddress = vm.addr(privateKey);

        DeploymentArguments memory args = DeploymentArguments({
            privateKey: privateKey,
            deployerAddress: deployerAddress,
            forkUrlOrAlias: "",
            overrides: _tokenOverridesFromEnvironment(),
            vftManager: vm.envBytes32("VFT_MANAGER"),
            governanceAdmin: vm.envBytes32("GOVERNANCE_ADMIN"),
            governancePauser: vm.envBytes32("GOVERNANCE_PAUSER"),
            emergencyStopAdmin: vm.envAddress("EMERGENCY_STOP_ADMIN"),
            emergencyStopObservers: vm.envAddress("EMERGENCY_STOP_OBSERVERS", ","),
            bridgingPaymentFee: vm.envUint("BRIDGING_PAYMENT_FEE")
        });
        _validateDeploymentArguments(args);
        deploymentArguments = args;
        address predictedQueue = _predictBeefyMessageQueue(args.deployerAddress, args.overrides);
        _deployBeefyClient(args.privateKey, predictedQueue, _loadBeefyBootstrap(predictedQueue));
        deployBridge(args);
    }

    /// @dev Fresh deployment only; retained fork rehearsal belongs to the test subclass.
    function deployBridge(DeploymentArguments memory args) public virtual {
        _requireFreshDeploymentChain();
        require(bytes(args.forkUrlOrAlias).length == 0, "fork rehearsal is test-only");
        deploymentArguments = args;
        _validateDeploymentArguments(deploymentArguments);
        console.log("Deployment arguments:");

        console.log("    deployerAddress:     ", deploymentArguments.deployerAddress);
        console.log("    vftManager:          ", vm.toString(deploymentArguments.vftManager));
        console.log("    governanceAdmin:     ", vm.toString(deploymentArguments.governanceAdmin));
        console.log("    governancePauser:    ", vm.toString(deploymentArguments.governancePauser));
        console.log("    bridgingPaymentFee:  ", deploymentArguments.bridgingPaymentFee, "wei");

        _startDeployment();

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        uint256 chainId = block.chainid;
        address erc20ManagerAddress;
        address governanceAdminAddress;
        address governancePauserAddress;
        if (messageOnlyBridge) {
            uint256 deployerNonce = vm.getNonce(deploymentArguments.deployerAddress);
            governanceAdminAddress = vm.computeCreateAddress(deploymentArguments.deployerAddress, deployerNonce);
            governancePauserAddress = vm.computeCreateAddress(deploymentArguments.deployerAddress, deployerNonce + 1);
        } else {
            console.log("ERC20 tokens:");

            if (!includeOrdinaryGearToken) {
                // Existing deployment profiles retain their verification-only token.
                erc20GearSupply = new ERC20GearSupply(deploymentArguments.deployerAddress, "MyToken", "MTK", 18);
            }

            if (shouldUseOverrides()) {
                circleToken = IERC20Metadata(deploymentArguments.overrides.circleToken);
                tetherToken = IERC20Metadata(deploymentArguments.overrides.tetherToken);
                wrappedEther = IERC20Metadata(deploymentArguments.overrides.wrappedEther);
                wrappedBitcoin = IERC20Metadata(deploymentArguments.overrides.wrappedBitcoin);
            } else {
                deployTestTokens();
            }
            if (includeOrdinaryGearToken) {
                // Reuse the existing CREATE slot; the queue and manager nonce geometry stays unchanged.
                address tokenMinter = vm.computeCreateAddress(
                    deploymentArguments.deployerAddress, vm.getNonce(deploymentArguments.deployerAddress) + 9
                );
                erc20GearSupply = new ERC20GearSupply(tokenMinter, "Bridged Gear Origin Test", "GOT", 12);
            }

            console.log("    USDC:                ", address(circleToken));
            console.log("    USDT:                ", address(tetherToken));
            console.log("    WETH:                ", address(wrappedEther));
            console.log("    WBTC:                ", address(wrappedBitcoin));

            erc20ManagerAddress = vm.computeCreateAddress(
                deploymentArguments.deployerAddress, vm.getNonce(deploymentArguments.deployerAddress) + 8
            );
            governanceAdminAddress = vm.computeCreateAddress(
                deploymentArguments.deployerAddress, vm.getNonce(deploymentArguments.deployerAddress) + 2
            );
            governancePauserAddress = vm.computeCreateAddress(
                deploymentArguments.deployerAddress, vm.getNonce(deploymentArguments.deployerAddress) + 3
            );

            // forge-lint: disable-next-line(todo-comment)
            // TODO: `npm warn exec The following package was not found and will be installed: @openzeppelin/upgrades-core@x.y.z`
            wrappedVara = WrappedVara(
                Upgrades.deployUUPSProxy(
                    "WrappedVara.sol",
                    abi.encodeCall(
                        WrappedVara.initialize,
                        (
                            IGovernance(governanceAdminAddress),
                            IGovernance(governancePauserAddress),
                            ERC20Manager(erc20ManagerAddress)
                        )
                    )
                )
            );

            if (chainId == 1) {
                console.log("    WVARA:               ", address(wrappedVara));
            } else {
                console.log("    WTVARA:              ", address(wrappedVara));
            }

            require(wrappedVara.governanceAdmin() == governanceAdminAddress, "wrapped governance admin mismatch");
            require(wrappedVara.governancePauser() == governancePauserAddress, "wrapped governance pauser mismatch");
            require(wrappedVara.minter() == erc20ManagerAddress, "wrapped minter mismatch");

            console.log();

            //////////////////////////////////////////////////////////////////////////////
        }

        console.log("Bridge governance:");

        uint256 queueDeployerNonce = vm.getNonce(deploymentArguments.deployerAddress);
        address messageQueueAddress = vm.computeCreateAddress(
            deploymentArguments.deployerAddress, queueDeployerNonce + GOVERNANCE_TO_QUEUE_PROXY_CREATES
        );
        if (expectedMessageQueueAddress != address(0)) {
            require(messageQueueAddress == expectedMessageQueueAddress, "predicted queue address mismatch");
        }

        governanceAdmin = new GovernanceAdmin(
            deploymentArguments.governanceAdmin,
            wrappedVara,
            MessageQueue(messageQueueAddress),
            ERC20Manager(erc20ManagerAddress)
        );
        console.log("    GovernanceAdmin:     ", address(governanceAdmin));

        require(governanceAdminAddress == address(governanceAdmin), "governance admin address mismatch");

        governancePauser = new GovernancePauser(
            deploymentArguments.governancePauser,
            wrappedVara,
            MessageQueue(messageQueueAddress),
            ERC20Manager(erc20ManagerAddress)
        );
        console.log("    GovernancePauser:    ", address(governancePauser));

        require(governancePauserAddress == address(governancePauser), "governance pauser address mismatch");

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridge core:");

        verifier = _deployVerifier(chainId, messageQueueAddress);

        console.log("    Verifier:            ", address(verifier));

        // forge-lint: disable-next-line(todo-comment)
        // TODO: `npm warn exec The following package was not found and will be installed: @openzeppelin/upgrades-core@x.y.z`
        bytes memory queueInitialization = abi.encodeCall(
            MessageQueue.initializeBeefy,
            (
                governanceAdmin,
                governancePauser,
                deploymentArguments.emergencyStopAdmin,
                deploymentArguments.emergencyStopObservers,
                verifier,
                beefyRecoveryTimelock
            )
        );
        messageQueue = MessageQueue(Upgrades.deployUUPSProxy("MessageQueue.sol", queueInitialization));
        console.log("    MessageQueue:        ", address(messageQueue));

        require(address(messageQueue) == messageQueueAddress, "queue address mismatch");

        if (messageOnlyBridge) {
            _finishDeployment();
            return;
        }

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridge:");

        IERC20Manager.TokenInfo[] memory tokens = new IERC20Manager.TokenInfo[](includeOrdinaryGearToken ? 6 : 5);

        tokens[0] = IERC20Manager.TokenInfo(address(circleToken), IERC20Manager.TokenType.Ethereum);
        tokens[1] = IERC20Manager.TokenInfo(address(tetherToken), IERC20Manager.TokenType.Ethereum);
        tokens[2] = IERC20Manager.TokenInfo(address(wrappedEther), IERC20Manager.TokenType.Ethereum);
        tokens[3] = IERC20Manager.TokenInfo(address(wrappedVara), IERC20Manager.TokenType.Gear);
        tokens[4] = IERC20Manager.TokenInfo(address(wrappedBitcoin), IERC20Manager.TokenType.Ethereum);
        if (includeOrdinaryGearToken) {
            tokens[5] = IERC20Manager.TokenInfo(address(erc20GearSupply), IERC20Manager.TokenType.Gear);
        }

        erc20Manager = ERC20Manager(
            Upgrades.deployUUPSProxy(
                "ERC20Manager.sol",
                abi.encodeCall(
                    ERC20Manager.initialize,
                    (governanceAdmin, governancePauser, messageQueue, deploymentArguments.vftManager, tokens)
                )
            )
        );
        console.log("    ERC20Manager:        ", address(erc20Manager));

        require(address(erc20Manager) == erc20ManagerAddress, "manager address mismatch");

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridging payment:");

        // forge-lint: disable-next-item(reentrancy-no-eth)
        bridgingPayment = BridgingPayment(erc20Manager.createBridgingPayment(deploymentArguments.bridgingPaymentFee));
        console.log("    BridgingPayment:     ", address(bridgingPayment));


        //////////////////////////////////////////////////////////////////////////////

        _finishDeployment();
    }

    /// @dev Must run before the first broadcast, including BEEFY client/recovery deployment.
    function _validateDeploymentArguments(DeploymentArguments memory args) internal view {
        _requireFreshDeploymentChain();
        bool testChain = block.chainid == 31337 || block.chainid == 11155111 || block.chainid == 560048;
        require(testChain, "unsupported deployment chain");
        require(args.deployerAddress != address(0), "missing deployer");
        if (args.privateKey != 0) require(vm.addr(args.privateKey) == args.deployerAddress, "deployer key mismatch");
        require(args.governanceAdmin != bytes32(0), "missing source governance admin");
        require(args.governancePauser != bytes32(0), "missing source governance pauser");
        require(args.emergencyStopAdmin != address(0), "missing emergency stop admin");
        for (uint256 i; i < args.emergencyStopObservers.length; ++i) {
            require(args.emergencyStopObservers[i] != address(0), "missing emergency stop observer");
        }
        require(!deployTestTokenProfile || testChain, "test tokens require a test chain");
        address[4] memory tokens = [
            args.overrides.circleToken,
            args.overrides.tetherToken,
            args.overrides.wrappedEther,
            args.overrides.wrappedBitcoin
        ];
        uint256 selected;
        for (uint256 i; i < tokens.length; ++i) {
            if (tokens[i] != address(0)) ++selected;
        }
        require(selected == 0 || selected == 4, "partial token overrides");
        if (messageOnlyBridge) {
            require(selected == 0, "message-only profile cannot select tokens");
            return;
        }
        require(args.vftManager != bytes32(0), "missing source token manager");
        if (selected == 0) {
            require(args.privateKey == 0 || deployTestTokenProfile, "explicit test token generation required");
            return;
        }
        for (uint256 i; i < tokens.length; ++i) {
            require(tokens[i].code.length != 0, "token has no code");
            for (uint256 j; j < i; ++j) {
                require(tokens[i] != tokens[j], "duplicate token override");
            }
        }
    }

    function _tokenOverridesFromEnvironment() internal view returns (Overrides memory) {
        return Overrides({
            circleToken: vm.envOr("CIRCLE_TOKEN", address(0)),
            tetherToken: vm.envOr("TETHER_TOKEN", address(0)),
            wrappedEther: vm.envOr("WRAPPED_ETHER", address(0)),
            wrappedBitcoin: vm.envOr("WRAPPED_BITCOIN", address(0))
        });
    }

    struct BeefyBootstrap {
        bytes32 sourceDomain;
        bytes32 bridgeDomain;
        uint64 mmrStartBlock;
        uint64 initialBeefyBlock;
        uint64 initialSourceTimestampMs;
        BeefyClient.ValidatorSet currentSet;
        BeefyClient.ValidatorSet nextSet;
    }

    function _requireFreshDeploymentChain() internal view {
        if (block.chainid == 1) revert FreshMainnetDeploymentForbidden();
    }

    function _loadBeefyBootstrap(address predictedQueue) internal view returns (BeefyBootstrap memory bootstrap) {
        _requireFreshDeploymentChain();
        bootstrap = BeefyBootstrap({
            sourceDomain: vm.envBytes32("BEEFY_SOURCE_DOMAIN"),
            bridgeDomain: vm.envBytes32("BEEFY_BRIDGE_DOMAIN"),
            mmrStartBlock: _envUint64("BEEFY_MMR_START_BLOCK"),
            initialBeefyBlock: _envUint64("BEEFY_INITIAL_BLOCK"),
            initialSourceTimestampMs: _envUint64("BEEFY_INITIAL_SOURCE_TIMESTAMP_MS"),
            currentSet: BeefyClient.ValidatorSet(
                _envUint128("BEEFY_CURRENT_ID"), _envUint128("BEEFY_CURRENT_LENGTH"), vm.envBytes32("BEEFY_CURRENT_ROOT")
            ),
            nextSet: BeefyClient.ValidatorSet(
                _envUint128("BEEFY_NEXT_ID"), _envUint128("BEEFY_NEXT_LENGTH"), vm.envBytes32("BEEFY_NEXT_ROOT")
            )
        });
        _validateBeefyBootstrap(bootstrap, predictedQueue);
    }

    function _validateBeefyBootstrap(BeefyBootstrap memory bootstrap, address predictedQueue) internal view {
        require(bootstrap.sourceDomain != bytes32(0), "missing source domain");
        require(
            predictedQueue != address(0) && bootstrap.bridgeDomain == keccak256(
                abi.encodePacked("vara/gear-eth-bridge-domain/v2", bootstrap.sourceDomain, bytes32(block.chainid), predictedQueue)
            ),
            "configured bridge domain mismatch"
        );
        require(
            bootstrap.mmrStartBlock != 0 && bootstrap.initialBeefyBlock > bootstrap.mmrStartBlock
                && bootstrap.initialBeefyBlock <= type(uint32).max,
            "invalid BEEFY bootstrap height"
        );
        uint256 timestamp = bootstrap.initialSourceTimestampMs;
        require(
            timestamp != 0 && timestamp <= block.timestamp * 1000 + 120_000
                && block.timestamp * 1000 <= timestamp + 86_400_000,
            "invalid BEEFY bootstrap timestamp"
        );
        require(
            bootstrap.currentSet.id < type(uint64).max && bootstrap.nextSet.id == bootstrap.currentSet.id + 1,
            "invalid BEEFY validator ids"
        );
        require(
            bootstrap.currentSet.length != 0 && bootstrap.currentSet.length <= 256
                && bootstrap.nextSet.length != 0 && bootstrap.nextSet.length <= 256
                && bootstrap.currentSet.root != bytes32(0) && bootstrap.nextSet.root != bytes32(0),
            "invalid BEEFY validator sets"
        );
    }

    function _deployBeefyClient(uint256 privateKey, address predictedQueue, BeefyBootstrap memory bootstrap) internal {
        _requireFreshDeploymentChain();
        _validateBeefyBootstrap(bootstrap, predictedQueue);
        _authorizeFreshDeployment(privateKey, predictedQueue, bootstrap);
        address recovery = vm.envAddress("BEEFY_RECOVERY_TIMELOCK");
        require(
            recovery.code.length != 0 && TimelockController(payable(recovery)).getMinDelay() != 0,
            "missing positive-delay BEEFY recovery timelock"
        );
        beefyRecoveryTimelock = recovery;
        expectedMessageQueueAddress = predictedQueue;
        vm.startBroadcast(privateKey);
        beefyClient = new BeefyClient(
            bootstrap.sourceDomain,
            block.chainid,
            predictedQueue,
            bootstrap.mmrStartBlock,
            bootstrap.initialBeefyBlock,
            bootstrap.initialSourceTimestampMs,
            bootstrap.currentSet,
            bootstrap.nextSet
        );
        vm.stopBroadcast();
    }

    function _localDeploymentFixture() internal view virtual returns (bool) {
        return block.chainid == 31337 && vm.envOr("BEEFY_LOCAL_FIXTURE", false);
    }

    function _deploymentArgumentsHash() private view returns (bytes32) {
        DeploymentArguments memory a = deploymentArguments;
        return keccak256(abi.encode(
            a.deployerAddress, a.overrides, a.vftManager, a.governanceAdmin, a.governancePauser,
            a.emergencyStopAdmin, a.emergencyStopObservers, a.bridgingPaymentFee,
            messageOnlyBridge, includeOrdinaryGearToken, deployTestTokenProfile
        ));
    }

    function _authorizeFreshDeployment(uint256 key, address queue, BeefyBootstrap memory bootstrap) internal {
        if (_localDeploymentFixture()) return;
        string memory path = vm.envOr("BEEFY_DEPLOYMENT_MANIFEST", string(""));
        bytes32 approved = vm.envOr("BEEFY_APPROVED_MANIFEST_SHA256", bytes32(0));
        if (bytes(path).length == 0 || approved == bytes32(0)) revert DeploymentNotAuthorized();
        string memory manifest = vm.readFile(path);
        if (sha256(bytes(manifest)) != approved || vm.parseJsonUint(manifest, ".schemaVersion") != 2
            || !vm.parseJsonBool(manifest, ".executionAuthorized")) revert DeploymentNotAuthorized();
        address deployer = vm.addr(key);
        uint256 queueNonce = _beefyMessageQueueNonce(deployer, deploymentArguments.overrides);
        if (vm.parseJsonUint(manifest, ".ethereum.chainId") != block.chainid
            || vm.parseJsonAddress(manifest, ".ethereum.queue") != queue
            || vm.parseJsonAddress(manifest, ".ethereum.client") != vm.computeCreateAddress(deployer, vm.getNonce(deployer))
            || vm.parseJsonBytes32(manifest, ".ethereum.sourceGenesis") != bootstrap.sourceDomain
            || vm.parseJsonBytes32(manifest, ".ethereum.sourceDomain") != bootstrap.sourceDomain
            || vm.parseJsonBytes32(manifest, ".ethereum.bridgeDomain") != bootstrap.bridgeDomain
            || vm.parseJsonUint(manifest, ".ethereum.mmrStartBlock") != bootstrap.mmrStartBlock) revert DeploymentNotAuthorized();
        if (vm.parseJsonAddress(manifest, ".ethereum.verifier") != vm.computeCreateAddress(deployer, queueNonce - 2)
            || vm.parseJsonAddress(manifest, ".ethereum.receiver") != vm.computeCreateAddress(deployer, queueNonce + (messageOnlyBridge ? 1 : 2))
            || vm.parseJsonAddress(manifest, ".execution.authorities.governanceAdmin.address") != vm.computeCreateAddress(deployer, queueNonce - 4)
            || vm.parseJsonAddress(manifest, ".execution.authorities.governancePauser.address") != vm.computeCreateAddress(deployer, queueNonce - 3)
            || vm.parseJsonBytes32(manifest, ".gearManager") != deploymentArguments.vftManager
            || vm.parseJsonBytes32(manifest, ".execution.authorities.governanceAdmin.storage.governance.value") != deploymentArguments.governanceAdmin
            || vm.parseJsonBytes32(manifest, ".execution.authorities.governancePauser.storage.governance.value") != deploymentArguments.governancePauser) revert DeploymentNotAuthorized();
        address recovery = vm.envAddress("BEEFY_RECOVERY_TIMELOCK");
        bytes32 arguments = _deploymentArgumentsHash();
        bytes32 plan = keccak256(abi.encode(
            block.chainid, vm.addr(key), vm.getNonce(vm.addr(key)), queue, bootstrap, arguments,
            recovery, recovery.codehash, TimelockController(payable(recovery)).getMinDelay(),
            keccak256(type(BeefyClient).creationCode), keccak256(type(VaraQueueRootVerifier).creationCode)
        ));
        if (vm.parseJsonBytes32(manifest, ".execution.deploymentPlanHash") != plan) revert DeploymentNotAuthorized();
        // The existing native Beacon/MPT verifier authenticates source, governance,
        // complete roles/Safe state and planned-address absence before the first CREATE.
        // No signer is passed. Missing evidence/binary/approval is a hard HOLD.
        string[] memory command = new string[](12);
        command[0] = vm.envString("BEEFY_RELAY_BIN");
        command[1] = "authorize-deployment";
        command[2] = "--source-rpc";
        command[3] = vm.envString("SOURCE_RPC");
        command[4] = "--witness-rpc";
        command[5] = vm.envString("WITNESS_RPC");
        command[6] = "--ethereum-rpc";
        command[7] = vm.envString("ETH_RPC");
        command[8] = "--deployment-manifest";
        command[9] = path;
        command[10] = "--plan-hash";
        command[11] = vm.toString(plan);
        bytes memory result = vm.ffi(command);
        if (result.length != 32 || abi.decode(result, (bytes32)) != approved) revert DeploymentNotAuthorized();
        approvedDeploymentManifest = approved;
        approvedDeploymentArguments = arguments;
    }

    function _startDeployment() internal virtual {
        if (!_localDeploymentFixture()) {
            if (approvedDeploymentManifest == bytes32(0)
                || approvedDeploymentManifest != sha256(bytes(vm.readFile(vm.envString("BEEFY_DEPLOYMENT_MANIFEST"))))
                || approvedDeploymentArguments != _deploymentArgumentsHash()) revert DeploymentNotAuthorized();
        }
        require(deploymentArguments.privateKey != 0, "missing deployment key");
        require(address(beefyClient) != address(0), "missing BEEFY client");
        vm.startBroadcast(deploymentArguments.privateKey);
    }

    function _finishDeployment() internal virtual {
        if (!messageOnlyBridge) {
            printContractInfo("WrappedVara", address(wrappedVara), Upgrades.getImplementationAddress(address(wrappedVara)));
            printContractInfo("MessageQueue", address(messageQueue), Upgrades.getImplementationAddress(address(messageQueue)));
            printContractInfo("ERC20Manager", address(erc20Manager), Upgrades.getImplementationAddress(address(erc20Manager)));
        }
        vm.stopBroadcast();
    }

    function _deployVerifier(uint256 chainId, address messageQueueAddress) internal virtual returns (IVerifier) {
        require(address(beefyClient) != address(0), "missing BEEFY client");
        return new VaraQueueRootVerifier(beefyClient, messageQueueAddress, chainId);
    }

    function _predictBeefyMessageQueue(address deployer, Overrides memory tokens) internal view returns (address) {
        return vm.computeCreateAddress(deployer, _beefyMessageQueueNonce(deployer, tokens));
    }

    function _beefyMessageQueueNonce(address deployer, Overrides memory tokens) internal view returns (uint256) {
        // The client is deployed first; ordinary Gear tokens reuse the same token CREATE slots.
        uint256 createsBeforeQueue = 1 + GOVERNANCE_TO_QUEUE_PROXY_CREATES;
        if (!messageOnlyBridge) createsBeforeQueue += TOKEN_PROFILE_CREATES;
        if (!messageOnlyBridge && tokens.circleToken != address(0)) createsBeforeQueue -= 4;
        return vm.getNonce(deployer) + createsBeforeQueue;
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

    function deployTestTokens() internal {
        circleToken = new CircleToken(deploymentArguments.deployerAddress);
        tetherToken = new TetherToken(deploymentArguments.deployerAddress);
        wrappedEther = new WrappedEther();
        wrappedBitcoin = new WrappedBitcoin(deploymentArguments.deployerAddress);
    }

    function shouldUseOverrides() public view returns (bool) {
        return deploymentArguments.overrides.circleToken != address(0)
            && deploymentArguments.overrides.tetherToken != address(0)
            && deploymentArguments.overrides.wrappedEther != address(0)
            && deploymentArguments.overrides.wrappedBitcoin != address(0);
    }

    function printContractInfo(string memory contractName, address contractAddress, address expectedImplementation)
        public
        view
    {
        console.log("================================================================================================");
        console.log("[ CONTRACT  ]", contractName);
        console.log("[ ADDRESS   ]", contractAddress);
        if (expectedImplementation != address(0)) {
            console.log("[ IMPL ADDR ]", expectedImplementation);
            console.log(
                "[ PROXY VERIFICATION ] Click \"Is this a proxy?\" on Etherscan to be able read and write as proxy."
            );
            console.log("                       Alternatively, run the following curl request.");
            console.log("```");
            uint256 chainId = block.chainid;
            console.log("curl \\");
            console.log(string.concat("    --data \"address=", vm.toString(contractAddress), "\" \\"));
            console.log(
                string.concat("    --data \"expectedimplementation=", vm.toString(expectedImplementation), "\" \\")
            );
            console.log(
                string.concat(
                    "    \"https://api.etherscan.io/v2/api?chainid=",
                    vm.toString(chainId),
                    "&module=contract&action=verifyproxycontract&apikey=$ETHERSCAN_API_KEY\""
                )
            );
            console.log("```");
        }
        console.log("================================================================================================");
        console.log();
    }
}
