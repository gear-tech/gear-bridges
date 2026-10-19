// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {TimelockController} from "@openzeppelin/contracts/governance/TimelockController.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {RetainedBridgeMigration, RetainedBridgeMigrationScript} from "script/RetainedBridgeMigration.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {VaraQueueRootVerifier} from "src/VaraQueueRootVerifier.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {VaraBridgeMetadata} from "src/beefy/VaraBridgeMetadata.sol";
import {IMessageQueue, VaraMessage} from "src/interfaces/IMessageQueue.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";
import {Hasher} from "src/libraries/Hasher.sol";
import {
    GovernancePacker,
    PauseProxyMessage,
    UnpauseProxyMessage,
    UpgradeProxyMessage
} from "src/libraries/packing/GovernancePacker.sol";
import {Base} from "test/Base.sol";
import {BeefyFixtureTest} from "test/BeefyInterop.t.sol";

/// @dev Test-only surface: local pins below exercise chronology; production finality always uses canonical RPC/archive readback.
contract RetainedMigrationHarness is RetainedBridgeMigration {
    mapping(bytes32 => string) private rpcResponses;

    function rpcResponse(string memory method, string memory params, string memory response) external {
        rpcResponses[keccak256(abi.encode(method, params))] = response;
    }

    function _retainedRpc(string memory method, string memory params) internal override returns (string memory) {
        string memory response = rpcResponses[keccak256(abi.encode(method, params))];
        require(bytes(response).length != 0, "unexpected fixture RPC");
        return response;
    }

    function sourceCut(MigrationPlan memory p, MigrationStage stage) external view {
        _authenticateRetainedSourceCut(p, stage);
    }

    function predecessor(MigrationPlan memory p, MigrationAction memory a) external {
        _authenticateRetainedPredecessor(p, a);
    }

    function creation(MigrationPlan memory p, MigrationStage stage) external pure returns (bytes memory) {
        return _retainedCreationCode(p, stage);
    }

    function emptyChronology(MigrationPlan memory p, EmptyBarrier memory barrier) external view {
        _checkRetainedEmptyChronology(p, barrier);
    }

    function authenticateOriginal(
        OriginalTransaction memory evidence,
        address signer,
        uint64 nonce,
        address target,
        bytes memory input,
        address created
    ) external returns (FinalizedPin memory) {
        return _authenticateRetainedOriginal(evidence, signer, nonce, target, input, created);
    }

    function validate(MigrationPlan memory p, bool migrated) external view {
        validateRetainedMigration(p, migrated);
    }

    function upgradeData(MigrationPlan memory p) external pure returns (bytes memory) {
        return retainedUpgradePayload(p);
    }

    function root(MigrationPlan memory p, GovernanceDelivery memory d, GovernanceAction a) external {
        registerRetainedGovernanceRoot(p, d, a);
    }

    function execute(MigrationPlan memory p, GovernanceDelivery memory d, GovernanceAction a) external {
        executeRetainedGovernance(p, d, a);
    }

    function unpauseRoot(MigrationPlan memory p, GovernanceDelivery memory d, EmptyBarrier memory b) external {
        registerRetainedUnpauseRoot(p, d, b);
    }

    function unpause(MigrationPlan memory p, GovernanceDelivery memory d, EmptyBarrier memory b) external {
        executeRetainedUnpause(p, d, b);
    }

    function chronology(MigrationPlan memory p, GovernanceDelivery memory d, EmptyBarrier memory b) external view {
        _checkRetainedUnpauseChronology(p, d, b);
    }

    function original(
        bytes32 hash,
        address signer,
        uint64 nonce,
        address target,
        bytes memory input,
        address created,
        string memory transaction,
        string memory receipt
    ) external pure returns (FinalizedPin memory) {
        return _validateRetainedOriginalRpc(hash, signer, nonce, target, input, created, transaction, receipt);
    }
}

