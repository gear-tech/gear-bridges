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
import {DeploymentArguments, DeploymentBase, Overrides} from "script/DeploymentBase.sol";
import {RetainedBridgeMigration} from "script/RetainedBridgeMigration.sol";
import {BridgingPayment} from "src/BridgingPayment.sol";
import {ERC20Manager} from "src/ERC20Manager.sol";
import {GovernanceAdmin} from "src/GovernanceAdmin.sol";
import {GovernancePauser} from "src/GovernancePauser.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {VaraQueueRootVerifier} from "src/VaraQueueRootVerifier.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {ERC20GearSupply} from "src/erc20/managed/ERC20GearSupply.sol";
import {IERC20Manager} from "src/interfaces/IERC20Manager.sol";
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

abstract contract Base is
    DeploymentBase,
    RetainedBridgeMigration,
    StdAssertions,
    StdChains,
    StdCheats,
    StdInvariant,
    StdUtils
{
    using Hasher for VaraMessage;

    using GovernancePacker for UpgradeProxyMessage;
    using GovernancePacker for PauseProxyMessage;
    using GovernancePacker for UnpauseProxyMessage;

    using ERC20ManagerPacker for TransferMessage;

    uint256 public messageNonce;
    uint256 public currentBlockNumber = 1;

    ForkBeefyFixture internal forkBeefy;

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

    function _startDeployment() internal override {
        if (deploymentArguments.privateKey != 0) {
            super._startDeployment();
            return;
        }
        vm.warp(vm.unixTime() / 1000);
        address[] memory proposers = new address[](1);
        proposers[0] = address(this);
        beefyRecoveryTimelock = address(new TimelockController(1 hours, proposers, proposers, address(0)));
        vm.deal(deploymentArguments.deployerAddress, BaseConstants.DEPLOYER_INITIAL_BALANCE);
        vm.startPrank(deploymentArguments.deployerAddress, deploymentArguments.deployerAddress);
    }

    function _finishDeployment() internal override {
        messageQueueAssertions(address(messageQueue));
        if (!messageOnlyBridge) {
            erc20ManagerAssertions(address(erc20Manager));
            bridgingPaymentAssertions();
        }
        if (deploymentArguments.privateKey != 0) {
            super._finishDeployment();
            return;
        }
        messageHandlerMock = new MessageHandlerMock();
        if (!messageOnlyBridge) newImplementationMock = new NewImplementationMock();
        vm.stopPrank();
    }

    /// forge-lint: disable-next-item(cyclomatic-complexity)
    function deployBridge(DeploymentArguments memory _deploymentArguments) public override {
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

        super.deployBridge(_deploymentArguments);
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
        p.retainedState = new StatePin[](tokens.length + 2);
        for (uint256 i; i < tokens.length; ++i) {
            p.tokens[i] = Identity(tokens[i], tokens[i].codehash);
            IERC20Metadata token = IERC20Metadata(tokens[i]);
            p.retainedState[i] = StatePin(
                tokens[i],
                abi.encodeCall(token.balanceOf, (address(erc20Manager))),
                keccak256(abi.encode(token.balanceOf(address(erc20Manager))))
            );
        }
        p.retainedState[tokens.length] = StatePin(
            address(messageQueue), abi.encodeCall(messageQueue.isProcessed, (836)), keccak256(abi.encode(true))
        );
        p.retainedState[tokens.length + 1] = StatePin(
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
        (uint128 id, uint128 length, bytes32 root) = beefyClient.currentValidatorSet();
        p.bootstrap.currentSet = BeefyClient.ValidatorSet(id, length, root);
        (id, length, root) = beefyClient.nextValidatorSet();
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

    function _deployVerifier(uint256 chainId, address messageQueueAddress) internal override returns (IVerifier) {
        if (deploymentArguments.privateKey != 0) return super._deployVerifier(chainId, messageQueueAddress);
        forkBeefy = new ForkBeefyFixture(MessageQueue(messageQueueAddress), 1, true);
        beefyClient = forkBeefy.client();
        return forkBeefy.verifier();
    }

    function _rootProof(uint256 sourceBlock, bytes32 root) internal returns (bytes memory) {
        require(sourceBlock <= type(uint32).max, "source block exceeds uint32");
        return forkBeefy.rootProof(uint32(sourceBlock), root);
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
