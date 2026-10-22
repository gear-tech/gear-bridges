// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {TimelockController} from "@openzeppelin/contracts/governance/TimelockController.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {CommonBase} from "forge-std/Base.sol";
import {Script} from "forge-std/Script.sol";
import {Vm, VmSafe} from "forge-std/Vm.sol";
import {BridgingPayment} from "src/BridgingPayment.sol";
import {ERC20Manager} from "src/ERC20Manager.sol";
import {GovernanceAdmin} from "src/GovernanceAdmin.sol";
import {GovernancePauser} from "src/GovernancePauser.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {VaraQueueRootVerifier} from "src/VaraQueueRootVerifier.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {IMessageQueue, VaraMessage} from "src/interfaces/IMessageQueue.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";
import {
    GovernancePacker,
    PauseProxyMessage,
    UnpauseProxyMessage,
    UpgradeProxyMessage
} from "src/libraries/packing/GovernancePacker.sol";

/// @dev Retains the entire deployed bridge. Each invocation prepares, verifies, or submits ONE approved transaction.
/// JSON inputs contain an ABI-encoded `plan` (schema 1) / `action` (schema 2) byte string; their exact file bytes
/// are independently approved with RETAINED_PLAN_HASH / RETAINED_ACTION_HASH. Foundry run artifacts remain the
/// original transaction/receipt record. An attempt marker only prevents a second signature if that record is missing.
abstract contract RetainedBridgeMigration is CommonBase {
    struct Identity {
        address account;
        bytes32 codeHash;
    }

    struct Bootstrap {
        bytes32 sourceDomain;
        uint64 mmrStartBlock;
        uint64 initialBeefyBlock;
        uint64 initialSourceTimestampMs;
        BeefyClient.ValidatorSet currentSet;
        BeefyClient.ValidatorSet nextSet;
    }

    struct RolePin {
        bytes32 role;
        address account;
        bool member;
    }

    // Include original root/maturity/replay coordinates and custody/role readbacks in the approved source-cut report.
    struct StatePin {
        address target;
        bytes data;
        bytes32 resultHash;
    }

    struct ProxyPin {
        address proxy;
        bytes32 implementationSlot;
        Identity implementation;
    }

    struct LegacyRootPin {
        uint256 sourceBlock;
        bytes32 root;
    }

    struct MigrationPlan {
        uint256 chainId;
        bytes32 ethereumGenesis;
        Identity queue;
        Identity legacyImplementation;
        Identity legacyVerifier;
        Identity governanceAdmin;
        Identity governancePauser;
        Identity manager;
        Identity wrappedVara;
        Identity[] tokens;
        Identity[] payments;
        bytes32 sourceAdmin;
        bytes32 sourcePauser;
        bytes32[] sourceManagers;
        address emergencyAdmin;
        address[] emergencyObservers;
        uint256 genesisBlock;
        uint256 legacyMax;
        uint256 firstBeefyBlock;
        uint64 legacyInitializerVersion;
        Identity implementation;
        Identity client;
        Identity verifier;
        Bootstrap bootstrap;
        Identity recovery;
        uint256 recoveryDelay;
        RolePin[] recoveryRoles;
        StatePin[] retainedState;
        bytes32 sourceCutApproval;
        bytes32 outstandingUnpauseInventory;
        ProxyPin[] retainedProxies;
    }

    struct GovernanceDelivery {
        uint256 sourceBlock;
        bytes32 root;
        bytes rootProof;
        uint256 totalLeaves;
        uint256 leafIndex;
        VaraMessage message;
        bytes32[] proof;
    }
    enum GovernanceAction {
        Pause,
        Upgrade,
        Unpause
    }
    enum MigrationStage {
        Prepare,
        DeployImplementation,
        DeployClient,
        DeployVerifier,
        Bootstrap,
        RegisterPause,
        RegisterUpgrade,
        Pause,
        Upgrade,
        Verify,
        EmptyBarrier,
        RegisterUnpause,
        Unpause
    }

    struct FinalizedPin {
        uint256 number;
        bytes32 blockHash;
        uint256 timestamp;
    }

    /// @dev Captured after original-receipt and finalized-state qualification, not at transaction inclusion.
    struct FinalityObservation {
        FinalizedPin state;
        FinalizedPin head;
        uint256 observedAt;
    }

    struct EmptyBarrier {
        FinalityObservation upgradeReadback;
        FinalityObservation publication;
        uint256 sourceBlock;
        bytes proof;
        OriginalTransaction original;
    }

    struct OriginalTransaction {
        string artifact;
        bytes32 artifactHash;
        uint256 transactionIndex;
        bytes32 transactionHash;
    }

    struct UpgradeEvidence {
        GovernanceDelivery delivery;
        OriginalTransaction original;
    }

    struct MigrationAction {
        bytes32 planHash;
        MigrationStage stage;
        address signer;
        uint64 nonce;
        GovernanceDelivery delivery;
        // Exact submitInitial/commitPrevRandao/submitFinal/submitFiatShamir call, not a follower binding bypass.
        bytes bootstrapCall;
        UpgradeEvidence upgrade;
        FinalizedPin upgradeReadback;
        EmptyBarrier barrier;
    }

    struct QueueSnapshotProof {
        uint8 proofVersion;
        uint8 bridgeVersion;
        bool initialized;
        bytes32 bridgeDomain;
        uint64 sourceTimestampMs;
        uint64 queueId;
        uint64 anchorBlock;
        bytes32 anchorRoot;
        BeefyClient.MMRLeaf leaf;
        bytes32[] items;
        uint256 order;
    }

    bytes32 internal constant RETAINED_INITIALIZER_SLOT =
        0xf0c57e16840df040f15088dc2f81fe391c3923bec73e23a9662efc9c229c6a00;
    error RetainedMigrationHold(string reason);
    event RetainedMigrationPrepared(bytes32 indexed planHash, bytes migrationCalldata, bytes upgradePayload);
    event RetainedMigrationVerified(bytes32 indexed planHash, uint256 blockNumber, bytes32 blockHash);
    event RetainedFinalityObserved(
        bytes32 indexed planHash, MigrationStage stage, bytes32 artifactHash, bytes observation
    );

    function retainedMigrationCalldata(MigrationPlan memory p) internal pure returns (bytes memory) {
        return abi.encodeCall(
            MessageQueue.migrateToBeefy,
            (
                IVerifier(p.legacyVerifier.account),
                IVerifier(p.verifier.account),
                p.legacyMax,
                p.firstBeefyBlock,
                p.recovery.account
            )
        );
    }

    function retainedUpgradePayload(MigrationPlan memory p) internal pure returns (bytes memory) {
        return GovernancePacker.pack(
            UpgradeProxyMessage(p.queue.account, p.implementation.account, retainedMigrationCalldata(p))
        );
    }

    function _retainedRequire(bool ok, string memory reason) internal pure {
        if (!ok) revert RetainedMigrationHold(reason);
    }

    function _retainedIdentity(Identity memory identity) internal view {
        _retainedRequire(
            identity.account != address(0) && identity.account.code.length != 0 && identity.codeHash != bytes32(0)
                && identity.account.codehash == identity.codeHash,
            "contract identity"
        );
    }

    function _retainedPlannedIdentity(Identity memory identity) private pure {
        _retainedRequire(identity.account != address(0) && identity.codeHash != bytes32(0), "missing planned identity");
    }

    /// @dev Does not deploy, mutate authority, infer source approval from live state, or relax legacy maximum checks.
    function validateRetainedMigration(MigrationPlan memory p, bool migrated) internal view {
        _retainedRequire(block.chainid == p.chainId && p.ethereumGenesis != bytes32(0), "chain identity");
        _retainedRequire(
            p.sourceAdmin != 0 && p.sourcePauser != 0 && p.bootstrap.sourceDomain != 0 && p.sourceManagers.length != 0
                && p.emergencyAdmin != address(0),
            "zero source identity"
        );
        _retainedRequire(
            p.sourceCutApproval != 0 && p.outstandingUnpauseInventory != 0 && p.retainedState.length != 0,
            "missing cut/inventory/retained evidence"
        );
        _retainedRequire(
            p.genesisBlock != 0 && p.legacyInitializerVersion < 8 && p.legacyMax >= p.genesisBlock
                && p.firstBeefyBlock > p.legacyMax && p.firstBeefyBlock <= type(uint32).max
                && p.firstBeefyBlock <= p.legacyMax + 57_600 && p.bootstrap.mmrStartBlock != 0
                && p.firstBeefyBlock >= p.bootstrap.mmrStartBlock
                && p.bootstrap.initialBeefyBlock > p.bootstrap.mmrStartBlock,
            "cut/bootstrap range"
        );
        Bootstrap memory bootstrap = p.bootstrap;
        _retainedRequire(
            bootstrap.currentSet.length != 0 && bootstrap.currentSet.length <= 256 && bootstrap.nextSet.length != 0
                && bootstrap.nextSet.length <= 256 && bootstrap.currentSet.root != 0 && bootstrap.nextSet.root != 0
                && bootstrap.currentSet.id < type(uint64).max && bootstrap.nextSet.id == bootstrap.currentSet.id + 1
                && bootstrap.initialBeefyBlock <= type(uint32).max && bootstrap.initialSourceTimestampMs != 0,
            "invalid bootstrap policy"
        );
        if (!migrated && p.client.account.code.length == 0) {
            _retainedRequire(
                uint256(bootstrap.initialSourceTimestampMs) <= block.timestamp * 1000 + 120_000
                    && block.timestamp * 1000 <= uint256(bootstrap.initialSourceTimestampMs) + 86_400_000,
                "bootstrap timestamp"
            );
        }
        _retainedPlannedIdentity(p.implementation);
        _retainedPlannedIdentity(p.client);
        _retainedPlannedIdentity(p.verifier);
        _retainedRequire(
            p.implementation.account != p.legacyImplementation.account && p.verifier.account != p.legacyVerifier.account
                && p.client.account != p.queue.account && p.implementation.account != p.client.account
                && p.implementation.account != p.verifier.account && p.client.account != p.verifier.account,
            "replacement is not fresh"
        );
        _retainedIdentity(p.queue);
        _retainedIdentity(p.legacyImplementation);
        _retainedIdentity(p.legacyVerifier);
        _retainedIdentity(p.governanceAdmin);
        _retainedIdentity(p.governancePauser);
        _retainedIdentity(p.manager);
        _retainedIdentity(p.wrappedVara);
        _retainedIdentity(p.recovery);
        _retainedProxyInventory(p);
        MessageQueue q = MessageQueue(p.queue.account);
        address implementation = address(uint160(uint256(vm.load(p.queue.account, ERC1967Utils.IMPLEMENTATION_SLOT))));
        _retainedRequire(
            implementation == (migrated ? p.implementation.account : p.legacyImplementation.account),
            "proxy implementation"
        );
        uint64 version = uint64(uint256(vm.load(p.queue.account, RETAINED_INITIALIZER_SLOT)));
        _retainedRequire(version == (migrated ? 8 : p.legacyInitializerVersion), "initializer version");
        _retainedRequire(q.verifier() == (migrated ? p.verifier.account : p.legacyVerifier.account), "queue verifier");
        _retainedRequire(
            q.genesisBlock() == p.genesisBlock
                && (migrated ? q.maxBlockNumber() >= p.legacyMax : q.maxBlockNumber() <= p.legacyMax),
            "legacy range changed"
        );
        _retainedRequire(!q.isEmergencyStopped() && !q.isChallengingRoot(), "emergency/challenge active");
        _retainedRequire(
            q.governanceAdmin() == p.governanceAdmin.account && q.governancePauser() == p.governancePauser.account
                && q.emergencyStopAdmin() == p.emergencyAdmin
                && keccak256(abi.encode(q.emergencyStopObservers())) == keccak256(abi.encode(p.emergencyObservers)),
            "retained queue authority"
        );
        _retainedRequire(
            q.hasRole(bytes32(0), p.governanceAdmin.account) && q.hasRole(q.PAUSER_ROLE(), p.governanceAdmin.account)
                && q.hasRole(q.PAUSER_ROLE(), p.governancePauser.account),
            "queue governance roles"
        );
        GovernanceAdmin admin = GovernanceAdmin(p.governanceAdmin.account);
        GovernancePauser pauser = GovernancePauser(p.governancePauser.account);
        _retainedRequire(
            admin.governance() == p.sourceAdmin && pauser.governance() == p.sourcePauser
                && address(admin.messageQueue()) == p.queue.account && address(pauser.messageQueue()) == p.queue.account
                && address(admin.erc20Manager()) == p.manager.account
                && address(pauser.erc20Manager()) == p.manager.account
                && address(admin.wrappedVara()) == p.wrappedVara.account
                && address(pauser.wrappedVara()) == p.wrappedVara.account,
            "retained governance binding"
        );
        ERC20Manager manager = ERC20Manager(p.manager.account);
        _retainedRequire(
            manager.messageQueue() == p.queue.account && manager.governanceAdmin() == p.governanceAdmin.account
                && manager.governancePauser() == p.governancePauser.account
                && keccak256(abi.encode(manager.vftManagers())) == keccak256(abi.encode(p.sourceManagers)),
            "manager binding"
        );
        for (uint256 i; i < p.sourceManagers.length; ++i) {
            _retainedRequire(p.sourceManagers[i] != 0, "zero source manager");
        }
        address[] memory tokens = manager.tokens();
        address[] memory payments = manager.bridgingPayments();
        _retainedRequire(
            tokens.length == p.tokens.length && tokens.length != 0 && payments.length == p.payments.length
                && payments.length != 0,
            "retained inventory length"
        );
        bool wrapped;
        for (uint256 i; i < tokens.length; ++i) {
            _retainedIdentity(p.tokens[i]);
            _retainedRequire(tokens[i] == p.tokens[i].account, "retained token");
            if (tokens[i] == p.wrappedVara.account) wrapped = true;
            for (uint256 j; j < i; ++j) {
                _retainedRequire(tokens[j] != tokens[i], "duplicate token");
            }
        }
        _retainedRequire(wrapped, "wrapped VARA absent");
        for (uint256 i; i < payments.length; ++i) {
            _retainedIdentity(p.payments[i]);
            _retainedRequire(
                payments[i] == p.payments[i].account
                    && BridgingPayment(payments[i]).erc20Manager() == p.manager.account,
                "retained payment"
            );
        }
        TimelockController recovery = TimelockController(payable(p.recovery.account));
        _retainedRequire(
            p.recoveryDelay != 0 && recovery.getMinDelay() == p.recoveryDelay && p.recoveryRoles.length != 0,
            "recovery policy"
        );
        for (uint256 i; i < p.recoveryRoles.length; ++i) {
            RolePin memory role = p.recoveryRoles[i];
            _retainedRequire(recovery.hasRole(role.role, role.account) == role.member, "recovery role");
        }
        for (uint256 i; i < p.retainedState.length; ++i) {
            StatePin memory pin = p.retainedState[i];
            _retainedRequire(pin.target.code.length != 0 && pin.data.length >= 4, "retained readback target");
            (bool ok, bytes memory result) = pin.target.staticcall(pin.data);
            _retainedRequire(ok && keccak256(result) == pin.resultHash, "retained state changed");
        }
        if (migrated) {
            _retainedRequire(
                q.beefyRootMinimum() == p.firstBeefyBlock && q.beefyRecoveryTimelock() == p.recovery.account,
                "migration configuration"
            );
            _retainedIdentity(p.implementation);
            _retainedClient(p, true);
            _retainedVerifier(p);
        }
    }

    function _retainedClient(MigrationPlan memory p, bool requireLiveRoot) internal view {
        _retainedIdentity(p.client);
        BeefyClient client = BeefyClient(p.client.account);
        _retainedRequire(
            client.sourceDomain() == p.bootstrap.sourceDomain && client.destinationChainId() == p.chainId
                && client.destinationQueue() == p.queue.account && client.mmrStartBlock() == p.bootstrap.mmrStartBlock
                && client.bridgeDomain()
                    == keccak256(
                        abi.encodePacked(
                            "vara/gear-eth-bridge-domain/v2",
                            p.bootstrap.sourceDomain,
                            bytes32(p.chainId),
                            p.queue.account
                        )
                    ),
            "client binding"
        );
        if (requireLiveRoot) {
            _retainedRequire(
                client.isLive() && client.latestMMRRoot() != 0 && client.latestBeefyBlock() > p.firstBeefyBlock,
                "bootstrap not genuinely accepted/live"
            );
        } else if (client.latestMMRRoot() == 0) {
            (uint128 id, uint128 length, bytes32 root) = client.currentValidatorSet();
            (uint128 nextId, uint128 nextLength, bytes32 nextRoot) = client.nextValidatorSet();
            _retainedRequire(
                id == p.bootstrap.currentSet.id && length == p.bootstrap.currentSet.length
                    && root == p.bootstrap.currentSet.root && nextId == p.bootstrap.nextSet.id
                    && nextLength == p.bootstrap.nextSet.length && nextRoot == p.bootstrap.nextSet.root
                    && client.latestBeefyBlock() == p.bootstrap.initialBeefyBlock
                    && client.lastAuthenticatedSourceTimestampMs() == p.bootstrap.initialSourceTimestampMs,
                "bootstrap identity"
            );
        }
    }

    function _retainedVerifier(MigrationPlan memory p) internal view {
        _retainedIdentity(p.verifier);
        VaraQueueRootVerifier verifier = VaraQueueRootVerifier(p.verifier.account);
        _retainedRequire(
            address(verifier.beefyClient()) == p.client.account && verifier.messageQueue() == p.queue.account
                && verifier.destinationChainId() == p.chainId,
            "verifier binding"
        );
    }

    function verifyRetainedPoststate(MigrationPlan memory p, bool expectedPaused) internal view {
        validateRetainedMigration(p, true);
        _retainedRequire(MessageQueue(p.queue.account).paused() == expectedPaused, "unexpected pause state");
    }

    function _retainedGovernancePayload(MigrationPlan memory p, GovernanceAction action)
        internal
        pure
        returns (bytes memory)
    {
        if (action == GovernanceAction.Upgrade) return retainedUpgradePayload(p);
        if (action == GovernanceAction.Pause) return GovernancePacker.pack(PauseProxyMessage(p.queue.account));
        return GovernancePacker.pack(UnpauseProxyMessage(p.queue.account));
    }

    function _validateRetainedDelivery(MigrationPlan memory p, GovernanceDelivery memory d, GovernanceAction action)
        internal
        pure
    {
        bool admin = action == GovernanceAction.Upgrade;
        _retainedRequire(
            d.sourceBlock != 0 && d.sourceBlock <= type(uint32).max && d.root != 0 && d.totalLeaves != 0
                && d.leafIndex < d.totalLeaves && d.message.source == (admin ? p.sourceAdmin : p.sourcePauser)
                && d.message.destination == (admin ? p.governanceAdmin.account : p.governancePauser.account)
                && keccak256(d.message.payload) == keccak256(_retainedGovernancePayload(p, action)),
            "governance intent"
        );
        _retainedRequire(
            action == GovernanceAction.Unpause ? d.sourceBlock >= p.firstBeefyBlock : d.sourceBlock <= p.legacyMax,
            "governance publication outside cut"
        );
    }

    function registerRetainedGovernanceRoot(
        MigrationPlan memory p,
        GovernanceDelivery memory d,
        GovernanceAction action
    ) internal {
        _validateRetainedDelivery(p, d, action);
        _retainedRequire(action != GovernanceAction.Unpause, "unpause needs finalized barrier");
        validateRetainedMigration(p, false);
        MessageQueue q = MessageQueue(p.queue.account);
        _retainedRequire(q.getMerkleRoot(d.sourceBlock) == 0 && !q.isProcessed(d.message.nonce), "root/message exists");
        q.submitMerkleRoot(d.sourceBlock, d.root, d.rootProof);
        _retainedRequire(
            q.getMerkleRoot(d.sourceBlock) == d.root && q.maxBlockNumber() <= p.legacyMax,
            "legacy publication changed intent"
        );
    }

    /// @dev Unpause has a separate evidence-bearing entrypoint. A nonce or local boolean cannot unlock it.
    function executeRetainedGovernance(MigrationPlan memory p, GovernanceDelivery memory d, GovernanceAction action)
        internal
    {
        _retainedRequire(action != GovernanceAction.Unpause, "unpause needs finalized barrier");
        _validateRetainedDelivery(p, d, action);
        validateRetainedMigration(p, false);
        MessageQueue q = MessageQueue(p.queue.account);
        _retainedRequire(q.maxBlockNumber() == p.legacyMax, "stale legacy maximum");
        _retainedRequire(
            q.getMerkleRoot(d.sourceBlock) == d.root && !q.isProcessed(d.message.nonce), "root/message state"
        );
        if (action == GovernanceAction.Pause) {
            _retainedRequire(!q.paused(), "unexpected prior pause");
        } else {
            _retainedRequire(q.paused(), "upgrade must remain paused");
            _retainedIdentity(p.implementation);
            _retainedClient(p, true);
            _retainedVerifier(p);
        }
        q.processMessage(d.sourceBlock, d.totalLeaves, d.leafIndex, d.message, d.proof);
        _retainedRequire(q.isProcessed(d.message.nonce) && q.paused(), "governance did not complete paused");
        if (action == GovernanceAction.Upgrade) verifyRetainedPoststate(p, true);
    }

    function _retainedSnapshot(bytes memory encoded) internal pure returns (QueueSnapshotProof memory p) {
        (
            p.proofVersion,
            p.bridgeVersion,
            p.initialized,
            p.bridgeDomain,
            p.sourceTimestampMs,
            p.queueId,
            p.anchorBlock,
            p.anchorRoot,
            p.leaf,
            p.items,
            p.order
        ) =
            abi.decode(
                encoded,
                (uint8, uint8, bool, bytes32, uint64, uint64, uint64, bytes32, BeefyClient.MMRLeaf, bytes32[], uint256)
            );
    }

    function _retainedObservationTime(FinalityObservation memory observation) internal pure returns (uint256) {
        _retainedRequire(
            observation.state.number != 0 && observation.state.blockHash != 0 && observation.head.blockHash != 0
                && observation.head.number >= observation.state.number
                && observation.head.timestamp >= observation.state.timestamp && observation.observedAt != 0
                && observation.observedAt >= observation.head.timestamp,
            "invalid finality observation"
        );
        return observation.observedAt;
    }

    function _checkRetainedEmptyChronology(MigrationPlan memory p, EmptyBarrier memory barrier) internal view {
        QueueSnapshotProof memory snapshot = _retainedSnapshot(barrier.proof);
        _retainedRequire(
            barrier.sourceBlock >= p.firstBeefyBlock && snapshot.initialized
                && uint256(snapshot.sourceTimestampMs)
                    > _retainedObservationTime(barrier.upgradeReadback) * 1000
                        + BeefyClient(p.client.account).MAX_FUTURE_SOURCE_SKEW_MS(),
            "empty barrier precedes finalized upgrade observation"
        );
    }

    function _checkRetainedUnpauseChronology(MigrationPlan memory p, GovernanceDelivery memory d, EmptyBarrier memory b)
        internal
        view
    {
        _validateRetainedDelivery(p, d, GovernanceAction.Unpause);
        QueueSnapshotProof memory empty = _retainedSnapshot(b.proof);
        QueueSnapshotProof memory unpause = _retainedSnapshot(d.rootProof);
        uint256 skew = BeefyClient(p.client.account).MAX_FUTURE_SOURCE_SKEW_MS();
        _retainedRequire(
            b.publication.state.number > b.upgradeReadback.state.number
                && _retainedObservationTime(b.publication) > _retainedObservationTime(b.upgradeReadback)
                && b.sourceBlock >= p.firstBeefyBlock && d.sourceBlock > b.sourceBlock && empty.initialized
                && unpause.initialized && empty.queueId < unpause.queueId
                && uint256(empty.sourceTimestampMs) > b.upgradeReadback.observedAt * 1000 + skew
                && uint256(unpause.sourceTimestampMs) > b.publication.observedAt * 1000 + skew,
            "unpause precedes finalized upgrade/empty epoch"
        );
    }

    /// @dev Used only after canonical original publication receipts and finalized pins have been authenticated.
    /// Queue proof verification is still performed by the actual queue when this previously absent root is published.
    function registerRetainedUnpauseRoot(MigrationPlan memory p, GovernanceDelivery memory d, EmptyBarrier memory b)
        internal
    {
        verifyRetainedPoststate(p, true);
        _checkRetainedUnpauseChronology(p, d, b);
        MessageQueue q = MessageQueue(p.queue.account);
        _retainedRequire(
            q.getMerkleRoot(d.sourceBlock) == 0 && !q.isProcessed(d.message.nonce), "unpause already present"
        );
        q.submitMerkleRoot(d.sourceBlock, d.root, d.rootProof);
        _retainedRequire(q.getMerkleRoot(d.sourceBlock) == d.root, "unpause publication failed");
    }

    /// @dev As for registration, the environment runner first authenticates the original barrier and root receipts.
    /// Typed local/fork rehearsals supply their explicitly test-only finalized evidence outside this operational helper.
    function executeRetainedUnpause(MigrationPlan memory p, GovernanceDelivery memory d, EmptyBarrier memory b)
        internal
    {
        verifyRetainedPoststate(p, true);
        _checkRetainedUnpauseChronology(p, d, b);
        MessageQueue q = MessageQueue(p.queue.account);
        _retainedRequire(
            q.getMerkleRoot(d.sourceBlock) == d.root && !q.isProcessed(d.message.nonce), "unpause root/message state"
        );
        q.processMessage(d.sourceBlock, d.totalLeaves, d.leafIndex, d.message, d.proof);
        _retainedRequire(q.isProcessed(d.message.nonce), "unpause original message not consumed");
        verifyRetainedPoststate(p, false);
    }

    function _retainedCreationCode(MigrationPlan memory p, MigrationStage stage) internal pure returns (bytes memory) {
        if (stage == MigrationStage.DeployImplementation) return type(MessageQueue).creationCode;
        if (stage == MigrationStage.DeployClient) {
            return abi.encodePacked(
                type(BeefyClient).creationCode,
                abi.encode(
                    p.bootstrap.sourceDomain,
                    p.chainId,
                    p.queue.account,
                    p.bootstrap.mmrStartBlock,
                    p.bootstrap.initialBeefyBlock,
                    p.bootstrap.initialSourceTimestampMs,
                    p.bootstrap.currentSet,
                    p.bootstrap.nextSet
                )
            );
        }
        _retainedRequire(stage == MigrationStage.DeployVerifier, "not a deployment stage");
        return abi.encodePacked(
            type(VaraQueueRootVerifier).creationCode,
            abi.encode(BeefyClient(p.client.account), p.queue.account, p.chainId)
        );
    }

    function _retainedActionCall(MigrationPlan memory p, MigrationAction memory a)
        internal
        pure
        returns (address target, bytes memory data, address created)
    {
        if (a.stage >= MigrationStage.DeployImplementation && a.stage <= MigrationStage.DeployVerifier) {
            created = a.stage == MigrationStage.DeployImplementation
                ? p.implementation.account
                : a.stage == MigrationStage.DeployClient ? p.client.account : p.verifier.account;
            return (address(0), _retainedCreationCode(p, a.stage), created);
        }
        if (a.stage == MigrationStage.Bootstrap) {
            bytes4 selector = bytes4(a.bootstrapCall);
            _retainedRequire(
                selector == BeefyClient.submitFiatShamir.selector || selector == BeefyClient.submitInitial.selector
                    || selector == BeefyClient.commitPrevRandao.selector
                    || selector == BeefyClient.submitFinal.selector,
                "unapproved bootstrap entrypoint"
            );
            return (p.client.account, a.bootstrapCall, address(0));
        }
        GovernanceDelivery memory d = a.delivery;
        if (a.stage == MigrationStage.EmptyBarrier) {
            return (
                p.queue.account,
                abi.encodeCall(MessageQueue.submitEmptyQueueProgress, (a.barrier.sourceBlock, a.barrier.proof)),
                address(0)
            );
        }
        if (
            a.stage == MigrationStage.RegisterPause || a.stage == MigrationStage.RegisterUpgrade
                || a.stage == MigrationStage.RegisterUnpause
        ) {
            return (
                p.queue.account,
                abi.encodeCall(IMessageQueue.submitMerkleRoot, (d.sourceBlock, d.root, d.rootProof)),
                address(0)
            );
        }
        _retainedRequire(
            a.stage == MigrationStage.Pause || a.stage == MigrationStage.Upgrade || a.stage == MigrationStage.Unpause,
            "stage has no transaction"
        );
        return (
            p.queue.account,
            abi.encodeCall(
                IMessageQueue.processMessage, (d.sourceBlock, d.totalLeaves, d.leafIndex, d.message, d.proof)
            ),
            address(0)
        );
    }

    /// @dev No signing key is read during prepare/verify/reconciliation. PRIVATE_KEY is used only for a new,
    /// independently approved action whose nonce is unused in both latest and pending state.
    function runRetainedMigrationFromEnvironment() internal returns (MessageQueue existingQueue) {
        string memory json = vm.readFile(vm.envString("RETAINED_PLAN"));
        bytes32 planHash = keccak256(bytes(json));
        _retainedRequire(
            planHash == vm.envBytes32("RETAINED_PLAN_HASH") && vm.parseJsonUint(json, ".schemaVersion") == 1,
            "plan hash/schema changed"
        );
        MigrationPlan memory p = abi.decode(vm.parseJsonBytes(json, ".plan"), (MigrationPlan));
        existingQueue = MessageQueue(p.queue.account);
        _authenticateRetainedNetwork(p);
        string memory actionJson = vm.readFile(vm.envString("RETAINED_ACTION"));
        bytes32 actionHash = keccak256(bytes(actionJson));
        _retainedRequire(
            actionHash == vm.envBytes32("RETAINED_ACTION_HASH") && vm.parseJsonUint(actionJson, ".schemaVersion") == 2,
            "action hash/schema changed"
        );
        MigrationAction memory a = abi.decode(vm.parseJsonBytes(actionJson, ".action"), (MigrationAction));
        _retainedRequire(a.planHash == planHash, "action belongs to another plan");
        _authenticateRetainedSourceCut(p, a.stage);
        bool migrated = _retainedImplementation(p.queue.account) == p.implementation.account;
        validateRetainedMigration(p, migrated);
        _authenticateRetainedRecoveryRoles(p);
        if (a.stage == MigrationStage.Prepare) {
            _retainedRequire(!migrated && !existingQueue.paused(), "preparation requires active legacy queue");
            emit RetainedMigrationPrepared(planHash, retainedMigrationCalldata(p), retainedUpgradePayload(p));
            return existingQueue;
        }
        _authenticateRetainedDeploymentHistory(p, a.stage);
        _authenticateRetainedPredecessor(p, a);
        if (a.stage == MigrationStage.Verify) {
            _verifyRetainedFinalizedPoststate(p, a.upgradeReadback);
            _captureRetainedObservation(
                a.planHash, MigrationStage.Upgrade, a.upgrade.original.transactionHash, a.upgradeReadback
            );
            emit RetainedMigrationVerified(planHash, a.upgradeReadback.number, a.upgradeReadback.blockHash);
            return existingQueue;
        }
        (address target, bytes memory data, address created) = _retainedActionCall(p, a);
        string memory artifact = vm.envString("RETAINED_BROADCAST_ARTIFACT");
        string memory attempt = string.concat(
            vm.projectRoot(),
            "/broadcast/retained-",
            vm.toString(p.chainId),
            "-",
            vm.toString(a.signer),
            "-",
            vm.toString(uint256(a.nonce)),
            ".attempt"
        );
        string memory intent = string.concat(
            vm.toString(planHash),
            ":",
            vm.toString(actionHash),
            ":",
            vm.toString(a.signer),
            ":",
            vm.toString(uint256(a.nonce)),
            ":",
            vm.toString(keccak256(data))
        );
        string memory intentPath = string.concat(attempt, "/intent");
        if (vm.exists(intentPath)) {
            _retainedRequire(
                keccak256(bytes(vm.readFile(intentPath))) == keccak256(bytes(intent)), "original intent changed"
            );
        }
        if (vm.exists(artifact)) {
            OriginalTransaction memory original = OriginalTransaction(
                artifact,
                vm.envBytes32("RETAINED_BROADCAST_HASH"),
                vm.envUint("RETAINED_TRANSACTION_INDEX"),
                vm.envBytes32("RETAINED_TRANSACTION_HASH")
            );
            FinalizedPin memory included =
                _authenticateRetainedOriginal(original, a.signer, a.nonce, target, data, created);
            _retainedActionPoststate(p, a);
            if (a.stage == MigrationStage.EmptyBarrier) {
                _checkRetainedEmptyChronology(p, a.barrier);
                _verifyRetainedFinalizedPoststate(p, included);
                _captureRetainedObservation(a.planHash, a.stage, original.transactionHash, included);
            }
            if (a.stage == MigrationStage.RegisterUnpause || a.stage == MigrationStage.Unpause) {
                _authenticateRetainedBarrier(p, a.barrier);
                _checkRetainedUnpauseChronology(p, a.delivery, a.barrier);
                if (a.stage == MigrationStage.Unpause) _authenticateRetainedRootOriginal(p, a.delivery);
            }
            return existingQueue;
        }
        _retainedRequire(!vm.exists(attempt), "original outcome unknown; reconcile Foundry artifact, never re-sign");
        _retainedActionPrestate(p, a, migrated);
        _retainedRequire(
            a.signer != address(0) && _retainedNonce(a.signer, "latest") == a.nonce
                && _retainedNonce(a.signer, "pending") == a.nonce,
            "signer nonce owned or changed"
        );
        if (created != address(0)) {
            _retainedRequire(
                created.code.length == 0 && vm.computeCreateAddress(a.signer, a.nonce) == created,
                "deployment nonce/address changed"
            );
        }
        uint256 key = vm.envUint("PRIVATE_KEY");
        _retainedRequire(vm.addr(key) == a.signer, "approved signer mismatch");
        // A dry run does not claim nonce ownership. A broadcast attempt does, even if its RPC response is lost.
        if (vm.isContext(VmSafe.ForgeContext.ScriptBroadcast)) {
            vm.createDir(string.concat(vm.projectRoot(), "/broadcast"), true);
            string[] memory command = new string[](2);
            command[0] = "/bin/mkdir";
            command[1] = attempt;
            Vm.FfiResult memory acquired = vm.tryFfi(command);
            _retainedRequire(acquired.exitCode == 0, "another original owns this signer nonce");
            vm.writeFile(intentPath, intent);
        }
        vm.startBroadcast(key);
        if (a.stage == MigrationStage.DeployImplementation) {
            _retainedRequire(address(new MessageQueue()) == created, "implementation address");
        } else if (a.stage == MigrationStage.DeployClient) {
            Bootstrap memory b = p.bootstrap;
            _retainedRequire(
                address(
                    new BeefyClient(
                        b.sourceDomain,
                        p.chainId,
                        p.queue.account,
                        b.mmrStartBlock,
                        b.initialBeefyBlock,
                        b.initialSourceTimestampMs,
                        b.currentSet,
                        b.nextSet
                    )
                ) == created,
                "client address"
            );
        } else if (a.stage == MigrationStage.DeployVerifier) {
            _retainedRequire(
                address(new VaraQueueRootVerifier(BeefyClient(p.client.account), p.queue.account, p.chainId))
                    == created,
                "verifier address"
            );
        } else if (a.stage == MigrationStage.RegisterPause || a.stage == MigrationStage.RegisterUpgrade) {
            registerRetainedGovernanceRoot(
                p,
                a.delivery,
                a.stage == MigrationStage.RegisterPause ? GovernanceAction.Pause : GovernanceAction.Upgrade
            );
        } else if (a.stage == MigrationStage.Pause || a.stage == MigrationStage.Upgrade) {
            executeRetainedGovernance(
                p, a.delivery, a.stage == MigrationStage.Pause ? GovernanceAction.Pause : GovernanceAction.Upgrade
            );
        } else if (a.stage == MigrationStage.RegisterUnpause) {
            registerRetainedUnpauseRoot(p, a.delivery, a.barrier);
        } else if (a.stage == MigrationStage.Unpause) {
            executeRetainedUnpause(p, a.delivery, a.barrier);
        } else {
            (bool ok, bytes memory reason) = target.call(data);
            if (!ok) {
                assembly ("memory-safe") { revert(add(reason, 32), mload(reason)) }
            }
        }
        vm.stopBroadcast();
        _retainedActionPoststate(p, a);
    }

    function _retainedActionPrestate(MigrationPlan memory p, MigrationAction memory a, bool migrated) private {
        MessageQueue q = MessageQueue(p.queue.account);
        if (a.stage <= MigrationStage.Bootstrap) {
            _retainedRequire(!migrated && !q.paused(), "preparation is no longer permitted");
            if (a.stage == MigrationStage.DeployVerifier) _retainedClient(p, false);
            if (a.stage == MigrationStage.Bootstrap) {
                _retainedIdentity(p.implementation);
                _retainedIdentity(p.client);
                _retainedVerifier(p);
            }
            return;
        }
        if (a.stage == MigrationStage.EmptyBarrier) {
            verifyRetainedPoststate(p, true);
            _verifyRetainedFinalizedPoststate(p, a.barrier.upgradeReadback.state);
            _checkRetainedEmptyChronology(p, a.barrier);
            _retainedRequire(a.barrier.sourceBlock > q.maxBlockNumber(), "empty progress already present");
            return;
        }
        GovernanceAction action = a.stage == MigrationStage.RegisterPause || a.stage == MigrationStage.Pause
            ? GovernanceAction.Pause
            : a.stage == MigrationStage.RegisterUpgrade || a.stage == MigrationStage.Upgrade
                ? GovernanceAction.Upgrade
                : GovernanceAction.Unpause;
        _validateRetainedDelivery(p, a.delivery, action);
        bool registration = a.stage == MigrationStage.RegisterPause || a.stage == MigrationStage.RegisterUpgrade
            || a.stage == MigrationStage.RegisterUnpause;
        _retainedRequire(
            !q.isProcessed(a.delivery.message.nonce), "message already processed without original evidence"
        );
        if (action == GovernanceAction.Unpause) {
            _authenticateRetainedBarrier(p, a.barrier);
            _checkRetainedUnpauseChronology(p, a.delivery, a.barrier);
            verifyRetainedPoststate(p, true);
            if (!registration) _authenticateRetainedRootOriginal(p, a.delivery);
        } else {
            _retainedRequire(!migrated, "legacy action after upgrade");
            if (registration) _retainedRequire(!q.paused(), "legacy batch must precede pause");
            if (!registration) {
                _retainedRequire(q.maxBlockNumber() == p.legacyMax, "stale legacy maximum");
                _retainedRequire(q.paused() == (action == GovernanceAction.Upgrade), "unexpected pause state");
                if (action == GovernanceAction.Upgrade) {
                    _retainedIdentity(p.implementation);
                    _retainedClient(p, true);
                    _retainedVerifier(p);
                }
            }
        }
        _retainedRequire(
            registration
                ? q.getMerkleRoot(a.delivery.sourceBlock) == 0
                : q.getMerkleRoot(a.delivery.sourceBlock) == a.delivery.root,
            "publication state mismatch"
        );
    }

    function _retainedActionPoststate(MigrationPlan memory p, MigrationAction memory a) private view {
        if (a.stage == MigrationStage.DeployImplementation) {
            _retainedIdentity(p.implementation);
        } else if (a.stage == MigrationStage.DeployClient) {
            _retainedClient(p, BeefyClient(p.client.account).latestMMRRoot() != 0);
        } else if (a.stage == MigrationStage.DeployVerifier) {
            _retainedVerifier(p);
        } else if (a.stage == MigrationStage.Bootstrap) {
            _retainedIdentity(p.client);
        } else if (a.stage == MigrationStage.Upgrade) {
            verifyRetainedPoststate(p, true);
        } else if (a.stage == MigrationStage.Unpause) {
            verifyRetainedPoststate(p, false);
        } else if (a.stage == MigrationStage.EmptyBarrier) {
            verifyRetainedPoststate(p, true);
            _retainedRequire(
                MessageQueue(p.queue.account).maxBlockNumber() >= a.barrier.sourceBlock, "empty progress absent"
            );
        } else if (a.stage == MigrationStage.Pause) {
            validateRetainedMigration(p, false);
            _retainedRequire(
                MessageQueue(p.queue.account).paused()
                    && MessageQueue(p.queue.account).isProcessed(a.delivery.message.nonce),
                "pause absent"
            );
        } else {
            _retainedRequire(
                MessageQueue(p.queue.account).getMerkleRoot(a.delivery.sourceBlock) == a.delivery.root,
                "root publication absent"
            );
        }
    }

    function _retainedImplementation(address queue) private view returns (address) {
        return address(uint160(uint256(vm.load(queue, ERC1967Utils.IMPLEMENTATION_SLOT))));
    }

    /// @dev Matching state or a competing executor never disposes of an approved original.
    function _authenticateRetainedPredecessor(MigrationPlan memory p, MigrationAction memory a) internal {
        if (a.stage < MigrationStage.Verify) return;
        GovernanceDelivery memory delivery = a.upgrade.delivery;
        _validateRetainedDelivery(p, delivery, GovernanceAction.Upgrade);
        OriginalTransaction memory original = a.upgrade.original;
        string memory artifact = vm.readFile(original.artifact);
        string memory entry = string.concat(".transactions[", vm.toString(original.transactionIndex), "].transaction");
        uint256 nonce = vm.parseJsonUint(artifact, string.concat(entry, ".nonce"));
        _retainedRequire(nonce <= type(uint64).max, "upgrade nonce overflow");
        FinalizedPin memory included = _authenticateRetainedOriginal(
            original,
            vm.parseJsonAddress(artifact, string.concat(entry, ".from")),
            uint64(nonce),
            p.queue.account,
            abi.encodeCall(
                IMessageQueue.processMessage,
                (delivery.sourceBlock, delivery.totalLeaves, delivery.leafIndex, delivery.message, delivery.proof)
            ),
            address(0)
        );
        FinalizedPin memory readback =
            a.stage == MigrationStage.Verify ? a.upgradeReadback : a.barrier.upgradeReadback.state;
        _retainedRequire(
            included.number <= readback.number && included.timestamp <= readback.timestamp
                && MessageQueue(p.queue.account).isProcessed(delivery.message.nonce),
            "upgrade original outside qualified readback"
        );
        if (a.stage > MigrationStage.Verify) {
            _authenticateRetainedObservation(
                "RETAINED_UPGRADE_OBSERVATION",
                a.planHash,
                MigrationStage.Upgrade,
                original.transactionHash,
                a.barrier.upgradeReadback
            );
        }
        if (a.stage >= MigrationStage.RegisterUnpause) {
            _authenticateRetainedObservation(
                "RETAINED_BARRIER_OBSERVATION",
                a.planHash,
                MigrationStage.EmptyBarrier,
                a.barrier.original.transactionHash,
                a.barrier.publication
            );
        }
    }

    /// @dev An independently approved capture authenticates WHEN qualification was observed.
    /// Rechecking a currently-finalized header alone cannot establish historical finality timing.
    function _authenticateRetainedObservation(
        string memory variable,
        bytes32 planHash,
        MigrationStage stage,
        bytes32 transactionHash,
        FinalityObservation memory observation
    ) private {
        string memory json = vm.readFile(vm.envString(variable));
        _retainedRequire(
            keccak256(bytes(json)) == vm.envBytes32(string.concat(variable, "_HASH"))
                && vm.parseJsonUint(json, ".schemaVersion") == 1 && vm.parseJsonBytes32(json, ".planHash") == planHash
                && vm.parseJsonUint(json, ".stage") == uint256(stage) && transactionHash != 0
                && vm.parseJsonBytes32(json, ".transactionHash") == transactionHash
                && keccak256(vm.parseJsonBytes(json, ".observation")) == keccak256(abi.encode(observation)),
            "finality observation changed"
        );
        _retainedObservationTime(observation);
        _authenticateRetainedPin(observation.state);
        string memory head = _retainedBlock(_retainedHex(observation.head.number));
        _retainedRequire(
            vm.parseJsonBytes32(head, ".hash") == observation.head.blockHash
                && vm.parseJsonUint(head, ".number") == observation.head.number
                && vm.parseJsonUint(head, ".timestamp") == observation.head.timestamp,
            "observation head no longer canonical"
        );
    }

    /// @dev Called only after original receipt AND archived poststate qualification. The host clock and qualified
    /// RPC are operator trust boundaries; the emitted record requires independent approval before source authorization.
    function _captureRetainedObservation(
        bytes32 planHash,
        MigrationStage stage,
        bytes32 transactionHash,
        FinalizedPin memory state
    ) private {
        string memory path = vm.envString("RETAINED_FINALITY_OUTPUT");
        _retainedRequire(!vm.exists(path), "retain existing finality observation; do not overwrite");
        // Reserve before RPC work: concurrent qualifications must never replace an already captured observation.
        string[] memory command = new string[](2);
        command[0] = "/bin/mkdir";
        command[1] = string.concat(path, ".capture");
        Vm.FfiResult memory acquired = vm.tryFfi(command);
        _retainedRequire(acquired.exitCode == 0, "finality observation output already reserved");
        _authenticateRetainedPin(state);
        string memory latest = _retainedBlock("latest");
        FinalityObservation memory observation;
        observation.state = state;
        observation.head = FinalizedPin(
            vm.parseJsonUint(latest, ".number"),
            vm.parseJsonBytes32(latest, ".hash"),
            vm.parseJsonUint(latest, ".timestamp")
        );
        observation.observedAt = vm.unixTime() / 1000;
        if (observation.observedAt < observation.head.timestamp) observation.observedAt = observation.head.timestamp;
        _retainedObservationTime(observation);
        bytes memory encoded = abi.encode(observation);
        string memory json = string.concat(
            '{"schemaVersion":1,"planHash":"',
            vm.toString(planHash),
            '","stage":',
            vm.toString(uint256(stage)),
            ',"transactionHash":"',
            vm.toString(transactionHash),
            '","observation":"',
            vm.toString(encoded),
            '"}'
        );
        vm.writeFile(path, json);
        emit RetainedFinalityObserved(planHash, stage, keccak256(bytes(json)), encoded);
    }

    function _retainedRpc(string memory method, string memory params) internal virtual returns (string memory) {
        return vm.rpcJson(vm.envString("RETAINED_RPC_URL"), method, params);
    }

    function _retainedBlock(string memory tag) private returns (string memory) {
        return _retainedRpc("eth_getBlockByNumber", string.concat('["', tag, '",false]'));
    }

    function _retainedHex(uint256 value) private pure returns (string memory) {
        bytes16 digits = "0123456789abcdef";
        uint256 length = 1;
        for (uint256 v = value; v > 15; v >>= 4) {
            ++length;
        }
        bytes memory encoded = new bytes(length + 2);
        encoded[0] = "0";
        encoded[1] = "x";
        for (uint256 i = length + 1; i > 1; --i) {
            encoded[i] = digits[value & 15];
            value >>= 4;
        }
        return string(encoded);
    }

    function _authenticateRetainedNetwork(MigrationPlan memory p) private {
        _retainedRequire(vm.parseJsonBytes32(_retainedBlock("0x0"), ".hash") == p.ethereumGenesis, "RPC genesis");
        string memory head = _retainedBlock(_retainedHex(block.number));
        _retainedRequire(
            vm.parseJsonUint(head, ".number") == block.number && vm.parseJsonUint(head, ".timestamp") == block.timestamp
                && vm.parseJsonBytes32(head, ".parentHash") == blockhash(block.number - 1),
            "stale/wrong RPC execution view"
        );
        _retainedRequire(vm.parseJsonUint(_retainedRpc("eth_chainId", "[]"), "$") == p.chainId, "RPC chain");
    }

    function _retainedNonce(address signer, string memory tag) private returns (uint256) {
        return vm.parseJsonUint(
            _retainedRpc("eth_getTransactionCount", string.concat('["', vm.toString(signer), '","', tag, '"]')), "$"
        );
    }

    function _authenticateRetainedPin(FinalizedPin memory pin) private {
        _retainedRequire(pin.blockHash != 0 && pin.number != 0, "missing finalized pin");
        string memory finalized = _retainedBlock("finalized");
        bytes32 finalizedHash = vm.parseJsonBytes32(finalized, ".hash");
        _retainedRequire(
            finalizedHash != 0
                && vm.parseJsonBytes32(_retainedBlock(_retainedHex(vm.parseJsonUint(finalized, ".number"))), ".hash")
                    == finalizedHash,
            "invalid finalized anchor"
        );
        string memory header = _retainedBlock(_retainedHex(pin.number));
        _retainedRequire(
            vm.parseJsonUint(finalized, ".number") >= pin.number
                && vm.parseJsonBytes32(header, ".hash") == pin.blockHash
                && vm.parseJsonUint(header, ".number") == pin.number
                && vm.parseJsonUint(header, ".timestamp") == pin.timestamp,
            "noncanonical/unfinalized pin"
        );
    }

    function _verifyRetainedFinalizedPoststate(MigrationPlan memory p, FinalizedPin memory pin) private {
        _authenticateRetainedPin(pin);
        uint256 originalFork = vm.activeFork();
        vm.createSelectFork(vm.envString("RETAINED_RPC_URL"), pin.number);
        _authenticateRetainedSourceCut(p, MigrationStage.Verify);
        verifyRetainedPoststate(p, true);
        vm.selectFork(originalFork);
        _authenticateRetainedPin(pin);
    }

    /// @dev Original success is never inferred from a processed bit, a rival transaction, or matching latest state.
    /// RPC absence, revert, nonfinal inclusion, changed bytes, or missing artifact all HOLD without a new signature.
    function _validateRetainedOriginalRpc(
        bytes32 hash,
        address signer,
        uint64 nonce,
        address target,
        bytes memory input,
        address created,
        string memory transaction,
        string memory receipt
    ) internal pure returns (FinalizedPin memory included) {
        _retainedRequire(
            vm.parseJsonBytes32(transaction, ".hash") == hash && vm.parseJsonAddress(transaction, ".from") == signer
                && vm.parseJsonUint(transaction, ".nonce") == nonce && vm.parseJsonUint(transaction, ".value") == 0
                && keccak256(vm.parseJsonBytes(transaction, ".input")) == keccak256(input),
            "original transaction changed"
        );
        if (created == address(0)) {
            _retainedRequire(
                vm.parseJsonAddress(transaction, ".to") == target && vm.parseJsonAddress(receipt, ".to") == target,
                "original target"
            );
        } else {
            _retainedRequire(
                keccak256(vm.parseJson(transaction, ".to")) == keccak256(abi.encode(bytes32(0)))
                    && vm.parseJsonAddress(receipt, ".contractAddress") == created,
                "original constructor identity"
            );
        }
        _retainedRequire(
            vm.parseJsonBytes32(receipt, ".transactionHash") == hash && vm.parseJsonAddress(receipt, ".from") == signer
                && vm.parseJsonUint(receipt, ".status") == 1,
            "original absent/reverted"
        );
        included.number = vm.parseJsonUint(receipt, ".blockNumber");
        included.blockHash = vm.parseJsonBytes32(receipt, ".blockHash");
        _retainedRequire(
            vm.parseJsonUint(transaction, ".blockNumber") == included.number
                && vm.parseJsonBytes32(transaction, ".blockHash") == included.blockHash,
            "original inclusion changed"
        );
    }

    function _authenticateRetainedOriginal(
        OriginalTransaction memory original,
        address signer,
        uint64 nonce,
        address target,
        bytes memory input,
        address created
    ) internal returns (FinalizedPin memory included) {
        _retainedRequire(original.transactionHash != 0 && original.artifactHash != 0, "missing original identity");
        string memory artifact = vm.readFile(original.artifact);
        _retainedRequire(keccak256(bytes(artifact)) == original.artifactHash, "Foundry artifact changed");
        string memory entry = string.concat(".transactions[", vm.toString(original.transactionIndex), "]");
        _retainedRequire(
            vm.parseJsonBytes32(artifact, string.concat(entry, ".hash")) == original.transactionHash,
            "artifact original hash"
        );
        string memory txPath = string.concat(entry, ".transaction");
        _retainedRequire(
            vm.parseJsonAddress(artifact, string.concat(txPath, ".from")) == signer
                && vm.parseJsonUint(artifact, string.concat(txPath, ".nonce")) == nonce
                && keccak256(vm.parseJsonBytes(artifact, string.concat(txPath, ".input"))) == keccak256(input),
            "artifact transaction intent"
        );
        _retainedRequire(
            vm.parseJsonUint(artifact, string.concat(txPath, ".chainId")) == block.chainid
                && vm.parseJsonUint(artifact, string.concat(txPath, ".value")) == 0,
            "artifact chain/value"
        );
        string memory hashParam = string.concat('["', vm.toString(original.transactionHash), '"]');
        string memory txJson = _retainedRpc("eth_getTransactionByHash", hashParam);
        string memory receipt = _retainedRpc("eth_getTransactionReceipt", hashParam);
        _retainedRequire(vm.parseJsonUint(txJson, ".chainId") == block.chainid, "original chain");
        included = _validateRetainedOriginalRpc(
            original.transactionHash, signer, nonce, target, input, created, txJson, receipt
        );
        if (created == address(0)) {
            _retainedRequire(vm.parseJsonAddress(artifact, string.concat(txPath, ".to")) == target, "artifact target");
        } else {
            _retainedRequire(
                vm.parseJsonAddress(artifact, string.concat(entry, ".contractAddress")) == created,
                "artifact constructor identity"
            );
        }
        included.timestamp = vm.parseJsonUint(_retainedBlock(_retainedHex(included.number)), ".timestamp");
        _retainedRequire(
            vm.parseJsonUint(txJson, ".blockNumber") == included.number
                && vm.parseJsonBytes32(txJson, ".blockHash") == included.blockHash,
            "original inclusion changed"
        );
        _authenticateRetainedPin(included);
        uint256 txIndex = vm.parseJsonUint(receipt, ".transactionIndex");
        _retainedRequire(
            vm.parseJsonUint(txJson, ".transactionIndex") == txIndex
                && vm.parseJsonBytes32(
                    _retainedBlock(_retainedHex(included.number)),
                    string.concat(".transactions[", vm.toString(txIndex), "]")
                ) == original.transactionHash,
            "original absent from canonical block"
        );
        uint256 count = vm.parseJsonArrayLength(artifact, ".receipts");
        bool found;
        for (uint256 i; i < count; ++i) {
            string memory r = string.concat(".receipts[", vm.toString(i), "]");
            if (vm.parseJsonBytes32(artifact, string.concat(r, ".transactionHash")) != original.transactionHash) {
                continue;
            }
            _retainedRequire(
                !found && vm.parseJsonUint(artifact, string.concat(r, ".status")) == 1
                    && vm.parseJsonUint(artifact, string.concat(r, ".blockNumber")) == included.number
                    && vm.parseJsonBytes32(artifact, string.concat(r, ".blockHash")) == included.blockHash,
                "original artifact receipt mismatch"
            );
            found = true;
        }
        _retainedRequire(found, "original Foundry receipt missing");
    }

    function _authenticateRetainedBarrier(MigrationPlan memory p, EmptyBarrier memory barrier) private {
        _verifyRetainedFinalizedPoststate(p, barrier.upgradeReadback.state);
        _authenticateRetainedPin(barrier.publication.state);
        bytes memory input = abi.encodeCall(MessageQueue.submitEmptyQueueProgress, (barrier.sourceBlock, barrier.proof));
        string memory artifact = vm.readFile(barrier.original.artifact);
        string memory entry =
            string.concat(".transactions[", vm.toString(barrier.original.transactionIndex), "].transaction");
        uint256 nonce = vm.parseJsonUint(artifact, string.concat(entry, ".nonce"));
        _retainedRequire(nonce <= type(uint64).max, "barrier nonce overflow");
        FinalizedPin memory included = _authenticateRetainedOriginal(
            barrier.original,
            vm.parseJsonAddress(artifact, string.concat(entry, ".from")),
            uint64(nonce),
            p.queue.account,
            input,
            address(0)
        );
        _retainedRequire(
            included.number > barrier.upgradeReadback.state.number
                && included.number <= barrier.publication.state.number,
            "barrier original outside post-upgrade interval"
        );
        _checkRetainedEmptyChronology(p, barrier);
    }

    function _authenticateRetainedRootOriginal(MigrationPlan memory p, GovernanceDelivery memory d) private {
        OriginalTransaction memory original = OriginalTransaction(
            vm.envString("RETAINED_ROOT_ARTIFACT"),
            vm.envBytes32("RETAINED_ROOT_ARTIFACT_HASH"),
            vm.envUint("RETAINED_ROOT_TRANSACTION_INDEX"),
            vm.envBytes32("RETAINED_ROOT_TRANSACTION_HASH")
        );
        string memory artifact = vm.readFile(original.artifact);
        string memory entry = string.concat(".transactions[", vm.toString(original.transactionIndex), "].transaction");
        uint256 nonce = vm.parseJsonUint(artifact, string.concat(entry, ".nonce"));
        _retainedRequire(nonce <= type(uint64).max, "root nonce overflow");
        _authenticateRetainedOriginal(
            original,
            vm.parseJsonAddress(artifact, string.concat(entry, ".from")),
            uint64(nonce),
            p.queue.account,
            abi.encodeCall(IMessageQueue.submitMerkleRoot, (d.sourceBlock, d.root, d.rootProof)),
            address(0)
        );
    }

    /// @dev Timelock AccessControl is not enumerable: authenticate its full grant/revoke history, not only supplied members.
    function _authenticateRetainedRecoveryRoles(MigrationPlan memory p) private {
        string memory events = _retainedRpc(
            "eth_getLogs",
            string.concat(
                '[{"address":"',
                vm.toString(p.recovery.account),
                '","fromBlock":"0x0","toBlock":"latest","topics":[["',
                vm.toString(keccak256("RoleGranted(bytes32,address,address)")),
                '","',
                vm.toString(keccak256("RoleRevoked(bytes32,address,address)")),
                '"]]}]'
            )
        );
        uint256 count = vm.parseJsonArrayLength(events, "$");
        TimelockController recovery = TimelockController(payable(p.recovery.account));
        for (uint256 i; i < count; ++i) {
            string memory e = string.concat("$[", vm.toString(i), "]");
            bytes32 role = vm.parseJsonBytes32(events, string.concat(e, ".topics[1]"));
            address account = address(uint160(uint256(vm.parseJsonBytes32(events, string.concat(e, ".topics[2]")))));
            if (!recovery.hasRole(role, account)) continue;
            bool approved;
            for (uint256 j; j < p.recoveryRoles.length; ++j) {
                if (
                    p.recoveryRoles[j].role == role && p.recoveryRoles[j].account == account
                        && p.recoveryRoles[j].member
                ) approved = true;
            }
            _retainedRequire(approved, "unapproved live recovery role");
        }
    }

    function _authenticateRetainedDeploymentHistory(MigrationPlan memory p, MigrationStage stage) private {
        if (stage == MigrationStage.DeployImplementation) return;
        string memory json = vm.readFile(vm.envString("RETAINED_DEPLOYMENTS"));
        _retainedRequire(
            keccak256(bytes(json)) == vm.envBytes32("RETAINED_DEPLOYMENTS_HASH"), "deployment evidence changed"
        );
        uint256 required = stage == MigrationStage.DeployClient ? 1 : stage == MigrationStage.DeployVerifier ? 2 : 3;
        for (uint256 i; i < required; ++i) {
            string memory key = i == 0 ? ".implementation" : i == 1 ? ".client" : ".verifier";
            OriginalTransaction memory original = abi.decode(vm.parseJsonBytes(json, key), (OriginalTransaction));
            string memory artifact = vm.readFile(original.artifact);
            string memory entry =
                string.concat(".transactions[", vm.toString(original.transactionIndex), "].transaction");
            uint256 nonce = vm.parseJsonUint(artifact, string.concat(entry, ".nonce"));
            address signer = vm.parseJsonAddress(artifact, string.concat(entry, ".from"));
            _retainedRequire(nonce <= type(uint64).max, "deployment nonce overflow");
            address created = i == 0 ? p.implementation.account : i == 1 ? p.client.account : p.verifier.account;
            _retainedRequire(vm.computeCreateAddress(signer, nonce) == created, "original deployment address");
            _authenticateRetainedOriginal(
                original,
                signer,
                uint64(nonce),
                address(0),
                _retainedCreationCode(p, MigrationStage(uint256(MigrationStage.DeployImplementation) + i)),
                created
            );
        }
    }

    function _retainedProxyInventory(MigrationPlan memory p) private view {
        for (uint256 i; i < p.retainedProxies.length; ++i) {
            ProxyPin memory pin = p.retainedProxies[i];
            _retainedIdentity(pin.implementation);
            _retainedRequire(
                pin.proxy != p.queue.account && pin.proxy.code.length != 0
                    && address(uint160(uint256(vm.load(pin.proxy, pin.implementationSlot))))
                        == pin.implementation.account,
                "retained implementation changed"
            );
        }
        _retainedProxySlot(p, p.manager.account, ERC1967Utils.IMPLEMENTATION_SLOT);
        _retainedProxySlot(p, p.wrappedVara.account, ERC1967Utils.IMPLEMENTATION_SLOT);
        for (uint256 i; i < p.tokens.length; ++i) {
            _retainedProxySlot(p, p.tokens[i].account, ERC1967Utils.IMPLEMENTATION_SLOT);
            _retainedProxySlot(p, p.tokens[i].account, keccak256("org.zeppelinos.proxy.implementation"));
        }
    }

    function _retainedProxySlot(MigrationPlan memory p, address proxy, bytes32 slot) private view {
        if (vm.load(proxy, slot) == 0) return;
        bool found;
        for (uint256 i; i < p.retainedProxies.length; ++i) {
            if (p.retainedProxies[i].proxy == proxy && p.retainedProxies[i].implementationSlot == slot) found = true;
        }
        _retainedRequire(found, "retained proxy implementation not approved");
    }

    function _authenticateRetainedSourceCut(MigrationPlan memory p, MigrationStage stage) internal view {
        string memory cut = vm.readFile(vm.envString("RETAINED_SOURCE_CUT"));
        _retainedRequire(
            keccak256(bytes(cut)) == p.sourceCutApproval && vm.parseJsonAddress(cut, ".queue") == p.queue.account
                && vm.parseJsonBytes32(cut, ".sourceDomain") == p.bootstrap.sourceDomain
                && vm.parseJsonUint(cut, ".legacyMax") == p.legacyMax
                && vm.parseJsonUint(cut, ".firstBeefyBlock") == p.firstBeefyBlock,
            "source cut changed"
        );
        LegacyRootPin[] memory roots = abi.decode(vm.parseJsonBytes(cut, ".roots"), (LegacyRootPin[]));
        _retainedRequire(roots.length != 0, "source cut has no coverage");
        MessageQueue q = MessageQueue(p.queue.account);
        bool requireCoverage = stage >= MigrationStage.Pause || q.paused()
            || _retainedImplementation(p.queue.account) == p.implementation.account;
        for (uint256 i; i < roots.length; ++i) {
            _retainedRequire(
                roots[i].sourceBlock >= p.genesisBlock && roots[i].sourceBlock <= p.legacyMax && roots[i].root != 0,
                "cut coverage outside legacy range"
            );
            bytes32 actualRoot = q.getMerkleRoot(roots[i].sourceBlock);
            if (requireCoverage || actualRoot != 0) {
                _retainedRequire(actualRoot == roots[i].root, "unsettled/conflicting cut root");
            }
        }
        string memory inventory = vm.readFile(vm.envString("RETAINED_UNPAUSE_INVENTORY"));
        _retainedRequire(
            keccak256(bytes(inventory)) == p.outstandingUnpauseInventory
                && vm.parseJsonAddress(inventory, ".queue") == p.queue.account
                && vm.parseJsonBytes32(inventory, ".sourcePauser") == p.sourcePauser,
            "unpause inventory changed"
        );
        uint256[] memory nonces = vm.parseJsonUintArray(inventory, ".unpauseNonces");
        for (uint256 i; i < nonces.length; ++i) {
            _retainedRequire(q.isProcessed(nonces[i]), "outstanding prior unpause requires governance reconciliation");
        }
    }
}

/// @dev Explicit retained-proxy entrypoint; fresh deployment never dispatches here.
contract RetainedBridgeMigrationScript is Script, RetainedBridgeMigration {
    function run() public returns (MessageQueue) {
        require(
            keccak256(bytes(vm.envOr("DEPLOYMENT_MODE", string("")))) == keccak256("retained-beefy-migration"),
            "DEPLOYMENT_MODE must be retained-beefy-migration"
        );
        return runRetainedMigrationFromEnvironment();
    }
}