contract RetainedBridgeMigrationTest is BeefyFixtureTest, Base {
    using Hasher for VaraMessage;

    RetainedMigrationHarness internal migration;
    RetainedBridgeMigration.MigrationPlan internal approved;
    BeefyClient internal migrationClient;

    function setUp() public {
        vm.pauseGasMetering();
        // Keep deployment artifact/JSON allocations in a separate EVM call frame.
        this.deployBridgeFromConstants();
        // Explicit legacy migration fixture only: preserve Base custody/authorities, install the pinned old runtime.
        // Fresh deployment intentionally has no legacy initializer or legacy verifier mode.
        string memory legacy = vm.readFile("test/fixtures/mainnet-legacy-queue.json");
        address legacyImplementation = vm.parseJsonAddress(legacy, ".implementation");
        vm.etch(legacyImplementation, vm.parseJsonBytes(legacy, ".deployedBytecode"));
        assertEq(legacyImplementation.codehash, vm.parseJsonBytes32(legacy, ".codeHash"));
        vm.store(
            address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT, bytes32(uint256(uint160(legacyImplementation)))
        );
        address historicalVerifier = address(0x1E6A77);
        vm.etch(historicalVerifier, hex"00");
        verifier = IVerifier(historicalVerifier);
        vm.store(address(messageQueue), bytes32(uint256(5)), bytes32(uint256(uint160(address(verifier)))));
        vm.store(address(messageQueue), bytes32(uint256(14)), bytes32(0));
        vm.store(address(messageQueue), bytes32(uint256(16)), bytes32(0));
        bytes32 initializerSlot = vm.parseJsonBytes32(legacy, ".initializableSlot");
        vm.store(address(messageQueue), initializerSlot, bytes32(0));
        (bool initialized,) = address(messageQueue)
            .call(
                abi.encodeWithSignature(
                    "initialize(address,address,address,address[],address)",
                    address(governanceAdmin),
                    address(governancePauser),
                    deploymentArguments.emergencyStopAdmin,
                    deploymentArguments.emergencyStopObservers,
                    address(verifier)
                )
            );
        require(initialized, "frozen legacy initialization failed");
        fixtures = vm.readFile("test/fixtures/beefy-interop.json");
        migration = new RetainedMigrationHarness();
        _mockLegacyRoot(100, keccak256("retained user root"));
        messageQueue.submitMerkleRoot(100, keccak256("retained user root"), "");
        vm.clearMockedCalls();
        approved = _approvedPlan();
        // Actual BEEFY signatures and MMR verification, not a verifier mock for the new lane.
        customQueueProofFor(
            migrationClient,
            103,
            106,
            keccak256("bootstrap"),
            uint64(vm.getBlockTimestamp() * 1000),
            uint64(vm.getBlockTimestamp() * 1000)
        );
        vm.resumeGasMetering();
    }

    function testRetainedEntrypointRejectsFreshModeBeforeCreate() public {
        RetainedBridgeMigrationScript script = new RetainedBridgeMigrationScript();
        vm.setEnv("DEPLOYMENT_MODE", "fresh-beefy");
        uint64 nonce = vm.getNonce(address(script));
        vm.expectRevert();
        script.run();
        assertEq(vm.getNonce(address(script)), nonce);
    }

    function _identity(address account) private view returns (RetainedBridgeMigration.Identity memory) {
        return RetainedBridgeMigration.Identity(account, account.codehash);
    }

    function _approvedPlan() private returns (RetainedBridgeMigration.MigrationPlan memory p) {
        p.chainId = block.chainid;
        p.ethereumGenesis = keccak256("test-only genesis pin");
        p.queue = _identity(address(messageQueue));
        p.legacyImplementation =
            _identity(address(uint160(uint256(vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT)))));
        p.legacyVerifier = _identity(address(verifier));
        p.governanceAdmin = _identity(address(governanceAdmin));
        p.governancePauser = _identity(address(governancePauser));
        p.manager = _identity(address(erc20Manager));
        p.wrappedVara = _identity(address(wrappedVara));
        address[] memory tokens = erc20Manager.tokens();
        p.tokens = new RetainedBridgeMigration.Identity[](tokens.length);
        for (uint256 i; i < tokens.length; ++i) {
            p.tokens[i] = _identity(tokens[i]);
        }
        address[] memory payments = erc20Manager.bridgingPayments();
        p.payments = new RetainedBridgeMigration.Identity[](payments.length);
        for (uint256 i; i < payments.length; ++i) {
            p.payments[i] = _identity(payments[i]);
        }
        p.sourceAdmin = governanceAdmin.governance();
        p.sourcePauser = governancePauser.governance();
        p.sourceManagers = erc20Manager.vftManagers();
        p.emergencyAdmin = messageQueue.emergencyStopAdmin();
        p.emergencyObservers = messageQueue.emergencyStopObservers();
        p.genesisBlock = 100;
        p.legacyMax = 102;
        p.firstBeefyBlock = 103;
        p.legacyInitializerVersion = uint64(
            uint256(vm.load(address(messageQueue), 0xf0c57e16840df040f15088dc2f81fe391c3923bec73e23a9662efc9c229c6a00))
        );
        p.implementation = _identity(address(new MessageQueue()));
        uint128 count = uint128(fixtureUint(0, "validatorCount"));
        bytes32 authority = fixtureHash(0, "authorityRoot");
        p.bootstrap = RetainedBridgeMigration.Bootstrap(
            fixtureHash(0, "sourceDomain"),
            103,
            104,
            uint64(vm.getBlockTimestamp() * 1000),
            BeefyClient.ValidatorSet(0, count, authority),
            BeefyClient.ValidatorSet(1, count, authority)
        );
        migrationClient = new BeefyClient(
            p.bootstrap.sourceDomain,
            block.chainid,
            address(messageQueue),
            103,
            104,
            p.bootstrap.initialSourceTimestampMs,
            p.bootstrap.currentSet,
            p.bootstrap.nextSet
        );
        p.client = _identity(address(migrationClient));
        p.verifier =
            _identity(address(new VaraQueueRootVerifier(migrationClient, address(messageQueue), block.chainid)));
        address[] memory members = new address[](1);
        members[0] = address(this);
        TimelockController recovery = new TimelockController(1 hours, members, members, address(0));
        p.recovery = _identity(address(recovery));
        p.recoveryDelay = 1 hours;
        p.recoveryRoles = new RetainedBridgeMigration.RolePin[](3);
        p.recoveryRoles[0] = RetainedBridgeMigration.RolePin(recovery.PROPOSER_ROLE(), address(this), true);
        p.recoveryRoles[1] = RetainedBridgeMigration.RolePin(recovery.EXECUTOR_ROLE(), address(this), true);
        p.recoveryRoles[2] = RetainedBridgeMigration.RolePin(recovery.DEFAULT_ADMIN_ROLE(), address(recovery), true);
        p.retainedState = new RetainedBridgeMigration.StatePin[](2);
        p.retainedState[0] = RetainedBridgeMigration.StatePin(
            address(messageQueue), abi.encodeCall(MessageQueue.genesisBlock, ()), keccak256(abi.encode(uint256(100)))
        );
        p.retainedState[1] = RetainedBridgeMigration.StatePin(
            address(messageQueue),
            abi.encodeCall(MessageQueue.getMerkleRoot, (100)),
            keccak256(abi.encode(keccak256("retained user root")))
        );
        p.sourceCutApproval = keccak256("test-only source cut");
        p.outstandingUnpauseInventory = keccak256("test-only reconciled empty inventory");
        p.retainedProxies = new RetainedBridgeMigration.ProxyPin[](2);
        p.retainedProxies[0] = RetainedBridgeMigration.ProxyPin(
            address(erc20Manager),
            ERC1967Utils.IMPLEMENTATION_SLOT,
            _identity(address(uint160(uint256(vm.load(address(erc20Manager), ERC1967Utils.IMPLEMENTATION_SLOT)))))
        );
        p.retainedProxies[1] = RetainedBridgeMigration.ProxyPin(
            address(wrappedVara),
            ERC1967Utils.IMPLEMENTATION_SLOT,
            _identity(address(uint160(uint256(vm.load(address(wrappedVara), ERC1967Utils.IMPLEMENTATION_SLOT)))))
        );
    }

    function _delivery(uint256 sourceBlock, uint256 nonce, bool upgrade)
        private
        view
        returns (RetainedBridgeMigration.GovernanceDelivery memory d)
    {
        RetainedBridgeMigration.MigrationPlan memory p = approved;
        d.sourceBlock = sourceBlock;
        d.totalLeaves = 1;
        d.message = VaraMessage(
            nonce,
            upgrade ? p.sourceAdmin : p.sourcePauser,
            upgrade ? p.governanceAdmin.account : p.governancePauser.account,
            upgrade ? migration.upgradeData(p) : GovernancePacker.pack(PauseProxyMessage(p.queue.account))
        );
        d.root = d.message.hash();
    }

    function _mockLegacyRoot(uint256 sourceBlock, bytes32 root) private {
        uint256[] memory inputs = new uint256[](2);
        inputs[0] = uint256(root) >> 64;
        inputs[1] = (uint256(uint64(uint256(root))) << 128) | (sourceBlock << 96);
        vm.mockCall(
            address(verifier),
            abi.encodeWithSignature("safeVerifyProof(bytes,uint256[])", bytes(""), inputs),
            abi.encode(true)
        );
    }

    function _registerLegacyRoot(
        RetainedBridgeMigration.GovernanceDelivery memory delivery,
        RetainedBridgeMigration.GovernanceAction action
    ) private {
        _mockLegacyRoot(delivery.sourceBlock, delivery.root);
        migration.root(approved, delivery, action);
        vm.clearMockedCalls();
    }

    function _registerAndPause() private returns (RetainedBridgeMigration.GovernanceDelivery memory upgrade) {
        RetainedBridgeMigration.GovernanceDelivery memory pause = _delivery(101, 5001, false);
        upgrade = _delivery(102, 5002, true);
        _registerLegacyRoot(pause, RetainedBridgeMigration.GovernanceAction.Pause);
        assertEq(messageQueue.maxBlockNumber(), 101);
        _registerLegacyRoot(upgrade, RetainedBridgeMigration.GovernanceAction.Upgrade);
        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());
        migration.execute(approved, pause, RetainedBridgeMigration.GovernanceAction.Pause);
        assertTrue(messageQueue.paused());
    }

    function testUnsafeIdentitiesRejectBeforeCreate() public {
        RetainedBridgeMigration.MigrationPlan memory p = approved;
        uint64 nonce = vm.getNonce(address(migration));
        p.sourceAdmin = 0;
        vm.expectRevert();
        migration.validate(p, false);
        p = approved;
        p.sourcePauser = 0;
        vm.expectRevert();
        migration.validate(p, false);
        p = approved;
        p.sourceManagers[0] = 0;
        vm.expectRevert();
        migration.validate(p, false);
        p = approved;
        p.bootstrap.sourceDomain = 0;
        vm.expectRevert();
        migration.validate(p, false);
        p = approved;
        p.chainId++;
        vm.expectRevert();
        migration.validate(p, false);
        p = approved;
        p.queue.codeHash = keccak256("wrong proxy code");
        vm.expectRevert();
        migration.validate(p, false);
        p = approved;
        p.legacyVerifier.account = p.manager.account;
        p.legacyVerifier.codeHash = p.manager.codeHash;
        vm.expectRevert();
        migration.validate(p, false);
        p = approved;
        p.firstBeefyBlock = p.legacyMax;
        vm.expectRevert();
        migration.validate(p, false);
        p = approved;
        p.recoveryDelay++;
        vm.expectRevert();
        migration.validate(p, false);
        p = approved;
        p.recoveryRoles[0].account = address(0xBAD);
        vm.expectRevert();
        migration.validate(p, false);
        p = approved;
        p.retainedProxies[0].implementation.codeHash = keccak256("different manager implementation");
        vm.expectRevert();
        migration.validate(p, false);
        assertEq(vm.getNonce(address(migration)), nonce);
    }

    function testInvalidBootstrapRejectsDuringPreparation() public {
        RetainedBridgeMigration.MigrationPlan memory p = approved;
        p.bootstrap.currentSet.root = 0;
        vm.expectRevert();
        migration.validate(p, false);
        p = approved;
        p.bootstrap.nextSet.id += 2;
        vm.expectRevert();
        migration.validate(p, false);
        p = approved;
        p.bootstrap.nextSet.length = 257;
        vm.expectRevert();
        migration.validate(p, false);
    }

    function testSharedExecutionRetainsQueueCustodyAndReplay() public {
        uint256 balance = circleToken.balanceOf(address(erc20Manager));
        RetainedBridgeMigration.GovernanceDelivery memory upgrade = _registerAndPause();
        migration.execute(approved, upgrade, RetainedBridgeMigration.GovernanceAction.Upgrade);
        migration.validate(approved, true);
        assertEq(circleToken.balanceOf(address(erc20Manager)), balance);
        assertEq(messageQueue.verifier(), approved.verifier.account);
        assertEq(messageQueue.beefyRootMinimum(), 103);
        assertEq(messageQueue.beefyRecoveryTimelock(), approved.recovery.account);
        assertTrue(messageQueue.isProcessed(5001));
        assertTrue(messageQueue.isProcessed(5002));
        assertTrue(messageQueue.paused());
        vm.expectRevert();
        migration.execute(approved, upgrade, RetainedBridgeMigration.GovernanceAction.Upgrade);
    }

    function testInterveningRootRejectsStaleApprovedMaximum() public {
        RetainedBridgeMigration.GovernanceDelivery memory upgrade = _registerAndPause();
        _mockLegacyRoot(103, keccak256("permissionless newer legacy root"));
        messageQueue.submitMerkleRoot(103, keccak256("permissionless newer legacy root"), "");
        vm.clearMockedCalls();
        vm.expectRevert();
        migration.execute(approved, upgrade, RetainedBridgeMigration.GovernanceAction.Upgrade);
        assertFalse(messageQueue.isProcessed(5002));
        assertEq(messageQueue.verifier(), approved.legacyVerifier.account);
    }

    function testUnsignedOrExpiredBootstrapCannotMigrate() public {
        RetainedBridgeMigration.GovernanceDelivery memory upgrade = _registerAndPause();
        vm.warp(vm.getBlockTimestamp() + 1 days);
        vm.expectRevert();
        migration.execute(approved, upgrade, RetainedBridgeMigration.GovernanceAction.Upgrade);
        assertFalse(messageQueue.isProcessed(5002));
    }

    function testCannotPrepareOrDeliverLegacyUnpauseWithPauseBatch() public {
        RetainedBridgeMigration.GovernanceDelivery memory d = _delivery(101, 7001, false);
        d.message.payload = GovernancePacker.pack(UnpauseProxyMessage(address(messageQueue)));
        d.root = d.message.hash();
        vm.expectRevert();
        migration.root(approved, d, RetainedBridgeMigration.GovernanceAction.Unpause);
        vm.expectRevert();
        migration.execute(approved, d, RetainedBridgeMigration.GovernanceAction.Unpause);
        assertEq(messageQueue.getMerkleRoot(101), bytes32(0));
    }

    function _evidencePath(string memory suffix) private pure returns (string memory) {
        return string.concat("broadcast/retained-regression-", vm.toString(uint32(msg.sig)), "-", suffix, ".json");
    }

    function _approvedCutFiles(bool registerLiability)
        private
        returns (
            RetainedBridgeMigration.GovernanceDelivery memory pause,
            RetainedBridgeMigration.GovernanceDelivery memory upgrade
        )
    {
        approved.legacyMax = 104;
        approved.firstBeefyBlock = 105;
        bytes32 liability = keccak256("approved outstanding user liability");
        if (registerLiability) {
            _mockLegacyRoot(101, liability);
            messageQueue.submitMerkleRoot(101, liability, "");
            vm.clearMockedCalls();
        }
        pause = _delivery(103, 9101, false);
        upgrade = _delivery(104, 9102, true);
        RetainedBridgeMigration.LegacyRootPin[] memory roots = new RetainedBridgeMigration.LegacyRootPin[](4);
        roots[0] = RetainedBridgeMigration.LegacyRootPin(100, messageQueue.getMerkleRoot(100));
        roots[1] = RetainedBridgeMigration.LegacyRootPin(101, liability);
        roots[2] = RetainedBridgeMigration.LegacyRootPin(103, pause.root);
        roots[3] = RetainedBridgeMigration.LegacyRootPin(104, upgrade.root);
        string memory cut = string.concat(
            '{"queue":"',
            vm.toString(address(messageQueue)),
            '","sourceDomain":"',
            vm.toString(approved.bootstrap.sourceDomain),
            '","legacyMax":104,"firstBeefyBlock":105,"roots":"',
            vm.toString(abi.encode(roots)),
            '"}'
        );
        string memory inventory = string.concat(
            '{"queue":"',
            vm.toString(address(messageQueue)),
            '","sourcePauser":"',
            vm.toString(approved.sourcePauser),
            '","unpauseNonces":[]}'
        );
        vm.createDir("broadcast", true);
        vm.writeFile(_evidencePath("cut"), cut);
        vm.writeFile(_evidencePath("inventory"), inventory);
        vm.setEnv("RETAINED_SOURCE_CUT", _evidencePath("cut"));
        vm.setEnv("RETAINED_UNPAUSE_INVENTORY", _evidencePath("inventory"));
        approved.sourceCutApproval = keccak256(bytes(cut));
        approved.outstandingUnpauseInventory = keccak256(bytes(inventory));
        migration.sourceCut(approved, RetainedBridgeMigration.MigrationStage.Prepare);
        _registerLegacyRoot(pause, RetainedBridgeMigration.GovernanceAction.Pause);
        _registerLegacyRoot(upgrade, RetainedBridgeMigration.GovernanceAction.Upgrade);
        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());
    }

    function _externalGovernance(RetainedBridgeMigration.GovernanceDelivery memory d) private {
        vm.prank(address(0xE7EC));
        messageQueue.processMessage(d.sourceBlock, d.totalLeaves, d.leafIndex, d.message, d.proof);
        assertTrue(messageQueue.isProcessed(d.message.nonce));
    }

    function _removeCutFiles() private {
        vm.removeFile(_evidencePath("cut"));
        vm.removeFile(_evidencePath("inventory"));
    }

    function testMissingApprovedLiabilityHoldsBeforePauseAndAfterExternalUpgrade() public {
        (
            RetainedBridgeMigration.GovernanceDelivery memory pause,
            RetainedBridgeMigration.GovernanceDelivery memory upgrade
        ) = _approvedCutFiles(false);
        assertEq(messageQueue.maxBlockNumber(), approved.legacyMax);
        assertEq(messageQueue.getMerkleRoot(101), bytes32(0));
        vm.expectRevert(abi.encodeWithSelector(RetainedMigrationHold.selector, "unsettled/conflicting cut root"));
        migration.sourceCut(approved, RetainedBridgeMigration.MigrationStage.Pause);
        _externalGovernance(pause);
        _externalGovernance(upgrade);
        migration.validate(approved, true);
        for (uint256 stage; stage <= uint256(RetainedBridgeMigration.MigrationStage.Unpause); ++stage) {
            vm.expectRevert(abi.encodeWithSelector(RetainedMigrationHold.selector, "unsettled/conflicting cut root"));
            migration.sourceCut(approved, RetainedBridgeMigration.MigrationStage(stage));
        }
        assertTrue(messageQueue.paused());
        _removeCutFiles();
    }

    function testCompleteApprovedLiabilitiesPassEveryPostUpgradeSourceReadback() public {
        (
            RetainedBridgeMigration.GovernanceDelivery memory pause,
            RetainedBridgeMigration.GovernanceDelivery memory upgrade
        ) = _approvedCutFiles(true);
        migration.sourceCut(approved, RetainedBridgeMigration.MigrationStage.Pause);
        _externalGovernance(pause);
        _externalGovernance(upgrade);
        for (uint256 stage; stage <= uint256(RetainedBridgeMigration.MigrationStage.Unpause); ++stage) {
            migration.sourceCut(approved, RetainedBridgeMigration.MigrationStage(stage));
        }
        migration.validate(approved, true);
        _removeCutFiles();
    }

    function testSignedNativeHandoverRetainsOriginalBootstrapAndMigrates() public {
        RetainedBridgeMigration.Bootstrap memory originalBootstrap = approved.bootstrap;
        VaraBridgeMetadata.Snapshot memory s = VaraBridgeMetadata.Snapshot({
            version: 2,
            bridgeDomain: migrationClient.bridgeDomain(),
            sourceTimestampMs: migrationClient.lastAuthenticatedSourceTimestampMs() + 1,
            initialized: true,
            queueId: 9,
            queueRoot: keccak256("authenticated handover snapshot")
        });
        BeefyClient.MMRLeaf memory leaf = leafForSnapshot(
            s, 107, 2, uint32(originalBootstrap.nextSet.length), keccak256("authenticated successor authority")
        );
        BeefyClient.Commitment memory c = commitment(108, 1, keccak256(leafBytes(leaf)));
        bytes32 digest = migrationClient.computeCommitmentHash(c);
        BeefyClient.ValidatorProof memory initial = validatorProof(0, 0);
        (initial.v, initial.r, initial.s) = vm.sign(1, digest);
        uint256[] memory signers = allSigners();
        migrationClient.submitInitial(c, signers, initial);
        vm.roll(vm.getBlockNumber() + 128);
        vm.prevrandao(keccak256("native handover randomness"));
        migrationClient.commitPrevRandao(digest);
        uint256[] memory selection = migrationClient.createFinalBitfield(digest, signers);
        migrationClient.submitFinal(
            c, signers, selectedProofs(migrationClient, c, 0, selection[0], ""), leaf, s, new bytes32[](0), 0
        );
        (uint128 current,,,) = migrationClient.currentValidatorSet();
        (uint128 next,, bytes32 nextRoot,) = migrationClient.nextValidatorSet();
        assertEq(current, 1);
        assertEq(next, 2);
        assertEq(nextRoot, leaf.nextAuthoritySetRoot);
        assertEq(keccak256(abi.encode(approved.bootstrap)), keccak256(abi.encode(originalBootstrap)));
        migration.validate(approved, false);
        RetainedBridgeMigration.GovernanceDelivery memory upgrade = _registerAndPause();
        migration.execute(approved, upgrade, RetainedBridgeMigration.GovernanceAction.Upgrade);
        migration.validate(approved, true);
        assertEq(messageQueue.verifier(), approved.verifier.account);
    }

    function _observation(uint256 number, string memory label, uint256 includedAt)
        private
        pure
        returns (RetainedBridgeMigration.FinalityObservation memory observation)
    {
        observation.state = RetainedBridgeMigration.FinalizedPin(number, keccak256(bytes(label)), includedAt);
        observation.head =
            RetainedBridgeMigration.FinalizedPin(number + 64, keccak256(abi.encode(label, "head")), includedAt + 768);
        observation.observedAt = includedAt + 768;
    }

    function _barrierAndUnpause()
        private
        returns (RetainedBridgeMigration.EmptyBarrier memory b, RetainedBridgeMigration.GovernanceDelivery memory d)
    {
        return _barrierAndUnpauseTiming(false, false);
    }

    function _barrierAndUnpauseTiming(bool earlyEmpty, bool earlyUnpause)
        private
        returns (RetainedBridgeMigration.EmptyBarrier memory b, RetainedBridgeMigration.GovernanceDelivery memory d)
    {
        RetainedBridgeMigration.GovernanceDelivery memory upgrade = _registerAndPause();
        migration.execute(approved, upgrade, RetainedBridgeMigration.GovernanceAction.Upgrade);
        b.upgradeReadback = _observation(10, "local finalized upgrade", vm.getBlockTimestamp());
        vm.warp(b.upgradeReadback.observedAt);
        vm.warp(vm.getBlockTimestamp() + 121);
        b.sourceBlock = 200;
        b.proof = encodeProof(
            customQueueProofForEpoch(
                migrationClient,
                200,
                202,
                bytes32(0),
                uint64((earlyEmpty ? b.upgradeReadback.state.timestamp + 121 : vm.getBlockTimestamp()) * 1000),
                uint64(vm.getBlockTimestamp() * 1000),
                7
            )
        );
        messageQueue.submitEmptyQueueProgress(b.sourceBlock, b.proof);
        b.publication = _observation(100, "local finalized empty publication", vm.getBlockTimestamp());
        vm.warp(b.publication.observedAt);
        vm.warp(vm.getBlockTimestamp() + 121);
        d.sourceBlock = 201;
        d.totalLeaves = 1;
        d.message = VaraMessage(
            8001,
            approved.sourcePauser,
            approved.governancePauser.account,
            GovernancePacker.pack(UnpauseProxyMessage(address(messageQueue)))
        );
        d.root = d.message.hash();
        d.rootProof = encodeProof(
            customQueueProofForEpoch(
                migrationClient,
                201,
                205,
                d.root,
                uint64((earlyUnpause ? b.publication.state.timestamp + 121 : vm.getBlockTimestamp()) * 1000),
                uint64(vm.getBlockTimestamp() * 1000),
                8
            )
        );
    }

    function testLaterAuthenticatedEpochCanUnpauseThroughRetainedGovernance() public {
        (RetainedBridgeMigration.EmptyBarrier memory b, RetainedBridgeMigration.GovernanceDelivery memory d) =
            _barrierAndUnpause();
        migration.unpauseRoot(approved, d, b);
        assertTrue(messageQueue.paused());
        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());
        migration.unpause(approved, d, b);
        assertFalse(messageQueue.paused());
        assertTrue(messageQueue.isProcessed(d.message.nonce));
    }

    function testEmptyEpochAfterInclusionButBeforeObservedFinalityHolds() public {
        (RetainedBridgeMigration.EmptyBarrier memory b, RetainedBridgeMigration.GovernanceDelivery memory d) =
            _barrierAndUnpauseTiming(true, false);
        assertEq(b.upgradeReadback.observedAt, b.upgradeReadback.state.timestamp + 768);
        assertEq(_retainedSnapshot(b.proof).sourceTimestampMs, (b.upgradeReadback.state.timestamp + 121) * 1000);
        assertEq(messageQueue.maxBlockNumber(), b.sourceBlock); // Actual signed empty proof was accepted by the queue.
        vm.expectRevert(
            abi.encodeWithSelector(
                RetainedMigrationHold.selector, "empty barrier precedes finalized upgrade observation"
            )
        );
        migration.emptyChronology(approved, b);
        vm.expectRevert(
            abi.encodeWithSelector(RetainedMigrationHold.selector, "unpause precedes finalized upgrade/empty epoch")
        );
        migration.unpauseRoot(approved, d, b);
        assertEq(messageQueue.getMerkleRoot(d.sourceBlock), bytes32(0));
        assertTrue(messageQueue.paused());
    }

    function testUnpauseEpochAfterPublicationButBeforeObservedFinalityHolds() public {
        (RetainedBridgeMigration.EmptyBarrier memory b, RetainedBridgeMigration.GovernanceDelivery memory d) =
            _barrierAndUnpauseTiming(false, true);
        assertEq(b.publication.observedAt, b.publication.state.timestamp + 768);
        assertEq(_retainedSnapshot(d.rootProof).sourceTimestampMs, (b.publication.state.timestamp + 121) * 1000);
        vm.expectRevert(
            abi.encodeWithSelector(RetainedMigrationHold.selector, "unpause precedes finalized upgrade/empty epoch")
        );
        migration.unpauseRoot(approved, d, b);
        assertEq(messageQueue.getMerkleRoot(d.sourceBlock), bytes32(0));
        messageQueue.submitMerkleRoot(d.sourceBlock, d.root, d.rootProof);
        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());
        vm.expectRevert(
            abi.encodeWithSelector(RetainedMigrationHold.selector, "unpause precedes finalized upgrade/empty epoch")
        );
        migration.unpause(approved, d, b);
        assertFalse(messageQueue.isProcessed(d.message.nonce));
        assertTrue(messageQueue.paused());
    }

    function testEarlyOrSameEpochUnpauseFailsWithoutRegistration() public {
        (RetainedBridgeMigration.EmptyBarrier memory b, RetainedBridgeMigration.GovernanceDelivery memory d) =
            _barrierAndUnpause();
        b.publication.state.number = b.upgradeReadback.state.number;
        vm.expectRevert();
        migration.unpauseRoot(approved, d, b);
        assertEq(messageQueue.getMerkleRoot(d.sourceBlock), bytes32(0));
        b.publication.state.number = 100;
        b.publication.observedAt = vm.getBlockTimestamp(); // Future skew is not chronology proof.
        vm.expectRevert();
        migration.unpauseRoot(approved, d, b);
        assertTrue(messageQueue.paused());
        b.publication.observedAt -= 121;
        d.rootProof = encodeProof(
            customQueueProofForEpoch(
                migrationClient,
                201,
                208,
                d.root,
                uint64(vm.getBlockTimestamp() * 1000),
                uint64(vm.getBlockTimestamp() * 1000),
                7
            )
        );
        vm.expectRevert();
        migration.unpauseRoot(approved, d, b);
        assertEq(messageQueue.getMerkleRoot(d.sourceBlock), bytes32(0));
    }

    function _rpcTransaction(bytes32 hash) private view returns (string memory) {
        return string.concat(
            '{"hash":"',
            vm.toString(hash),
            '","from":"',
            vm.toString(address(this)),
            '","to":"',
            vm.toString(address(messageQueue)),
            '","chainId":',
            vm.toString(block.chainid),
            ',"nonce":36,"value":0,"input":"0x12345678",',
            '"blockNumber":20,"blockHash":"',
            vm.toString(keccak256("canonical block")),
            '"}'
        );
    }

    function _rpcReceipt(bytes32 hash, bool success) private view returns (string memory) {
        return string.concat(
            '{"transactionHash":"',
            vm.toString(hash),
            '","from":"',
            vm.toString(address(this)),
            '","to":"',
            vm.toString(address(messageQueue)),
            '","status":',
            success ? "1" : "0",
            ',"blockNumber":20,"blockHash":"',
            vm.toString(keccak256("canonical block")),
            '"}'
        );
    }

    function testMatchingOriginalReceiptReturnsItsOwnInclusion() public view {
        bytes32 hash = keccak256("original signed bytes");
        RetainedBridgeMigration.FinalizedPin memory pin = migration.original(
            hash,
            address(this),
            36,
            address(messageQueue),
            hex"12345678",
            address(0),
            _rpcTransaction(hash),
            _rpcReceipt(hash, true)
        );
        assertEq(pin.number, 20);
        assertEq(pin.blockHash, keccak256("canonical block"));
    }

    function testCompetitorReceiptAndFinalizedRevertNeverBecomeOriginalSuccess() public {
        bytes32 original = keccak256("original signed bytes");
        bytes32 competitor = keccak256("competing success");
        string memory txJson = _rpcTransaction(original);
        string memory failed = _rpcReceipt(original, false);
        bytes32 preserved = keccak256(bytes(failed));
        vm.expectRevert();
        migration.original(
            original,
            address(this),
            36,
            address(messageQueue),
            hex"12345678",
            address(0),
            txJson,
            _rpcReceipt(competitor, true)
        );
        vm.expectRevert();
        migration.original(
            original, address(this), 36, address(messageQueue), hex"12345678", address(0), txJson, failed
        );
        vm.expectRevert();
        migration.original(
            original,
            address(this),
            37,
            address(messageQueue),
            hex"12345678",
            address(0),
            txJson,
            _rpcReceipt(original, true)
        );
        assertEq(keccak256(bytes(failed)), preserved);
    }

    function _fixtureReceipt(bytes32 hash, address target, address created, bool success)
        private
        view
        returns (string memory)
    {
        return string.concat(
            '{"transactionHash":"',
            vm.toString(hash),
            '","from":"',
            vm.toString(address(this)),
            '","to":',
            created == address(0) ? string.concat('"', vm.toString(target), '"') : "null",
            ',"contractAddress":"',
            vm.toString(created),
            '","status":',
            success ? "1" : "0",
            ',"transactionIndex":0,"blockNumber":20,"blockHash":"',
            vm.toString(keccak256("canonical block")),
            '"}'
        );
    }

    function _originalFixture(bytes memory input, address target, address created)
        private
        returns (
            RetainedBridgeMigration.OriginalTransaction memory original,
            string memory transaction,
            string memory receipt
        )
    {
        original.transactionHash = keccak256(abi.encode("approved original", input, target, created));
        transaction = string.concat(
            '{"hash":"',
            vm.toString(original.transactionHash),
            '","from":"',
            vm.toString(address(this)),
            '","to":',
            created == address(0) ? string.concat('"', vm.toString(target), '"') : "null",
            ',"chainId":',
            vm.toString(block.chainid),
            ',"nonce":36,"value":0,"input":"',
            vm.toString(input),
            '","transactionIndex":0,"blockNumber":20,"blockHash":"',
            vm.toString(keccak256("canonical block")),
            '"}'
        );
        receipt = _fixtureReceipt(original.transactionHash, target, created, true);
        string memory artifact = string.concat(
            '{"transactions":[{"hash":"',
            vm.toString(original.transactionHash),
            '","contractAddress":"',
            vm.toString(created),
            '","transaction":',
            transaction,
            '}],"receipts":[',
            receipt,
            "]}"
        );
        original.artifact = _evidencePath("original");
        original.artifactHash = keccak256(bytes(artifact));
        vm.createDir("broadcast", true);
        vm.writeFile(original.artifact, artifact);
        string memory hashParam = string.concat('["', vm.toString(original.transactionHash), '"]');
        migration.rpcResponse("eth_getTransactionByHash", hashParam, transaction);
        migration.rpcResponse("eth_getTransactionReceipt", hashParam, receipt);
        string memory inclusion = string.concat(
            '{"number":20,"hash":"',
            vm.toString(keccak256("canonical block")),
            '","timestamp":',
            vm.toString(vm.getBlockTimestamp()),
            ',"transactions":["',
            vm.toString(original.transactionHash),
            '"]}'
        );
        string memory finalized = string.concat(
            '{"number":84,"hash":"',
            vm.toString(keccak256("canonical finalized head")),
            '","timestamp":',
            vm.toString(vm.getBlockTimestamp() + 768),
            "}"
        );
        migration.rpcResponse("eth_getBlockByNumber", '["0x14",false]', inclusion);
        migration.rpcResponse("eth_getBlockByNumber", '["finalized",false]', finalized);
        migration.rpcResponse("eth_getBlockByNumber", '["0x54",false]', finalized);
    }

    function testOriginalConstructorApprovalRejectsDifferentBootstrapInputs() public {
        bytes memory originalCreation =
            migration.creation(approved, RetainedBridgeMigration.MigrationStage.DeployClient);
        (RetainedBridgeMigration.OriginalTransaction memory evidence,,) =
            _originalFixture(originalCreation, address(0), approved.client.account);
        migration.authenticateOriginal(
            evidence, address(this), 36, address(0), originalCreation, approved.client.account
        );
        RetainedBridgeMigration.MigrationPlan memory changed = approved;
        changed.bootstrap.currentSet.root = keccak256("different independently approved bootstrap");
        bytes memory changedCreation = migration.creation(changed, RetainedBridgeMigration.MigrationStage.DeployClient);
        vm.expectRevert(abi.encodeWithSelector(RetainedMigrationHold.selector, "artifact transaction intent"));
        migration.authenticateOriginal(
            evidence, address(this), 36, address(0), changedCreation, approved.client.account
        );
        vm.removeFile(evidence.artifact);
    }

    function _predecessorFixture()
        private
        returns (
            RetainedBridgeMigration.MigrationAction memory action,
            string memory transaction,
            string memory receipt
        )
    {
        RetainedBridgeMigration.GovernanceDelivery memory upgrade = _registerAndPause();
        _externalGovernance(upgrade);
        migration.validate(approved, true); // Matching state was genuinely produced by another executor.
        action.planHash = keccak256(abi.encode(approved));
        action.upgrade.delivery = upgrade;
        bytes memory input = abi.encodeCall(
            IMessageQueue.processMessage,
            (upgrade.sourceBlock, upgrade.totalLeaves, upgrade.leafIndex, upgrade.message, upgrade.proof)
        );
        (action.upgrade.original, transaction, receipt) = _originalFixture(input, address(messageQueue), address(0));
        action.upgradeReadback =
            RetainedBridgeMigration.FinalizedPin(20, keccak256("canonical block"), vm.getBlockTimestamp());
        action.barrier.upgradeReadback.state = action.upgradeReadback;
        action.barrier.upgradeReadback.head = RetainedBridgeMigration.FinalizedPin(
            84, keccak256("canonical finalized head"), vm.getBlockTimestamp() + 768
        );
        action.barrier.upgradeReadback.observedAt = vm.getBlockTimestamp() + 768;
        vm.warp(action.barrier.upgradeReadback.observedAt);
        string memory observation = string.concat(
            '{"schemaVersion":1,"planHash":"',
            vm.toString(action.planHash),
            '","stage":8,"transactionHash":"',
            vm.toString(action.upgrade.original.transactionHash),
            '","observation":"',
            vm.toString(abi.encode(action.barrier.upgradeReadback)),
            '"}'
        );
        vm.writeFile(_evidencePath("observation"), observation);
        vm.setEnv("RETAINED_UPGRADE_OBSERVATION", _evidencePath("observation"));
        vm.setEnv("RETAINED_UPGRADE_OBSERVATION_HASH", vm.toString(keccak256(bytes(observation))));
        vm.warp(vm.getBlockTimestamp() + 121);
        action.barrier.sourceBlock = 200;
        action.barrier.proof = encodeProof(
            customQueueProofForEpoch(
                migrationClient,
                200,
                202,
                bytes32(0),
                uint64(vm.getBlockTimestamp() * 1000),
                uint64(vm.getBlockTimestamp() * 1000),
                7
            )
        );
        messageQueue.submitEmptyQueueProgress(action.barrier.sourceBlock, action.barrier.proof);
        action.barrier.original.transactionHash = keccak256(
            abi.encodeCall(MessageQueue.submitEmptyQueueProgress, (action.barrier.sourceBlock, action.barrier.proof))
        );
        action.barrier.publication = _observation(85, "canonical empty publication", vm.getBlockTimestamp());
        string memory publicationHeader = string.concat(
            '{"number":85,"hash":"',
            vm.toString(action.barrier.publication.state.blockHash),
            '","timestamp":',
            vm.toString(action.barrier.publication.state.timestamp),
            "}"
        );
        string memory publicationHead = string.concat(
            '{"number":149,"hash":"',
            vm.toString(action.barrier.publication.head.blockHash),
            '","timestamp":',
            vm.toString(action.barrier.publication.head.timestamp),
            "}"
        );
        migration.rpcResponse("eth_getBlockByNumber", '["0x55",false]', publicationHeader);
        migration.rpcResponse("eth_getBlockByNumber", '["0x95",false]', publicationHead);
        migration.rpcResponse("eth_getBlockByNumber", '["finalized",false]', publicationHead);
        vm.warp(action.barrier.publication.observedAt);
        string memory barrierObservation = string.concat(
            '{"schemaVersion":1,"planHash":"',
            vm.toString(action.planHash),
            '","stage":10,"transactionHash":"',
            vm.toString(action.barrier.original.transactionHash),
            '","observation":"',
            vm.toString(abi.encode(action.barrier.publication)),
            '"}'
        );
        vm.writeFile(_evidencePath("barrier-observation"), barrierObservation);
        vm.setEnv("RETAINED_BARRIER_OBSERVATION", _evidencePath("barrier-observation"));
        vm.setEnv("RETAINED_BARRIER_OBSERVATION_HASH", vm.toString(keccak256(bytes(barrierObservation))));
    }

    function _allPredecessorStages(RetainedBridgeMigration.MigrationAction memory action, bool hold) private {
        for (
            uint256 stage = uint256(RetainedBridgeMigration.MigrationStage.Verify);
            stage <= uint256(RetainedBridgeMigration.MigrationStage.Unpause);
            ++stage
        ) {
            action.stage = RetainedBridgeMigration.MigrationStage(stage);
            if (hold) vm.expectRevert();
            migration.predecessor(approved, action);
        }
        assertTrue(messageQueue.paused());
        assertTrue(messageQueue.isProcessed(action.upgrade.delivery.message.nonce));
        assertEq(messageQueue.verifier(), approved.verifier.account);
    }

    function testEveryLaterStageRequiresOriginalUpgradeDespiteMatchingExternalState() public {
        (RetainedBridgeMigration.MigrationAction memory action, string memory transaction, string memory receipt) =
            _predecessorFixture();
        string memory params = string.concat('["', vm.toString(action.upgrade.original.transactionHash), '"]');
        _allPredecessorStages(action, false);
        migration.rpcResponse("eth_getTransactionByHash", params, "null");
        _allPredecessorStages(action, true);
        migration.rpcResponse("eth_getTransactionByHash", params, transaction);
        migration.rpcResponse("eth_getTransactionReceipt", params, "null");
        _allPredecessorStages(action, true);
        migration.rpcResponse(
            "eth_getTransactionReceipt",
            params,
            _fixtureReceipt(action.upgrade.original.transactionHash, address(messageQueue), address(0), false)
        );
        _allPredecessorStages(action, true);
        migration.rpcResponse(
            "eth_getTransactionReceipt",
            params,
            _fixtureReceipt(keccak256("competitor success"), address(messageQueue), address(0), true)
        );
        _allPredecessorStages(action, true);
        migration.rpcResponse("eth_getTransactionReceipt", params, receipt);
        migration.rpcResponse(
            "eth_getTransactionByHash", params, _rpcTransaction(action.upgrade.original.transactionHash)
        );
        _allPredecessorStages(action, true);
        migration.rpcResponse("eth_getTransactionByHash", params, transaction);
        action.upgrade.original.artifactHash ^= bytes32(uint256(1));
        _allPredecessorStages(action, true);
        action.upgrade.original.artifactHash ^= bytes32(uint256(1));
        migration.rpcResponse(
            "eth_getBlockByNumber",
            '["0x14",false]',
            string.concat(
                '{"number":20,"hash":"',
                vm.toString(keccak256("canonical block")),
                '","timestamp":',
                vm.toString(action.upgradeReadback.timestamp),
                ',"transactions":["',
                vm.toString(keccak256("different canonical transaction")),
                '"]}'
            )
        );
        _allPredecessorStages(action, true);
        vm.removeFile(action.upgrade.original.artifact);
        vm.removeFile(_evidencePath("observation"));
        vm.removeFile(_evidencePath("barrier-observation"));
    }

    function testUnapprovedOrBackdatedObservationCannotAdvanceBarrier() public {
        (RetainedBridgeMigration.MigrationAction memory action,,) = _predecessorFixture();
        action.stage = RetainedBridgeMigration.MigrationStage.EmptyBarrier;
        migration.predecessor(approved, action);
        action.barrier.upgradeReadback.observedAt = action.upgradeReadback.timestamp + 121;
        vm.expectRevert(abi.encodeWithSelector(RetainedMigrationHold.selector, "finality observation changed"));
        migration.predecessor(approved, action);
        string memory backdated = string.concat(
            '{"schemaVersion":1,"planHash":"',
            vm.toString(action.planHash),
            '","stage":8,"transactionHash":"',
            vm.toString(action.upgrade.original.transactionHash),
            '","observation":"',
            vm.toString(abi.encode(action.barrier.upgradeReadback)),
            '"}'
        );
        vm.writeFile(_evidencePath("observation"), backdated);
        vm.setEnv("RETAINED_UPGRADE_OBSERVATION_HASH", vm.toString(keccak256(bytes(backdated))));
        vm.expectRevert(abi.encodeWithSelector(RetainedMigrationHold.selector, "invalid finality observation"));
        migration.predecessor(approved, action);
        assertTrue(messageQueue.paused());
        vm.removeFile(action.upgrade.original.artifact);
        vm.removeFile(_evidencePath("observation"));
        vm.removeFile(_evidencePath("barrier-observation"));
    }
}
