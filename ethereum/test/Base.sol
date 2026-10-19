// Copyright (C) Gear Technologies Inc.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {TimelockController} from "@openzeppelin/contracts/governance/TimelockController.sol";
import {IERC20Metadata} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";
import {StdAssertions} from "forge-std/StdAssertions.sol";
import {StdChains} from "forge-std/StdChains.sol";
import {StdCheats} from "forge-std/StdCheats.sol";
import {StdInvariant} from "forge-std/StdInvariant.sol";
import {StdUtils} from "forge-std/StdUtils.sol";
import {console} from "forge-std/console.sol";
import {Upgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";
import {RetainedBridgeMigration} from "script/RetainedBridgeMigration.sol";
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
import {BeefyClientMock} from "src/mocks/BeefyClientMock.sol";
import {MessageHandlerMock} from "src/mocks/MessageHandlerMock.sol";
import {NewImplementationMock} from "src/mocks/NewImplementationMock.sol";
import {BaseConstants} from "test/BaseConstants.sol";

import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {VaraBridgeMetadata} from "src/beefy/VaraBridgeMetadata.sol";
import {VaraMessage} from "src/interfaces/IMessageQueue.sol";
import {Hasher} from "src/libraries/Hasher.sol";
import {ERC20ManagerPacker, TransferMessage} from "src/libraries/packing/ERC20ManagerPacker.sol";
import {
    GovernancePacker,
    PauseProxyMessage,
    UnpauseProxyMessage,
    UpgradeProxyMessage
} from "src/libraries/packing/GovernancePacker.sol";
import {BeefyFixtureTest} from "test/BeefyInterop.t.sol";

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

abstract contract Base is RetainedBridgeMigration, StdAssertions, StdChains, StdCheats, StdInvariant, StdUtils {
    using Hasher for VaraMessage;

    using GovernancePacker for UpgradeProxyMessage;
    using GovernancePacker for PauseProxyMessage;
    using GovernancePacker for UnpauseProxyMessage;

    using ERC20ManagerPacker for TransferMessage;

    uint256 public messageNonce;
    uint256 public currentBlockNumber = 1;

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
    ForkBeefyFixture internal forkBeefy;

    // CREATEs before the queue proxy: both governance contracts, verifier, queue implementation.
    uint256 internal constant GOVERNANCE_TO_QUEUE_PROXY_CREATES = 4;
    // Token profiles add five tokens and the WrappedVara implementation/proxy before governance.
    uint256 internal constant TOKEN_PROFILE_CREATES = 7;

    ERC20Manager public erc20Manager;

    BridgingPayment public bridgingPayment;

    MessageHandlerMock public messageHandlerMock;
    NewImplementationMock public newImplementationMock;

    function deployBridgeFromConstants() public {
        deployBridgeFromConstants(BaseConstants.DEPLOYER_ADDRESS, "");
    }

    function deployBridgeFromConstants(address deployerAddress, string memory forkUrlOrAlias) public {
        address[] memory emergencyStopObservers = new address[](2);

        emergencyStopObservers[0] = BaseConstants.EMERGENCY_STOP_OBSERVER1;
        emergencyStopObservers[1] = BaseConstants.EMERGENCY_STOP_OBSERVER2;

        deployBridge(
            DeploymentArguments({
                privateKey: 0,
                deployerAddress: deployerAddress,
                forkUrlOrAlias: forkUrlOrAlias,
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
    }

    function deployBridgeFromEnvironment() public {
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
        address predictedQueue = _predictBeefyMessageQueue(args.deployerAddress, args.overrides);
        _deployBeefyClient(args.privateKey, predictedQueue);
        deployBridge(args);
    }

    /// forge-lint: disable-next-item(cyclomatic-complexity)
    function deployBridge(DeploymentArguments memory _deploymentArguments) public {
        deploymentArguments = _deploymentArguments;

        bool isTest = deploymentArguments.privateKey == 0;
        bool isFork = bytes(deploymentArguments.forkUrlOrAlias).length != 0;

        if (isFork) {
            // Fork rehearsal only: never broadcast fixture governance or change token authorities.
            require(isTest && !messageOnlyBridge && !includeOrdinaryGearToken, "invalid migration profile");
            string memory legacyArtifact = vm.readFile("test/fixtures/mainnet-legacy-queue.json");
            uint256 height = vm.parseJsonUint(legacyArtifact, ".ethereumBlock");
            vm.createSelectFork(deploymentArguments.forkUrlOrAlias, height);
            require(block.chainid == 1, "migration fixture requires mainnet");
            vm.roll(height + 1);
            assertEq(blockhash(height), vm.parseJsonBytes32(legacyArtifact, ".ethereumBlockHash"));
            vm.roll(height);

            messageQueue = MessageQueue(vm.parseJsonAddress(legacyArtifact, ".queue"));
            address legacyImplementation =
                address(uint160(uint256(vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT))));
            assertEq(legacyImplementation, vm.parseJsonAddress(legacyArtifact, ".implementation"));
            assertEq(legacyImplementation.codehash, vm.parseJsonBytes32(legacyArtifact, ".codeHash"));
            assertEq(
                uint64(
                    uint256(vm.load(address(messageQueue), vm.parseJsonBytes32(legacyArtifact, ".initializableSlot")))
                ),
                6
            );
            governanceAdmin = GovernanceAdmin(messageQueue.governanceAdmin());
            governancePauser = GovernancePauser(messageQueue.governancePauser());
            assertEq(address(governanceAdmin.messageQueue()), address(messageQueue));
            assertEq(address(governancePauser.messageQueue()), address(messageQueue));
            wrappedVara = governanceAdmin.wrappedVara();
            erc20Manager = governanceAdmin.erc20Manager();
            verifier = IVerifier(messageQueue.verifier());
            bridgingPayment = BridgingPayment(erc20Manager.bridgingPayments()[0]);
            address[] memory tokens = erc20Manager.tokens();
            assertEq(tokens.length, 5);
            circleToken = IERC20Metadata(tokens[0]);
            tetherToken = IERC20Metadata(tokens[1]);
            wrappedEther = IERC20Metadata(tokens[2]);
            assertEq(tokens[3], address(wrappedVara));
            wrappedBitcoin = IERC20Metadata(tokens[4]);
            deploymentArguments.overrides = Overrides(tokens[0], tokens[1], tokens[2], tokens[4]);
            bytes32[] memory managers = erc20Manager.vftManagers();
            deploymentArguments.vftManager = managers[managers.length - 1];
            deploymentArguments.governanceAdmin = governanceAdmin.governance();
            deploymentArguments.governancePauser = governancePauser.governance();
            deploymentArguments.emergencyStopAdmin = messageQueue.emergencyStopAdmin();
            deploymentArguments.emergencyStopObservers = messageQueue.emergencyStopObservers();
            deploymentArguments.bridgingPaymentFee = bridgingPayment.fee();
            assertFalse(messageQueue.paused());
            assertFalse(messageQueue.isChallengingRoot());
            assertFalse(messageQueue.isEmergencyStopped());

            uint256 genesis = messageQueue.genesisBlock();
            uint256 firstGovernanceBlock = messageQueue.maxBlockNumber() + 1;
            uint256 legacyMax = firstGovernanceBlock + 1;
            uint256 floor = legacyMax + 1;
            require(floor < type(uint32).max - 2, "migration height overflow");
            address newImplementation1 = address(new MessageQueue());
            forkBeefy = new ForkBeefyFixture(messageQueue, uint64(floor), false);
            beefyClient = forkBeefy.client();
            IVerifier nextVerifier = forkBeefy.verifier();
            address[] memory proposers = new address[](1);
            proposers[0] = address(this);
            beefyRecoveryTimelock = address(new TimelockController(1 hours, proposers, proposers, address(0)));
            MigrationPlan memory plan =
                _forkMigrationPlan(legacyImplementation, newImplementation1, nextVerifier, legacyMax, floor);
            // Accept a fixture-signed MMR checkpoint before changing the queue's verifier.
            forkBeefy.rootProof(uint32(floor), keccak256("migration bootstrap"));

            messageNonce = 100_000_000;
            VaraMessage[2] memory messages;
            messages[0] = VaraMessage(
                messageNonce++,
                governancePauser.governance(),
                address(governancePauser),
                PauseProxyMessage(address(messageQueue)).pack()
            );
            messages[1] = VaraMessage(
                messageNonce++, governanceAdmin.governance(), address(governanceAdmin), retainedUpgradePayload(plan)
            );
            // Only pause and atomic upgrade are authorized before the cutover.
            for (uint256 i; i < messages.length; ++i) {
                assertFalse(messageQueue.isProcessed(messages[i].nonce));
                _registerForkGovernanceRoot(
                    plan,
                    firstGovernanceBlock + i,
                    messages[i],
                    i == 0 ? GovernanceAction.Pause : GovernanceAction.Upgrade
                );
            }
            vm.warp(block.timestamp + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());
            executeRetainedGovernance(
                plan, _forkDelivery(firstGovernanceBlock, messages[0], ""), GovernanceAction.Pause
            );
            assertTrue(messageQueue.paused());
            vm.record();
            executeRetainedGovernance(
                plan, _forkDelivery(firstGovernanceBlock + 1, messages[1], ""), GovernanceAction.Upgrade
            );
            (, bytes32[] memory writes) = vm.accesses(address(messageQueue));
            bytes32 initializerSlot = vm.parseJsonBytes32(legacyArtifact, ".initializableSlot");
            bytes32 processedSlot = keccak256(abi.encode(messages[1].nonce, uint256(13)));
            for (uint256 i; i < writes.length; ++i) {
                assertTrue(
                    writes[i] == ERC1967Utils.IMPLEMENTATION_SLOT || writes[i] == initializerSlot
                        || writes[i] == bytes32(uint256(5)) || writes[i] == bytes32(uint256(14))
                        || writes[i] == bytes32(uint256(16)) || writes[i] == processedSlot,
                    "migration overwrote retained queue state"
                );
            }
            assertEq(uint64(uint256(vm.load(address(messageQueue), initializerSlot))), 8);
            assertEq(
                address(uint160(uint256(vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT)))),
                newImplementation1
            );
            verifier = nextVerifier;
            assertEq(messageQueue.verifier(), address(verifier));
            assertEq(messageQueue.beefyRootMinimum(), floor);
            assertEq(messageQueue.beefyRecoveryTimelock(), beefyRecoveryTimelock);
            assertEq(messageQueue.genesisBlock(), genesis);
            assertEq(messageQueue.maxBlockNumber(), legacyMax);
            assertTrue(messageQueue.paused());
            assertTrue(messageQueue.hasRole(messageQueue.DEFAULT_ADMIN_ROLE(), address(governanceAdmin)));
            assertTrue(messageQueue.hasRole(messageQueue.PAUSER_ROLE(), address(governancePauser)));
            verifyRetainedPoststate(plan, true);
            for (uint256 i; i < messages.length; ++i) {
                assertTrue(messageQueue.isProcessed(messages[i].nonce));
                assertEq(messageQueue.getMerkleRoot(firstGovernanceBlock + i), messages[i].hash());
            }
            // Test-only finality observations: model the canonical head 64 slots after each inclusion.
            EmptyBarrier memory barrier;
            barrier.upgradeReadback.state =
                FinalizedPin(vm.getBlockNumber(), keccak256("fixture upgrade readback"), vm.getBlockTimestamp());
            vm.roll(vm.getBlockNumber() + 64);
            vm.warp(vm.getBlockTimestamp() + 768);
            barrier.upgradeReadback.head = FinalizedPin(
                vm.getBlockNumber(), keccak256("fixture upgrade observation head"), vm.getBlockTimestamp()
            );
            barrier.upgradeReadback.observedAt = vm.getBlockTimestamp();
            vm.roll(vm.getBlockNumber() + 1);
            vm.warp(barrier.upgradeReadback.observedAt + 121);
            barrier.sourceBlock = floor;
            barrier.proof = forkBeefy.rootProofForEpoch(uint32(floor), bytes32(0), 1);
            messageQueue.submitEmptyQueueProgress(floor, barrier.proof);
            barrier.publication.state =
                FinalizedPin(vm.getBlockNumber(), keccak256("fixture empty publication"), vm.getBlockTimestamp());
            vm.roll(vm.getBlockNumber() + 64);
            vm.warp(vm.getBlockTimestamp() + 768);
            barrier.publication.head =
                FinalizedPin(vm.getBlockNumber(), keccak256("fixture empty observation head"), vm.getBlockTimestamp());
            barrier.publication.observedAt = vm.getBlockTimestamp();
            vm.roll(vm.getBlockNumber() + 1);
            vm.warp(barrier.publication.observedAt + 121);
            VaraMessage memory unpause = VaraMessage(
                messageNonce++,
                governancePauser.governance(),
                address(governancePauser),
                UnpauseProxyMessage(address(messageQueue)).pack()
            );
            GovernanceDelivery memory delivery =
                _forkDelivery(floor + 1, unpause, forkBeefy.rootProofForEpoch(uint32(floor + 1), unpause.hash(), 2));
            registerRetainedUnpauseRoot(plan, delivery, barrier);
            vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());
            executeRetainedUnpause(plan, delivery, barrier);
            verifyRetainedPoststate(plan, false);
            assertTrue(messageQueue.isProcessed(unpause.nonce));
            currentBlockNumber = floor + 2;
            messageQueueAssertions(address(messageQueue));
            erc20ManagerAssertions(address(erc20Manager));
            bridgingPaymentAssertions();
            return;
        }

        _validateDeploymentArguments(deploymentArguments);
        console.log("Deployment arguments:");

        console.log("    deployerAddress:     ", deploymentArguments.deployerAddress);
        console.log("    vftManager:          ", vm.toString(deploymentArguments.vftManager));
        console.log("    governanceAdmin:     ", vm.toString(deploymentArguments.governanceAdmin));
        console.log("    governancePauser:    ", vm.toString(deploymentArguments.governancePauser));
        console.log("    bridgingPaymentFee:  ", deploymentArguments.bridgingPaymentFee, "wei");

        if (isTest) {
            // forge-lint: disable-next-item(reentrancy-no-eth)
            vm.warp(vm.unixTime() / 1000);
            address[] memory proposers = new address[](1);
            proposers[0] = address(this);
            beefyRecoveryTimelock = address(new TimelockController(1 hours, proposers, proposers, address(0)));
            // forge-lint: disable-next-item(reentrancy-no-eth)
            vm.deal(deploymentArguments.deployerAddress, BaseConstants.DEPLOYER_INITIAL_BALANCE);
            // forge-lint: disable-next-item(reentrancy-no-eth)
            vm.startPrank(deploymentArguments.deployerAddress, deploymentArguments.deployerAddress);
        } else {
            // forge-lint: disable-next-item(reentrancy-no-eth)
            vm.startBroadcast(deploymentArguments.privateKey);
        }

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

            assertEq(wrappedVara.governanceAdmin(), governanceAdminAddress);
            assertEq(wrappedVara.governancePauser(), governancePauserAddress);
            assertEq(wrappedVara.minter(), erc20ManagerAddress);

            console.log();

            //////////////////////////////////////////////////////////////////////////////
        }

        console.log("Bridge governance:");

        uint256 queueDeployerNonce = vm.getNonce(deploymentArguments.deployerAddress);
        address messageQueueAddress = vm.computeCreateAddress(
            deploymentArguments.deployerAddress, queueDeployerNonce + GOVERNANCE_TO_QUEUE_PROXY_CREATES
        );
        if (expectedMessageQueueAddress != address(0)) {
            assertEq(messageQueueAddress, expectedMessageQueueAddress, "predicted queue address mismatch");
        }

        governanceAdmin = new GovernanceAdmin(
            deploymentArguments.governanceAdmin,
            wrappedVara,
            MessageQueue(messageQueueAddress),
            ERC20Manager(erc20ManagerAddress)
        );
        console.log("    GovernanceAdmin:     ", address(governanceAdmin));

        assertEq(governanceAdminAddress, address(governanceAdmin));

        governancePauser = new GovernancePauser(
            deploymentArguments.governancePauser,
            wrappedVara,
            MessageQueue(messageQueueAddress),
            ERC20Manager(erc20ManagerAddress)
        );
        console.log("    GovernancePauser:    ", address(governancePauser));

        assertEq(governancePauserAddress, address(governancePauser));

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridge core:");

        verifier = _deployVerifier(isTest, chainId, messageQueueAddress);

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

        messageQueueAssertions(messageQueueAddress);

        if (messageOnlyBridge) {
            messageHandlerMock = new MessageHandlerMock();
            if (isTest) {
                vm.stopPrank();
            } else {
                vm.stopBroadcast();
            }
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

        erc20ManagerAssertions(erc20ManagerAddress);

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridging payment:");

        // forge-lint: disable-next-item(reentrancy-no-eth)
        bridgingPayment = BridgingPayment(erc20Manager.createBridgingPayment(deploymentArguments.bridgingPaymentFee));
        console.log("    BridgingPayment:     ", address(bridgingPayment));

        bridgingPaymentAssertions();

        //////////////////////////////////////////////////////////////////////////////

        if (isTest) {
            console.log();

            console.log("Test specific:");

            messageHandlerMock = new MessageHandlerMock();
            console.log("    MessageHandlerMock:  ", address(messageHandlerMock));

            newImplementationMock = new NewImplementationMock();
            console.log("    NewImplementationMock:", address(newImplementationMock));
        } else {
            console.log();

            console.log("Script specific:");

            printContractInfo(
                "WrappedVara", address(wrappedVara), Upgrades.getImplementationAddress(address(wrappedVara))
            );
            printContractInfo(
                "MessageQueue", address(messageQueue), Upgrades.getImplementationAddress(address(messageQueue))
            );
            printContractInfo(
                "ERC20Manager", address(erc20Manager), Upgrades.getImplementationAddress(address(erc20Manager))
            );
        }

        //////////////////////////////////////////////////////////////////////////////

        if (isTest) {
            vm.stopPrank();
        } else {
            vm.stopBroadcast();
        }
    }

    /// @dev Only the synthetic source-governance root is mocked; no bytecode or authority is replaced.
    function _registerForkGovernanceRoot(
        MigrationPlan memory plan,
        uint256 height,
        VaraMessage memory message,
        GovernanceAction action
    ) private {
        bytes32 root = message.hash();
        uint256[] memory inputs = new uint256[](2);
        inputs[0] = uint256(root) >> 64;
        inputs[1] = (uint256(uint64(uint256(root))) << 128) | (height << 96);
        vm.mockCall(
            address(verifier),
            abi.encodeWithSignature("safeVerifyProof(bytes,uint256[])", bytes(""), inputs),
            abi.encode(true)
        );
        registerRetainedGovernanceRoot(plan, _forkDelivery(height, message, ""), action);
        vm.clearMockedCalls();
    }

    function _forkDelivery(uint256 height, VaraMessage memory message, bytes memory proof)
        private
        pure
        returns (GovernanceDelivery memory)
    {
        return GovernanceDelivery(height, message.hash(), proof, 1, 0, message, new bytes32[](0));
    }

    function _forkMigrationPlan(
        address legacyImplementation,
        address implementation,
        IVerifier nextVerifier,
        uint256 legacyMax,
        uint256 floor
    ) internal view returns (MigrationPlan memory p) {
        p.chainId = block.chainid;
        p.ethereumGenesis = 0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3;
        p.queue = Identity(address(messageQueue), address(messageQueue).codehash);
        p.legacyImplementation = Identity(legacyImplementation, legacyImplementation.codehash);
        p.legacyVerifier = Identity(address(verifier), address(verifier).codehash);
        p.governanceAdmin = Identity(address(governanceAdmin), address(governanceAdmin).codehash);
        p.governancePauser = Identity(address(governancePauser), address(governancePauser).codehash);
        p.manager = Identity(address(erc20Manager), address(erc20Manager).codehash);
        p.wrappedVara = Identity(address(wrappedVara), address(wrappedVara).codehash);
        address[] memory tokens = erc20Manager.tokens();
        p.tokens = new Identity[](tokens.length);
        p.retainedState = new StatePin[](tokens.length * 2 + 2);
        for (uint256 i; i < tokens.length; ++i) {
            p.tokens[i] = Identity(tokens[i], tokens[i].codehash);
            IERC20Metadata token = IERC20Metadata(tokens[i]);
            p.retainedState[2 * i] = StatePin(
                tokens[i],
                abi.encodeCall(token.balanceOf, (address(erc20Manager))),
                keccak256(abi.encode(token.balanceOf(address(erc20Manager))))
            );
            p.retainedState[2 * i + 1] =
                StatePin(tokens[i], abi.encodeCall(token.totalSupply, ()), keccak256(abi.encode(token.totalSupply())));
        }
        p.retainedState[tokens.length * 2] = StatePin(
            address(messageQueue), abi.encodeCall(messageQueue.isProcessed, (836)), keccak256(abi.encode(true))
        );
        p.retainedState[tokens.length * 2 + 1] = StatePin(
            address(messageQueue), abi.encodeCall(messageQueue.isProcessed, (864)), keccak256(abi.encode(true))
        );
        address[] memory payments = erc20Manager.bridgingPayments();
        p.payments = new Identity[](payments.length);
        for (uint256 i; i < payments.length; ++i) {
            p.payments[i] = Identity(payments[i], payments[i].codehash);
        }
        p.sourceAdmin = governanceAdmin.governance();
        p.sourcePauser = governancePauser.governance();
        p.sourceManagers = erc20Manager.vftManagers();
        p.emergencyAdmin = messageQueue.emergencyStopAdmin();
        p.emergencyObservers = messageQueue.emergencyStopObservers();
        p.genesisBlock = messageQueue.genesisBlock();
        p.legacyMax = legacyMax;
        p.firstBeefyBlock = floor;
        p.legacyInitializerVersion = 6;
        p.implementation = Identity(implementation, implementation.codehash);
        p.client = Identity(address(beefyClient), address(beefyClient).codehash);
        p.verifier = Identity(address(nextVerifier), address(nextVerifier).codehash);
        p.bootstrap.sourceDomain = beefyClient.sourceDomain();
        p.bootstrap.mmrStartBlock = beefyClient.mmrStartBlock();
        p.bootstrap.initialBeefyBlock = beefyClient.latestBeefyBlock();
        p.bootstrap.initialSourceTimestampMs = beefyClient.lastAuthenticatedSourceTimestampMs();
        (uint128 id, uint128 length, bytes32 root,) = beefyClient.currentValidatorSet();
        p.bootstrap.currentSet = BeefyClient.ValidatorSet(id, length, root);
        (id, length, root,) = beefyClient.nextValidatorSet();
        p.bootstrap.nextSet = BeefyClient.ValidatorSet(id, length, root);
        p.recovery = Identity(beefyRecoveryTimelock, beefyRecoveryTimelock.codehash);
        p.recoveryDelay = 1 hours;
        p.recoveryRoles = new RolePin[](1);
        p.recoveryRoles[0] =
            RolePin(TimelockController(payable(beefyRecoveryTimelock)).PROPOSER_ROLE(), address(this), true);
        p.sourceCutApproval = keccak256("fork fixture only; not mainnet source-cut approval");
        p.outstandingUnpauseInventory = keccak256("fork fixture only; not production inventory approval");
        ProxyPin[] memory proxies = new ProxyPin[]((tokens.length + 1) * 2);
        bytes32[2] memory slots = [ERC1967Utils.IMPLEMENTATION_SLOT, keccak256("org.zeppelinos.proxy.implementation")];
        uint256 count;
        for (uint256 i; i <= tokens.length; ++i) {
            address proxy = i == tokens.length ? address(erc20Manager) : tokens[i];
            for (uint256 j; j < slots.length; ++j) {
                address target = address(uint160(uint256(vm.load(proxy, slots[j]))));
                if (target != address(0)) {
                    proxies[count++] = ProxyPin(proxy, slots[j], Identity(target, target.codehash));
                }
            }
        }
        assembly ("memory-safe") { mstore(proxies, count) }
        p.retainedProxies = proxies;
    }

    /// @dev Must run before the first broadcast, including BEEFY client/recovery deployment.
    function _validateDeploymentArguments(DeploymentArguments memory args) internal view {
        bool testChain = block.chainid == 31337 || block.chainid == 11155111 || block.chainid == 560048;
        require(block.chainid == 1 || testChain, "unsupported deployment chain");
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
            require(args.privateKey == 0 || block.chainid != 1, "mainnet requires canonical token overrides");
            require(args.privateKey == 0 || deployTestTokenProfile, "explicit test token generation required");
            return;
        }
        for (uint256 i; i < tokens.length; ++i) {
            require(tokens[i].code.length != 0, "token has no code");
            for (uint256 j; j < i; ++j) {
                require(tokens[i] != tokens[j], "duplicate token override");
            }
        }
        if (block.chainid == 1) {
            require(
                tokens[0] == 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48
                    && tokens[1] == 0xdAC17F958D2ee523a2206206994597C13D831ec7
                    && tokens[2] == 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2
                    && tokens[3] == 0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599,
                "noncanonical mainnet token overrides"
            );
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

    function _validateBeefyBootstrap(address predictedQueue) internal view {
        bytes32 sourceDomain = vm.envBytes32("BEEFY_SOURCE_DOMAIN");
        require(sourceDomain != bytes32(0), "missing source domain");
        require(
            predictedQueue != address(0)
                && vm.envBytes32("BEEFY_BRIDGE_DOMAIN")
                    == keccak256(
                        abi.encodePacked(
                            "vara/gear-eth-bridge-domain/v2", sourceDomain, bytes32(block.chainid), predictedQueue
                        )
                    ),
            "configured bridge domain mismatch"
        );
        uint64 start = _envUint64("BEEFY_MMR_START_BLOCK");
        uint64 initial = _envUint64("BEEFY_INITIAL_BLOCK");
        uint64 timestamp = _envUint64("BEEFY_INITIAL_SOURCE_TIMESTAMP_MS");
        require(start != 0 && initial > start && initial <= type(uint32).max, "invalid BEEFY bootstrap height");
        require(
            timestamp != 0 && uint256(timestamp) <= block.timestamp * 1000 + 120_000
                && block.timestamp * 1000 <= uint256(timestamp) + 86_400_000,
            "invalid BEEFY bootstrap timestamp"
        );
        uint128 currentId = _envUint128("BEEFY_CURRENT_ID");
        uint128 nextId = _envUint128("BEEFY_NEXT_ID");
        require(currentId < type(uint64).max && nextId == currentId + 1, "invalid BEEFY validator ids");
        uint128 currentLength = _envUint128("BEEFY_CURRENT_LENGTH");
        uint128 nextLength = _envUint128("BEEFY_NEXT_LENGTH");
        require(
            currentLength != 0 && currentLength <= 256 && nextLength != 0 && nextLength <= 256
                && vm.envBytes32("BEEFY_CURRENT_ROOT") != bytes32(0) && vm.envBytes32("BEEFY_NEXT_ROOT") != bytes32(0),
            "invalid BEEFY validator sets"
        );
    }

    function _validateBeefyRecoveryTimelock() internal view {
        address recovery = vm.envAddress("BEEFY_RECOVERY_TIMELOCK");
        require(
            recovery.code.length != 0 && TimelockController(payable(recovery)).getMinDelay() != 0,
            "missing positive-delay BEEFY recovery timelock"
        );
    }

    function _predictBeefyMessageQueue(address deployer, Overrides memory tokens) internal view returns (address) {
        // The client is deployed first; ordinary Gear tokens reuse the same token CREATE slots.
        uint256 createsBeforeQueue = 1 + GOVERNANCE_TO_QUEUE_PROXY_CREATES;
        if (!messageOnlyBridge) createsBeforeQueue += TOKEN_PROFILE_CREATES;
        if (!messageOnlyBridge && tokens.circleToken != address(0)) createsBeforeQueue -= 4;
        return vm.computeCreateAddress(deployer, vm.getNonce(deployer) + createsBeforeQueue);
    }

    function _deployBeefyClient(uint256 privateKey, address predictedQueue) internal {
        _validateBeefyBootstrap(predictedQueue);
        _validateBeefyRecoveryTimelock();
        beefyRecoveryTimelock = vm.envAddress("BEEFY_RECOVERY_TIMELOCK");
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
        vm.stopBroadcast();
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

    function _deployVerifier(bool isTest, uint256 chainId, address messageQueueAddress)
        internal
        virtual
        returns (IVerifier)
    {
        if (isTest) {
            forkBeefy = new ForkBeefyFixture(MessageQueue(messageQueueAddress), 1, true);
            beefyClient = forkBeefy.client();
            return forkBeefy.verifier();
        }
        require(address(beefyClient) != address(0), "missing BEEFY client");
        return new VaraQueueRootVerifier(beefyClient, messageQueueAddress, chainId);
    }

    function _rootProof(uint256 sourceBlock, bytes32 root) internal returns (bytes memory) {
        require(sourceBlock <= type(uint32).max, "source block exceeds uint32");
        return forkBeefy.rootProof(uint32(sourceBlock), root);
    }

    function deployTestTokens() public {
        circleToken = new CircleToken(deploymentArguments.deployerAddress);
        tetherToken = new TetherToken(deploymentArguments.deployerAddress);
        wrappedEther = new WrappedEther();
        wrappedBitcoin = new WrappedBitcoin(deploymentArguments.deployerAddress);
    }

    function shouldUseOverrides() public view returns (bool) {
        return deploymentArguments.overrides.circleToken != BaseConstants.ZERO_ADDRESS
            && deploymentArguments.overrides.tetherToken != BaseConstants.ZERO_ADDRESS
            && deploymentArguments.overrides.wrappedEther != BaseConstants.ZERO_ADDRESS
            && deploymentArguments.overrides.wrappedBitcoin != BaseConstants.ZERO_ADDRESS;
    }

    function messageQueueAssertions(address messageQueueAddress) public view {
        assertEq(messageQueueAddress, address(messageQueue));
        assertEq(messageQueue.governanceAdmin(), address(governanceAdmin));
        assertEq(messageQueue.governancePauser(), address(governancePauser));
        assertEq(messageQueue.emergencyStopAdmin(), deploymentArguments.emergencyStopAdmin);
        address[] memory emergencyStopObservers = messageQueue.emergencyStopObservers();
        assertEq(emergencyStopObservers.length, deploymentArguments.emergencyStopObservers.length);
        for (uint256 i = 0; i < emergencyStopObservers.length; i++) {
            assertEq(emergencyStopObservers[i], deploymentArguments.emergencyStopObservers[i]);
        }
        assertEq(messageQueue.verifier(), address(verifier));
        assertEq(messageQueue.beefyRecoveryTimelock(), beefyRecoveryTimelock);
        assertEq(messageQueue.isChallengingRoot(), false);
        assertEq(messageQueue.isEmergencyStopped(), false);
        if (bytes(deploymentArguments.forkUrlOrAlias).length == 0) {
            assertEq(messageQueue.genesisBlock(), 0);
            assertEq(messageQueue.maxBlockNumber(), 0);
        }
    }

    function erc20ManagerAssertions(address erc20ManagerAddress) public view {
        assertEq(erc20ManagerAddress, address(erc20Manager));
        assertEq(erc20Manager.governanceAdmin(), address(governanceAdmin));
        assertEq(erc20Manager.governancePauser(), address(governancePauser));
        assertEq(erc20Manager.messageQueue(), address(messageQueue));
        bool isMainnet = block.chainid == 1;
        uint256 expectedVftManagers = bytes(deploymentArguments.forkUrlOrAlias).length != 0 && isMainnet ? 3 : 1;
        assertEq(erc20Manager.totalVftManagers(), expectedVftManagers);
        bytes32[] memory vftManagers1 = erc20Manager.vftManagers();
        assertEq(vftManagers1.length, expectedVftManagers);
        assertEq(vftManagers1[expectedVftManagers - 1], deploymentArguments.vftManager);
        bytes32[] memory vftManagers2 = erc20Manager.vftManagers(expectedVftManagers, 1);
        assertEq(vftManagers2.length, 0);
        bytes32[] memory vftManagers3 = erc20Manager.vftManagers(expectedVftManagers - 1, 1);
        assertEq(vftManagers3.length, 1);
        assertEq(vftManagers3[0], deploymentArguments.vftManager);
        bytes32[] memory vftManagers4 = erc20Manager.vftManagers(0, 5);
        assertEq(vftManagers4.length, expectedVftManagers);
        assertEq(vftManagers4[expectedVftManagers - 1], deploymentArguments.vftManager);
        assertTrue(erc20Manager.isVftManager(deploymentArguments.vftManager));
        uint256 expectedTokens = includeOrdinaryGearToken ? 6 : 5;
        assertEq(erc20Manager.totalTokens(), expectedTokens);
        address[] memory tokens1 = erc20Manager.tokens();
        assertEq(tokens1.length, expectedTokens);
        assertEq(tokens1[0], address(circleToken));
        assertEq(tokens1[1], address(tetherToken));
        assertEq(tokens1[2], address(wrappedEther));
        assertEq(tokens1[3], address(wrappedVara));
        assertEq(tokens1[4], address(wrappedBitcoin));
        address[] memory tokens2 = erc20Manager.tokens(expectedTokens, 5);
        assertEq(tokens2.length, 0);
        address[] memory tokens3 = erc20Manager.tokens(0, 2);
        assertEq(tokens3.length, 2);
        assertEq(tokens3[0], address(circleToken));
        assertEq(tokens3[1], address(tetherToken));
        address[] memory tokens4 = erc20Manager.tokens(2, 3);
        assertEq(tokens4.length, 3);
        assertEq(tokens4[0], address(wrappedEther));
        assertEq(tokens4[1], address(wrappedVara));
        assertEq(tokens4[2], address(wrappedBitcoin));
        address[] memory tokens5 = erc20Manager.tokens(0, 6);
        assertEq(tokens5.length, expectedTokens);
        assertEq(tokens5[0], address(circleToken));
        assertEq(tokens5[1], address(tetherToken));
        assertEq(tokens5[2], address(wrappedEther));
        assertEq(tokens5[3], address(wrappedVara));
        assertEq(tokens5[4], address(wrappedBitcoin));
        assertTrue(erc20Manager.getTokenType(address(circleToken)) == IERC20Manager.TokenType.Ethereum);
        assertTrue(erc20Manager.getTokenType(address(tetherToken)) == IERC20Manager.TokenType.Ethereum);
        assertTrue(erc20Manager.getTokenType(address(wrappedEther)) == IERC20Manager.TokenType.Ethereum);
        assertTrue(erc20Manager.getTokenType(address(wrappedVara)) == IERC20Manager.TokenType.Gear);
        assertTrue(erc20Manager.getTokenType(address(wrappedBitcoin)) == IERC20Manager.TokenType.Ethereum);
        assertTrue(erc20Manager.getTokenType(address(0)) == IERC20Manager.TokenType.Unknown);
        if (includeOrdinaryGearToken) {
            assertEq(tokens1[5], address(erc20GearSupply));
            assertEq(tokens5[5], address(erc20GearSupply));
            assertEq(ERC20GearSupply(address(erc20GearSupply)).owner(), erc20ManagerAddress);
            assertTrue(erc20Manager.getTokenType(address(erc20GearSupply)) == IERC20Manager.TokenType.Gear);
        }
    }

    function bridgingPaymentAssertions() public view {
        assertEq(bridgingPayment.erc20Manager(), address(erc20Manager));
        assertEq(erc20Manager.totalBridgingPayments(), 1);
        address[] memory bridgingPayments1 = erc20Manager.bridgingPayments();
        assertEq(bridgingPayments1.length, 1);
        assertEq(bridgingPayments1[0], address(bridgingPayment));
        address[] memory bridgingPayments2 = erc20Manager.bridgingPayments(1, 1);
        assertEq(bridgingPayments2.length, 0);
        address[] memory bridgingPayments3 = erc20Manager.bridgingPayments(0, 1);
        assertEq(bridgingPayments3.length, 1);
        assertEq(bridgingPayments3[0], address(bridgingPayment));
        address[] memory bridgingPayments4 = erc20Manager.bridgingPayments(0, 5);
        assertEq(bridgingPayments4.length, 1);
        assertEq(bridgingPayments4[0], address(bridgingPayment));
        assertFalse(erc20Manager.isBridgingPayment(address(0)));
        assertTrue(erc20Manager.isBridgingPayment(address(bridgingPayment)));
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

/// @dev Full-bridge fixture: checkpoint injection is restricted to explicit unit setup.
contract ForkBeefyFixture is BeefyFixtureTest {
    BeefyClient public immutable client;
    VaraQueueRootVerifier public immutable verifier;
    bool private immutable mockCommitments;

    constructor(MessageQueue queue, uint64 floor, bool mockCommitments_) {
        mockCommitments = mockCommitments_;
        fixtures = vm.readFile("test/fixtures/beefy-interop.json");
        bytes32 authorityRoot = fixtureHash(0, "authorityRoot");
        uint128 count = uint128(fixtureUint(0, "validatorCount"));
        BeefyClient.ValidatorSet memory current = BeefyClient.ValidatorSet(0, count, authorityRoot);
        BeefyClient.ValidatorSet memory next = BeefyClient.ValidatorSet(1, count, authorityRoot);
        client = mockCommitments_
            ? new BeefyClientMock(
                fixtureHash(0, "sourceDomain"),
                block.chainid,
                address(queue),
                floor,
                floor + 1,
                uint64(block.timestamp * 1000),
                current,
                next
            )
            : new BeefyClient(
                fixtureHash(0, "sourceDomain"),
                block.chainid,
                address(queue),
                floor,
                floor + 1,
                uint64(block.timestamp * 1000),
                current,
                next
            );
        verifier = new VaraQueueRootVerifier(client, address(queue), block.chainid);
    }

    function _submitQueueCheckpoint(
        BeefyClient targetClient,
        uint64 anchorBlock,
        bytes32 anchorRoot,
        BeefyClient.MMRLeaf memory freshnessLeaf,
        VaraBridgeMetadata.Snapshot memory freshnessSnapshot,
        bytes32 historicalLeafHash
    ) internal override {
        if (mockCommitments) {
            BeefyClientMock(address(targetClient))
                .setCheckpoint(anchorBlock, anchorRoot, freshnessSnapshot.sourceTimestampMs);
        } else {
            super._submitQueueCheckpoint(
                targetClient, anchorBlock, anchorRoot, freshnessLeaf, freshnessSnapshot, historicalLeafHash
            );
        }
    }

    function rootProof(uint32 sourceBlock, bytes32 root) external returns (bytes memory) {
        uint64 anchor = client.latestBeefyBlock() + 1;
        if (anchor <= uint64(sourceBlock) + 1) anchor = uint64(sourceBlock) + 2;
        require(anchor <= type(uint32).max, "fixture anchor overflow");
        return encodeProof(
            customQueueProofFor(
                client, sourceBlock, anchor, root, uint64(block.timestamp * 1000), uint64(block.timestamp * 1000)
            )
        );
    }

    function rootProofForEpoch(uint32 sourceBlock, bytes32 root, uint64 epoch) external returns (bytes memory) {
        uint64 anchor = client.latestBeefyBlock() + 1;
        if (anchor <= uint64(sourceBlock) + 1) anchor = uint64(sourceBlock) + 2;
        require(anchor <= type(uint32).max, "fixture anchor overflow");
        return encodeProof(
            customQueueProofForEpoch(
                client, sourceBlock, anchor, root, uint64(block.timestamp * 1000), uint64(block.timestamp * 1000), epoch
            )
        );
    }
}
