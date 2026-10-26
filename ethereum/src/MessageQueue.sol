// Copyright (C) Gear Technologies Inc.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {AccessControlUpgradeable} from "@openzeppelin/contracts-upgradeable/access/AccessControlUpgradeable.sol";
import {PausableUpgradeable} from "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import {TimelockController} from "@openzeppelin/contracts/governance/TimelockController.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {Initializable} from "@openzeppelin/contracts/proxy/utils/Initializable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts/proxy/utils/UUPSUpgradeable.sol";
import {EnumerableSet} from "@openzeppelin/contracts/utils/structs/EnumerableSet.sol";
import {VaraQueueRootVerifier} from "src/VaraQueueRootVerifier.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {SubstrateMerkleProof} from "src/beefy/utils/SubstrateMerkleProof.sol";
import {IGovernance} from "src/interfaces/IGovernance.sol";
import {IMessageHandler} from "src/interfaces/IMessageHandler.sol";
import {IMessageQueue, LegacyControl, VaraMessage} from "src/interfaces/IMessageQueue.sol";
import {IPausable} from "src/interfaces/IPausable.sol";
import {IQueueProgressVerifier} from "src/interfaces/IQueueProgressVerifier.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";
import {Hasher} from "src/libraries/Hasher.sol";

/**
 * @dev MessageQueue smart contract is responsible for storing Merkle roots for blocks
 *      that were observed on Vara Network. Before storing Merkle roots, MessageQueue
 *      verifies received Merkle roots with help of Verifier smart contract.
 *
 *      MessageQueue smart contract is also responsible for verifying and processing
 *      received messages originated from Vara Network.
 */
contract MessageQueue is
    Initializable,
    AccessControlUpgradeable,
    PausableUpgradeable,
    UUPSUpgradeable,
    IPausable,
    IMessageQueue
{
    using EnumerableSet for EnumerableSet.AddressSet;

    using Hasher for VaraMessage;

    bytes32 public constant PAUSER_ROLE = bytes32(uint256(0x01));

    uint256 public constant CHALLENGE_ROOT_DELAY = 2 days;

    uint256 public constant PROCESS_ADMIN_MESSAGE_DELAY = 1 hours;
    uint256 public constant PROCESS_PAUSER_MESSAGE_DELAY = 5 minutes;
    uint256 public constant PROCESS_USER_MESSAGE_DELAY = 5 minutes;

    uint256 public constant MAX_BLOCK_DISTANCE = 57_600;

    IGovernance private _governanceAdmin;
    IGovernance private _governancePauser;
    address private _emergencyStopAdmin;
    EnumerableSet.AddressSet private _emergencyStopObservers;
    IVerifier private _verifier;
    uint256 private _challengingRootTimestamp;
    bool private _emergencyStop;
    uint256 private _genesisBlock;
    uint256 private _maxBlockNumber;
    bool private _allowMessageProcessing;
    mapping(uint256 blockNumber => bytes32 merkleRoot) private _blockNumbers;
    mapping(bytes32 merkleRoot => uint256 timestamp) private _merkleRootTimestamps;
    mapping(uint256 messageNonce => bool isProcessed) private _processedMessages;

    uint256 public beefyRootMinimum;
    mapping(uint256 blockNumber => uint256 timestamp) private _blockRootTimestamps;
    address public beefyRecoveryTimelock;

    // Append-only: retained proxy slots above must never move.
    struct ControlEnvelope {
        uint8 version;
        uint64 sequence;
        uint64 validUntil;
        uint8 action;
        address target;
        uint64 incidentEpoch;
        bytes32 requiredAdminAction;
        bytes32 requiredBarrier;
        bytes innerPayload;
    }

    struct AdminCompletion {
        address target;
        uint64 incidentEpoch;
    }

    struct Publication {
        uint64 queueId;
        uint64 sourceTimestampMs;
        uint256 destinationBlock;
        bytes32 root;
    }
    mapping(bytes32 => uint64) public executedControlSequence;
    mapping(bytes32 => uint64) public canceledControlSequence;
    mapping(bytes32 => bool) public retiredGovernanceStream;
    mapping(address => uint64) public incidentEpoch;
    mapping(bytes32 => AdminCompletion) public completedAdminActions;
    bool public legacyPublicationFrozen;
    uint256 public frozenLegacyMaxBlock;
    bytes32 public stagedMigrationMessage;
    bytes32 public stagedRecoveryMessage;
    mapping(bytes32 => uint64) private _legacyControlExpiry;
    bytes32 private _executingAdminMessage;
    bytes32 public migrationAdminAction;
    uint256 public migrationDestinationBlock;
    mapping(uint256 => Publication) public publications;
    bytes32 public migrationBarrier;
    uint256 public migrationBarrierSourceBlock;
    uint256 public migrationTimestamp;
    bool public migrationResumePending;
    mapping(bytes32 => bytes32) private _capturedMigrationPublicationHashes;

    /**
     * @custom:oz-upgrades-unsafe-allow constructor
     */
    constructor() {
        _disableInitializers();
    }

    /**
     * @dev Initializes the MessageQueue contract with the Verifier address.
     *      GovernanceAdmin contract is used to upgrade, pause/unpause the MessageQueue contract.
     *      GovernancePauser contract is used to pause/unpause the MessageQueue contract.
     * @dev Fresh BEEFY deployments validate the MessageQueue binding and pin the authenticated source floor.
     * @param governanceAdmin_ The address of the GovernanceAdmin contract that will process messages.
     * @param governancePauser_ The address of the GovernanceAdmin contract that will process pauser messages.
     * @param emergencyStopAdmin_ The address of EOA that will control `submitMerkleRoot` and `processMessage`
     *                            in case of an emergency stop.
     * @param verifier_ The address of the Verifier contract that will verify merkle roots.
     */
    function initializeBeefy(
        IGovernance governanceAdmin_,
        IGovernance governancePauser_,
        address emergencyStopAdmin_,
        address[] memory emergencyStopObservers_,
        IVerifier verifier_,
        address recoveryTimelock_
    ) public initializer {
        _initialize(governanceAdmin_, governancePauser_, emergencyStopAdmin_, emergencyStopObservers_, verifier_);
        (, uint64 mmrStartBlock) = _validateBeefyVerifier(_verifier);
        beefyRootMinimum = mmrStartBlock;
        _setBeefyRecoveryTimelock(recoveryTimelock_);
    }

    /// @dev Atomically switch a paused legacy proxy without resetting custody, roles, roots or replay state.
    function migrateToBeefy(
        IVerifier expectedLegacyVerifier,
        IVerifier beefyVerifier,
        uint256 expectedLegacyMaxBlock,
        uint256 firstBeefyBlock,
        address recoveryTimelock_
    ) external onlyRole(DEFAULT_ADMIN_ROLE) reinitializer(8) {
        _requirePaused();
        if (
            !legacyPublicationFrozen || frozenLegacyMaxBlock != expectedLegacyMaxBlock
                || _executingAdminMessage != stagedMigrationMessage || _executingAdminMessage == bytes32(0)
                || beefyRootMinimum != 0 || _genesisBlock == 0 || address(_verifier) != address(expectedLegacyVerifier)
                || _maxBlockNumber != expectedLegacyMaxBlock
        ) {
            revert InvalidBeefyMigration();
        }

        _verifier = beefyVerifier;
        (BeefyClient client, uint64 mmrStartBlock) = _validateBeefyVerifier(_verifier);
        if (
            firstBeefyBlock != expectedLegacyMaxBlock + 1 || firstBeefyBlock > type(uint32).max
                || firstBeefyBlock < mmrStartBlock || !client.isLive() || client.latestMMRRoot() == bytes32(0)
                || client.latestBeefyBlock() <= firstBeefyBlock
        ) {
            revert InvalidBeefyMigration();
        }
        beefyRootMinimum = firstBeefyBlock;
        _setBeefyRecoveryTimelock(recoveryTimelock_);
        migrationAdminAction = _executingAdminMessage;
        migrationDestinationBlock = block.number;
        migrationTimestamp = block.timestamp;
        migrationResumePending = true;
        _clearLegacyStage();
        emit MigratedToBeefy(
            address(expectedLegacyVerifier), address(beefyVerifier), expectedLegacyMaxBlock, firstBeefyBlock
        );
    }

    /// @dev Independently scheduled recovery only after expiry; retained claims and replay state are unchanged.
    function recoverBeefyVerifier(IVerifier expectedVerifier, IVerifier replacementVerifier, uint256 expectedMaxBlock)
        external
    {
        if (msg.sender != beefyRecoveryTimelock) revert NotBeefyRecoveryTimelock();
        if (
            beefyRootMinimum == 0 || address(_verifier) != address(expectedVerifier)
                || _maxBlockNumber != expectedMaxBlock || TimelockController(payable(msg.sender)).getMinDelay() == 0
        ) revert InvalidBeefyRecovery();
        (BeefyClient previous, uint64 start) = _validateBeefyVerifier(_verifier);
        if (previous.isLive()) revert InvalidBeefyRecovery();

        _verifier = replacementVerifier;
        (BeefyClient replacement, uint64 replacementStart) = _validateBeefyVerifier(_verifier);
        if (
            replacementStart != start || replacement.sourceDomain() != previous.sourceDomain() || !replacement.isLive()
                || replacement.latestMMRRoot() == bytes32(0)
                || replacement.latestBeefyBlock() <= previous.latestBeefyBlock()
                || replacement.latestBeefyBlock() <= _maxBlockNumber
        ) revert InvalidBeefyRecovery();
        ++incidentEpoch[address(this)];
        if (!paused()) _pause();
        emit BeefyVerifierRecovered(address(expectedVerifier), address(replacementVerifier), expectedMaxBlock);
    }

    /**
     * @dev Returns governance admin address.
     * @return governanceAdmin Governance admin address.
     */
    function governanceAdmin() external view returns (address) {
        return address(_governanceAdmin);
    }

    /**
     * @dev Returns governance pauser address.
     * @return governancePauser Governance pauser address.
     */
    function governancePauser() external view returns (address) {
        return address(_governancePauser);
    }

    /**
     * @dev Returns emergency stop admin address.
     * @return emergencyStopAdmin Emergency stop admin address.
     */
    function emergencyStopAdmin() external view returns (address) {
        return _emergencyStopAdmin;
    }

    /**
     * @dev Returns list of emergency stop observers.
     * @return emergencyStopObservers List of emergency stop observers.
     */
    function emergencyStopObservers() external view returns (address[] memory) {
        return _emergencyStopObservers.values();
    }

    /**
     * @dev Returns verifier address.
     * @return verifier Verifier address.
     */
    function verifier() external view returns (address) {
        return address(_verifier);
    }

    /**
     * @dev Returns challenging root status.
     * @return isChallengingRoot challenging root status.
     */
    function isChallengingRoot() public view returns (bool) {
        // forge-lint: disable-next-item(block-timestamp)
        return block.timestamp < _challengingRootTimestamp
            || block.timestamp - _challengingRootTimestamp < CHALLENGE_ROOT_DELAY;
    }

    /**
     * @dev Returns emergency stop status.
     * @return isEmergencyStopped emergency stop status.
     */
    function isEmergencyStopped() external view returns (bool) {
        return _emergencyStop;
    }

    /**
     * @dev Returns genesis block number.
     * @return genesisBlock Genesis block number.
     */
    function genesisBlock() external view returns (uint256) {
        return _genesisBlock;
    }

    /**
     * @dev Returns maximum block number.
     * @return maxBlockNumber Maximum block number.
     */
    function maxBlockNumber() external view returns (uint256) {
        return _maxBlockNumber;
    }

    /**
     * @dev Pauses the contract.
     */
    function pause() public onlyRole(PAUSER_ROLE) {
        _pause();
    }

    /**
     * @dev Unpauses the contract.
     */
    function unpause() public onlyRole(PAUSER_ROLE) {
        _unpause();
    }

    /**
     * @dev Function that should revert when `msg.sender` is not authorized to upgrade the contract.
     *      Called by {upgradeToAndCall}.
     */
    function _authorizeUpgrade(address newImplementation) internal override onlyRole(DEFAULT_ADMIN_ROLE) {}

    /**
     * @dev Puts MessageQueue into a high-priority paused state.
     *      Only the emergency stop admin or time expiry (CHALLENGE_ROOT_DELAY) can lift it.
     *
     * @dev Reverts if:
     *      - msg.sender is not emergency stop observer with `NotEmergencyStopObserver` error.
     *
     * @dev Emits `ChallengeRootEnabled(block.timestamp + CHALLENGE_ROOT_DELAY)` event.
     */
    function challengeRoot() external {
        if (!_emergencyStopObservers.contains(msg.sender)) {
            revert NotEmergencyStopObserver();
        }

        _challengingRootTimestamp = block.timestamp;

        emit ChallengeRootEnabled(block.timestamp + CHALLENGE_ROOT_DELAY);
    }

    /**
     * @dev Disables challenging root status.
     *
     * @dev Reverts if:
     *      - msg.sender is not emergency stop admin with `NotEmergencyStopAdmin` error.
     *      - challenging root status is not enabled with `ChallengeRootNotEnabled` error.
     *
     * @dev Emits `ChallengeRootDisabled` event.
     */
    function disableChallengeRoot() external {
        if (msg.sender != _emergencyStopAdmin) {
            revert NotEmergencyStopAdmin();
        }

        if (!isChallengingRoot()) {
            revert ChallengeRootNotEnabled();
        }

        _challengingRootTimestamp = 0;

        emit ChallengeRootDisabled();
    }

    /**
     * @dev Allows message processing when emergency stop is enabled.
     *
     * @dev Reverts if:
     *      - msg.sender is not emergency stop admin with `NotEmergencyStopAdmin` error.
     *      - emergency stop status is not enabled with `EmergencyStopNotEnabled` error.
     */
    function allowMessageProcessing() external {
        if (msg.sender != _emergencyStopAdmin) {
            revert NotEmergencyStopAdmin();
        }

        if (!_emergencyStop) {
            revert EmergencyStopNotEnabled();
        }

        _allowMessageProcessing = true;

        emit MessageProcessingAllowed();
    }

    /// forge-lint: disable-next-item(cyclomatic-complexity)
    /**
     * @dev Adds emergency stop observer.
     *
     * @param observer Address of observer to add.
     *
     * @dev Reverts if:
     *      - msg.sender is not emergency stop admin with `NotEmergencyStopAdmin` error.
     *
     * @dev Emits `EmergencyStopObserverAdded` event if observer was not already present.
     */
    function addEmergencyStopObserver(address observer) external {
        if (msg.sender != _emergencyStopAdmin) {
            revert NotEmergencyStopAdmin();
        }

        if (_emergencyStopObservers.add(observer)) {
            emit EmergencyStopObserverAdded(observer);
        }
    }

    /**
     * @dev Removes emergency stop observer.
     *
     * @param observer Address of observer to remove.
     *
     * @dev Reverts if:
     *      - msg.sender is not emergency stop admin with `NotEmergencyStopAdmin` error.
     *
     * @dev Emits `EmergencyStopObserverRemoved` event if observer was present.
     *
     * @dev Note: If removed observer had active challenge via `challengeRoot`, it remains active
     *      until `disableChallengeRoot` is called or `CHALLENGE_ROOT_DELAY` (2 days) expires.
     */
    function removeEmergencyStopObserver(address observer) external {
        if (msg.sender != _emergencyStopAdmin) {
            revert NotEmergencyStopAdmin();
        }

        if (_emergencyStopObservers.remove(observer)) {
            emit EmergencyStopObserverRemoved(observer);
        }
    }

    /// forge-lint: disable-next-item(cyclomatic-complexity)
    /**
     * @dev Receives, verifies and stores Merkle roots from Vara Network.
     *
     *      Upon successfully storing data about block number and corresponding Merkle root,
     *      MessageQueue smart contract will emit a `MerkleRoot` event.
     *
     *      It is important to note that anyone can submit a Merkle root because only
     *      validated Merkle roots will be stored in the MessageQueue smart contract.
     *
     * @param blockNumber Block number on Vara Network.
     * @param merkleRoot Merkle root of transactions included in block with corresponding block number.
     * @param proof Canonical v2 BEEFY-authenticated MMR queue proof envelope.
     * @dev Reverts if challenging root status is enabled and caller is not emergency stop admin with `ChallengeRoot` error.
     * @dev Reverts if emergency stop status is set and caller is not emergency stop admin with `EmergencyStop` error.
     * @dev Reverts if the queue proof is invalid or malformed with `InvalidQueueRootProof`.
     * @dev Reverts if block number is too far from max block number with `BlockNumberTooFar` error.
     */
    function submitMerkleRoot(uint256 blockNumber, bytes32 merkleRoot, bytes calldata proof) external {
        if (legacyPublicationFrozen) revert LegacyPublicationFrozen();
        if (merkleRoot == bytes32(0)) {
            revert InvalidMerkleRoot();
        }
        bool isFromEmergencyStopAdmin = msg.sender == _emergencyStopAdmin;

        if (isChallengingRoot() && !isFromEmergencyStopAdmin) {
            revert ChallengeRoot();
        }

        if (_emergencyStop && !isFromEmergencyStopAdmin) {
            revert EmergencyStop();
        }

        bytes32 previousMerkleRoot = _blockNumbers[blockNumber];
        _checkSubmissionBlock(blockNumber, false);

        if (!_verifyPublication(blockNumber, merkleRoot, proof)) {
            revert InvalidQueueRootProof();
        }

        if (previousMerkleRoot != 0) {
            if (previousMerkleRoot != merkleRoot) {
                delete _blockNumbers[blockNumber];
                delete _blockRootTimestamps[blockNumber];

                if (!_emergencyStop) {
                    _emergencyStop = true;

                    emit EmergencyStopEnabled();

                    if (isChallengingRoot()) {
                        _challengingRootTimestamp = 0;

                        emit ChallengeRootDisabled();
                    }
                }
            } else {
                revert MerkleRootAlreadySet(blockNumber);
            }
        } else {
            _blockNumbers[blockNumber] = merkleRoot;
            _blockRootTimestamps[blockNumber] = block.timestamp;
            if (_genesisBlock == 0) _genesisBlock = blockNumber;

            if (blockNumber > _maxBlockNumber) {
                _maxBlockNumber = blockNumber;
            }

            emit MerkleRoot(blockNumber, merkleRoot);
            _recordPublication(blockNumber, merkleRoot, proof);
        }
    }

    /**
     * @dev Returns merkle root for specified block number.
     *      Returns `bytes32(0)` if merkle root was not provided for specified block number.
     * @param blockNumber Target block number.
     * @return merkleRoot Merkle root for specified block number.
     */
    function getMerkleRoot(uint256 blockNumber) external view returns (bytes32) {
        return _blockNumbers[blockNumber];
    }

    /// @dev Retained legacy root-keyed history. New registrations never write this mapping.
    function getMerkleRootTimestamp(bytes32 merkleRoot) external view returns (uint256) {
        return _merkleRootTimestamps[merkleRoot];
    }

    /// @dev Effective maturity for a registered block; absent roots return zero.
    function getMerkleRootTimestampForBlock(uint256 blockNumber) external view returns (uint256) {
        return _effectiveRootTimestamp(blockNumber);
    }

    function _effectiveRootTimestamp(uint256 blockNumber) private view returns (uint256 timestamp) {
        bytes32 root = _blockNumbers[blockNumber];
        if (root == bytes32(0)) {
            return 0;
        }
        timestamp = _blockRootTimestamps[blockNumber];
        if (timestamp == 0 && (beefyRootMinimum == 0 || blockNumber < beefyRootMinimum)) {
            timestamp = _merkleRootTimestamps[root];
        }
        if (timestamp == 0) {
            revert MerkleRootTimestampNotFound(blockNumber);
        }
    }

    function _checkSubmissionBlock(uint256 sourceBlock, bool emptyProgress) private view {
        if (sourceBlock > type(uint32).max) revert BlockNumberOverflow(sourceBlock);
        if (beefyRootMinimum == 0) {
            // Recovery preserves the retained verifier's original publication lane.
            if (emptyProgress) revert InvalidBeefyVerifier();
            if (sourceBlock == 0) revert InvalidSourceBlock();
            if (_genesisBlock != 0 && sourceBlock > _maxBlockNumber + MAX_BLOCK_DISTANCE) {
                revert BlockNumberTooFar(sourceBlock, _maxBlockNumber + MAX_BLOCK_DISTANCE);
            }
            return;
        }
        if (sourceBlock > type(uint32).max) {
            revert BlockNumberOverflow(sourceBlock);
        }
        if (sourceBlock == 0) {
            revert InvalidSourceBlock();
        }
        if (sourceBlock < beefyRootMinimum) {
            revert BlockNumberBelowMinimum(sourceBlock, beefyRootMinimum);
        }
        if (_genesisBlock == 0) {
            if (emptyProgress) {
                revert EmptyQueueNotInitialized();
            }
            return;
        }
        if (emptyProgress && sourceBlock <= _maxBlockNumber) {
            revert EmptyQueueProgressNotForward();
        }
        uint256 maxAllowedBlockNumber = _maxBlockNumber + MAX_BLOCK_DISTANCE;
        if (sourceBlock > maxAllowedBlockNumber) {
            revert BlockNumberTooFar(sourceBlock, maxAllowedBlockNumber);
        }
    }

    /// @dev Move the authenticated empty queue height without creating a root or changing any maturity timestamp.
    function submitEmptyQueueProgress(uint256 sourceBlock, bytes calldata authenticatedSnapshotProof) external {
        if (legacyPublicationFrozen) revert LegacyPublicationFrozen();
        bool isFromEmergencyStopAdmin = msg.sender == _emergencyStopAdmin;

        if (isChallengingRoot() && !isFromEmergencyStopAdmin) {
            revert ChallengeRoot();
        }
        if (_emergencyStop && !isFromEmergencyStopAdmin) {
            revert EmergencyStop();
        }
        _checkSubmissionBlock(sourceBlock, true);

        bool valid;
        try IQueueProgressVerifier(address(_verifier))
            .safeVerifyEmptyQueueProgress(sourceBlock, authenticatedSnapshotProof) returns (
            bool verified
        ) {
            valid = verified;
        } catch {}
        if (!valid) {
            revert InvalidEmptyQueueProgressProof();
        }

        _maxBlockNumber = sourceBlock;
        _recordPublication(sourceBlock, bytes32(0), authenticatedSnapshotProof);
        emit EmptyQueueProgress(sourceBlock);
    }

    /**
     * @dev Verifies and processes message originated from Vara Network.
     *
     *      In this process, MessageQueue smart contract will calculate Merkle root
     *      for message and validate that it corresponds to Merkle root which is already stored
     *      in MessageQueue smart contract for same block number. If proof is correct, nonce of received
     *      message will be stored in smart contract and message will be forwarded to adequate message
     *      processor, either ERC20Manager or Governance smart contract.
     *
     *      Upon successful processing of the message MessageProcessed event is emited.
     *
     *      It is important to note that anyone can submit a message because all messages
     *      will be validated against previously stored Merkle roots in the MessageQueue smart contract.
     *
     * @param blockNumber Block number of block containing target merkle tree.
     * @param totalLeaves Number of leaves in target merkle tree.
     * @param leafIndex Index of leaf containing target message.
     * @param message Target message.
     * @param proof Merkle proof of inclusion of leaf #`leafIndex` into target merkle tree that
     *              was included into `blockNumber`.
     *
     * @dev Reverts if:
     *      - MessageQueue is in challenging root status with `ChallengeRoot` error.
     *      - MessageQueue is paused and message source is not any governance address.
     *      - MessageQueue emergency stop status is set.
     *      - Message nonce is already processed.
     *      - Merkle root is not set for the block number in MessageQueue smart contract.
     *      - Merkle proof is invalid.
     *      - Message processing fails.
     */
    function processMessage(
        uint256 blockNumber,
        uint256 totalLeaves,
        uint256 leafIndex,
        VaraMessage calldata message,
        bytes32[] calldata proof
    ) external {
        if (isChallengingRoot()) {
            revert ChallengeRoot();
        }

        bytes32 governanceAdminAddress = _governanceAdmin.governance();
        bytes32 governancePauserAddress = _governancePauser.governance();

        bool isAdmin = message.source == governanceAdminAddress && message.destination == address(_governanceAdmin);
        bool isPauser = message.source == governancePauserAddress && message.destination == address(_governancePauser);
        bool canBypassPause = isAdmin || isPauser;
        if (paused() && !canBypassPause) {
            revert EnforcedPause();
        }

        bool isFromEmergencyStopAdmin = msg.sender == _emergencyStopAdmin;
        bool canBypassEmergencyStop = isFromEmergencyStopAdmin || _allowMessageProcessing;
        if (_emergencyStop && !canBypassEmergencyStop) {
            revert EmergencyStop();
        }

        if (_processedMessages[message.nonce]) {
            revert MessageAlreadyProcessed(message.nonce);
        }

        bytes32 merkleRoot = _blockNumbers[blockNumber];
        if (merkleRoot == bytes32(0)) {
            revert MerkleRootNotFound(blockNumber);
        }

        uint256 messageDelay;
        if (isAdmin) {
            messageDelay = PROCESS_ADMIN_MESSAGE_DELAY;
        } else if (isPauser) {
            messageDelay = PROCESS_PAUSER_MESSAGE_DELAY;
        } else {
            messageDelay = PROCESS_USER_MESSAGE_DELAY;
        }

        uint256 timestamp = _effectiveRootTimestamp(blockNumber);
        // forge-lint: disable-next-item(block-timestamp)
        if (block.timestamp < timestamp || block.timestamp - timestamp < messageDelay) {
            revert MerkleRootDelayNotPassed();
        }

        bytes32 messageHash = message.hashCalldata();
        if (!SubstrateMerkleProof.verify(merkleRoot, messageHash, leafIndex, totalLeaves, proof)) {
            revert InvalidMerkleProof();
        }

        _processedMessages[message.nonce] = true;

        if (message.destination == address(_governanceAdmin) || message.destination == address(_governancePauser)) {
            if (!isAdmin && !isPauser) revert InvalidGovernanceControl();
            _dispatchControl(message, messageHash, isAdmin);
        } else {
            IMessageHandler(message.destination).handleMessage(message.source, message.payload);
        }

        // forge-lint: disable-next-item(reentrancy-events)
        emit MessageProcessed(blockNumber, messageHash, message.nonce, message.destination);
    }

    function _verifyPublication(uint256 sourceBlock, bytes32 root, bytes calldata proof) private view returns (bool) {
        if (beefyRootMinimum != 0) return _verifier.safeVerifyProof(sourceBlock, root, proof);
        // Exact retained ZK verifier public-input layout, used only before BEEFY migration.
        uint256[] memory inputs = new uint256[](2);
        inputs[0] = uint256(root) >> 64;
        inputs[1] = (uint256(uint64(uint256(root))) << 128) | (sourceBlock << 96);
        (bool ok, bytes memory result) =
            address(_verifier).staticcall(abi.encodeWithSignature("safeVerifyProof(bytes,uint256[])", proof, inputs));
        return ok && result.length == 32 && abi.decode(result, (bool));
    }

    function stageLegacyMigration(
        IVerifier expectedLegacyVerifier,
        uint256 expectedLegacyMaxBlock,
        LegacyControl calldata migration,
        LegacyControl calldata recovery
    ) external onlyRole(DEFAULT_ADMIN_ROLE) reinitializer(7) {
        _requirePaused();
        if (
            beefyRootMinimum != 0 || legacyPublicationFrozen || _genesisBlock == 0
                || address(_verifier) != address(expectedLegacyVerifier) || _maxBlockNumber != expectedLegacyMaxBlock
        ) revert InvalidLegacyStage();
        bytes32 migrationHash = _validateLegacyControl(migration, this.migrateToBeefy.selector);
        bytes32 recoveryHash = _validateLegacyControl(recovery, this.recoverLegacyPublication.selector);
        if (migrationHash == recoveryHash || migration.message.nonce == recovery.message.nonce) {
            revert InvalidLegacyStage();
        }
        // Bind the actual prepublished calldata, not the installation message's own hash.
        bytes calldata migrationData = migration.message.payload[45:];
        (IVerifier oldVerifier, IVerifier nextVerifier, uint256 cut, uint256 floor, address timelock) =
            abi.decode(migrationData, (IVerifier, IVerifier, uint256, uint256, address));
        (IVerifier recoveryVerifier, uint256 recoveryCut) =
            abi.decode(recovery.message.payload[45:], (IVerifier, uint256));
        if (
            oldVerifier != expectedLegacyVerifier || recoveryVerifier != expectedLegacyVerifier
                || cut != expectedLegacyMaxBlock || recoveryCut != cut || migration.message.payload.length != 205
                || recovery.message.payload.length != 109
        ) revert InvalidLegacyStage();
        (BeefyClient client, uint64 mmrStartBlock) = _validateBeefyVerifier(nextVerifier);
        if (
            floor != cut + 1 || floor > type(uint32).max || floor < mmrStartBlock || !client.isLive()
                || client.latestMMRRoot() == bytes32(0) || client.latestBeefyBlock() <= floor
                || timelock.code.length == 0 || TimelockController(payable(timelock)).getMinDelay() == 0
        ) revert InvalidLegacyStage();
        legacyPublicationFrozen = true;
        frozenLegacyMaxBlock = cut;
        stagedMigrationMessage = migrationHash;
        stagedRecoveryMessage = recoveryHash;
        _legacyControlExpiry[migrationHash] = migration.validUntil;
        _legacyControlExpiry[recoveryHash] = recovery.validUntil;
        emit LegacyMigrationStaged(cut, migrationHash, recoveryHash);
    }

    function _validateLegacyControl(LegacyControl calldata control, bytes4 selector)
        private
        view
        returns (bytes32 hash)
    {
        VaraMessage calldata message = control.message;
        if (
            message.source != _governanceAdmin.governance() || message.destination != address(_governanceAdmin)
                || message.payload.length < 45 || uint8(message.payload[0]) != 3
                || address(bytes20(message.payload[1:21])) != address(this)
                || address(bytes20(message.payload[21:41])).code.length == 0
                || bytes4(message.payload[41:45]) != selector || control.validUntil == 0
                || block.timestamp > control.validUntil || _processedMessages[message.nonce]
                || control.sourceBlock > _maxBlockNumber
        ) revert InvalidLegacyStage();
        (bool uups, bytes memory slot) =
            address(bytes20(message.payload[21:41])).staticcall(abi.encodeWithSignature("proxiableUUID()"));
        if (!uups || slot.length != 32 || abi.decode(slot, (bytes32)) != ERC1967Utils.IMPLEMENTATION_SLOT) {
            revert InvalidLegacyStage();
        }
        uint256 timestamp = _effectiveRootTimestamp(control.sourceBlock);
        hash = message.hashCalldata();
        if (
            timestamp == 0 || block.timestamp < timestamp + PROCESS_ADMIN_MESSAGE_DELAY
                || !SubstrateMerkleProof.verify(
                    _blockNumbers[control.sourceBlock], hash, control.leafIndex, control.totalLeaves, control.proof
                )
        ) revert InvalidLegacyStage();
    }

    function recoverLegacyPublication(IVerifier expectedLegacyVerifier, uint256 expectedLegacyMaxBlock)
        external
        onlyRole(DEFAULT_ADMIN_ROLE)
    {
        _requirePaused();
        if (
            !legacyPublicationFrozen || beefyRootMinimum != 0 || _executingAdminMessage == bytes32(0)
                || _executingAdminMessage != stagedRecoveryMessage || _verifier != expectedLegacyVerifier
                || frozenLegacyMaxBlock != expectedLegacyMaxBlock || _maxBlockNumber != expectedLegacyMaxBlock
        ) revert InvalidLegacyStage();
        _clearLegacyStage();
        ++incidentEpoch[address(this)];
        emit LegacyPublicationRecovered(expectedLegacyMaxBlock);
    }

    function _clearLegacyStage() private {
        delete _legacyControlExpiry[stagedMigrationMessage];
        delete _legacyControlExpiry[stagedRecoveryMessage];
        delete stagedMigrationMessage;
        delete stagedRecoveryMessage;
        legacyPublicationFrozen = false;
    }

    function _recordPublication(uint256 sourceBlock, bytes32 root, bytes calldata proof) private {
        if (beefyRootMinimum == 0) return;
        // These exact fields have already been authenticated by the pinned v2 verifier.
        (,,,, uint64 sourceTimestampMs, uint64 queueId) =
            abi.decode(proof, (uint8, uint8, bool, bytes32, uint64, uint64));
        publications[sourceBlock] = Publication(queueId, sourceTimestampMs, block.number, root);
    }

    /// @dev Capture canonical identity while blockhash is available; this is NOT a finality attestation.
    function captureMigrationPublicationHash(uint256 sourceBlock) external returns (bytes32 publicationHash) {
        Publication memory publication = publications[sourceBlock];
        if (
            migrationAdminAction == bytes32(0) || migrationBarrier != bytes32(0)
                || publication.destinationBlock <= migrationDestinationBlock || sourceBlock < beefyRootMinimum
                || publication.sourceTimestampMs <= migrationTimestamp * 1000
                || _blockNumbers[sourceBlock] != publication.root
        ) revert InvalidMigrationBarrier();
        bytes32 identity = _migrationPublicationIdentity(sourceBlock, publication);
        publicationHash = _capturedMigrationPublicationHashes[identity];
        if (publicationHash != bytes32(0)) return publicationHash;
        publicationHash = blockhash(publication.destinationBlock);
        if (publicationHash == bytes32(0)) revert InvalidMigrationBarrier();
        _capturedMigrationPublicationHashes[identity] = publicationHash;
        emit MigrationPublicationHashCaptured(sourceBlock, identity, publicationHash);
    }

    function migrationPublicationBlockHash(uint256 sourceBlock) public view returns (bytes32) {
        Publication memory publication = publications[sourceBlock];
        if (_blockNumbers[sourceBlock] != publication.root) return bytes32(0);
        return _capturedMigrationPublicationHashes[_migrationPublicationIdentity(sourceBlock, publication)];
    }

    function _migrationPublicationIdentity(uint256 sourceBlock, Publication memory publication)
        private
        view
        returns (bytes32)
    {
        return keccak256(abi.encode(block.chainid, address(this), migrationAdminAction, sourceBlock, publication));
    }

    /// @dev An approved admin attests finality after governance maturity, using the earlier canonical capture.
    function recordMigrationBarrier(uint256 sourceBlock, bytes32 publicationBlockHash)
        external
        onlyRole(DEFAULT_ADMIN_ROLE)
        returns (bytes32 barrier)
    {
        Publication memory publication = publications[sourceBlock];
        if (
            migrationAdminAction == bytes32(0) || migrationBarrier != bytes32(0)
                || publication.destinationBlock <= migrationDestinationBlock || sourceBlock < beefyRootMinimum
                || publication.sourceTimestampMs <= migrationTimestamp * 1000 || publicationBlockHash == bytes32(0)
                || migrationPublicationBlockHash(sourceBlock) != publicationBlockHash
                || _blockNumbers[sourceBlock] != publication.root
        ) revert InvalidMigrationBarrier();
        barrier = keccak256(
            abi.encode(
                block.chainid,
                address(this),
                migrationAdminAction,
                sourceBlock,
                publication.queueId,
                publication.root,
                publication.destinationBlock,
                publicationBlockHash
            )
        );
        migrationBarrier = barrier;
        migrationBarrierSourceBlock = sourceBlock;
        emit MigrationBarrierRecorded(barrier, sourceBlock, publicationBlockHash);
    }

    function _dispatchControl(VaraMessage calldata message, bytes32 messageHash, bool isAdmin) private {
        bytes32 stream = keccak256(abi.encode(message.source, message.destination));
        if (retiredGovernanceStream[stream]) revert GovernanceControlConsumed();
        if (_legacyControlExpiry[messageHash] != 0) {
            if (!legacyPublicationFrozen || !isAdmin || block.timestamp > _legacyControlExpiry[messageHash]) {
                revert InvalidLegacyStage();
            }
            _executingAdminMessage = messageHash;
            IMessageHandler(message.destination).handleMessage(message.source, message.payload);
            delete _executingAdminMessage;
            completedAdminActions[messageHash] = AdminCompletion(address(this), incidentEpoch[address(this)]);
            return;
        }
        if (legacyPublicationFrozen) revert LegacyPublicationFrozen();
        ControlEnvelope memory control = _decodeControl(message.payload);
        if (control.validUntil == 0 || block.timestamp > control.validUntil) revert GovernanceControlExpired();
        if (control.sequence <= executedControlSequence[stream] || control.sequence <= canceledControlSequence[stream]) revert GovernanceControlConsumed();
        executedControlSequence[stream] = control.sequence;
        if (control.action == 4) {
            if (!isAdmin || control.target != address(this)) revert InvalidGovernanceControl();
            (bytes32 source, address destination, uint64 through) =
                abi.decode(control.innerPayload, (bytes32, address, uint64));
            if (
                keccak256(control.innerPayload) != keccak256(abi.encode(source, destination, through))
                    || !((destination == address(_governanceAdmin) && source == _governanceAdmin.governance())
                        || (destination == address(_governancePauser) && source == _governancePauser.governance()))
            ) revert InvalidGovernanceControl();
            bytes32 canceled = keccak256(abi.encode(source, destination));
            if (through <= canceledControlSequence[canceled]) revert GovernanceControlConsumed();
            canceledControlSequence[canceled] = through;
        } else {
            _validateControlTarget(control, message.destination, isAdmin);
            bool dispatch = true;
            if (control.action == 0) {
                bytes32 next;
                bytes memory inner = control.innerPayload;
                assembly ("memory-safe") { next := mload(add(inner, 33)) }
                if (
                    next == bytes32(0) || next == message.source
                        || retiredGovernanceStream[keccak256(abi.encode(next, message.destination))]
                ) {
                    revert InvalidGovernanceControl();
                }
                retiredGovernanceStream[stream] = true;
            } else if (control.action == 1) {
                ++incidentEpoch[control.target];
                dispatch = !PausableUpgradeable(control.target).paused();
            } else if (control.action == 2) {
                AdminCompletion memory dependency = completedAdminActions[control.requiredAdminAction];
                if (
                    control.incidentEpoch != incidentEpoch[control.target] || dependency.target != control.target
                        || dependency.incidentEpoch != control.incidentEpoch
                        || control.requiredAdminAction == bytes32(0)
                ) revert GovernanceDependencyMissing();
                if (control.target == address(this) && migrationResumePending) {
                    if (
                        control.requiredAdminAction != migrationAdminAction || migrationBarrier == bytes32(0)
                            || control.requiredBarrier != migrationBarrier
                    ) revert GovernanceDependencyMissing();
                    migrationResumePending = false;
                } else if (control.requiredBarrier != bytes32(0) && control.requiredBarrier != migrationBarrier) {
                    revert InvalidGovernanceControl();
                }
                dispatch = PausableUpgradeable(control.target).paused();
            }
            if (dispatch) {
                if (isAdmin) _executingAdminMessage = messageHash;
                IMessageHandler(message.destination).handleMessage(message.source, control.innerPayload);
                if (isAdmin) delete _executingAdminMessage;
            }
        }
        if (isAdmin) {
            completedAdminActions[messageHash] = AdminCompletion(control.target, incidentEpoch[control.target]);
        }
        emit GovernanceControlCompleted(messageHash, stream, control.sequence);
    }

    function _decodeControl(bytes calldata payload) private pure returns (ControlEnvelope memory control) {
        if (payload.length < 1 || payload[0] != 0xff) revert InvalidGovernanceControl();
        (
            control.version,
            control.sequence,
            control.validUntil,
            control.action,
            control.target,
            control.incidentEpoch,
            control.requiredAdminAction,
            control.requiredBarrier,
            control.innerPayload
        ) = abi.decode(payload[1:], (uint8, uint64, uint64, uint8, address, uint64, bytes32, bytes32, bytes));
        if (
            control.version != 1 || control.action > 4 || control.target == address(0)
                || keccak256(payload[1:])
                    != keccak256(
                        abi.encode(
                            control.version,
                            control.sequence,
                            control.validUntil,
                            control.action,
                            control.target,
                            control.incidentEpoch,
                            control.requiredAdminAction,
                            control.requiredBarrier,
                            control.innerPayload
                        )
                    )
                || (control.action != 2
                    && (control.incidentEpoch != 0
                        || control.requiredAdminAction != bytes32(0)
                        || control.requiredBarrier != bytes32(0)))
        ) revert InvalidGovernanceControl();
    }

    function _validateControlTarget(ControlEnvelope memory control, address destination, bool isAdmin) private view {
        bytes memory inner = control.innerPayload;
        if (inner.length == 0 || uint8(inner[0]) != control.action) revert InvalidGovernanceControl();
        if (control.action == 0) {
            if (control.target != destination || inner.length != 33) revert InvalidGovernanceControl();
            return;
        }
        if (inner.length < 21) revert InvalidGovernanceControl();
        address target;
        assembly ("memory-safe") { target := shr(96, mload(add(inner, 33))) }
        if (
            target != control.target || (control.action == 3 && (!isAdmin || inner.length < 41))
                || (control.action != 3 && inner.length != 21)
        ) revert InvalidGovernanceControl();
        // Validate even redundant controls: skipping a call must not bypass the retained handler's target policy.
        (bool wrappedOk, bytes memory wrapped) = destination.staticcall(abi.encodeWithSignature("wrappedVara()"));
        (bool managerOk, bytes memory manager) = destination.staticcall(abi.encodeWithSignature("erc20Manager()"));
        if (
            !wrappedOk || !managerOk
                || (target != address(this)
                    && target != abi.decode(wrapped, (address))
                    && target != abi.decode(manager, (address)))
        ) {
            revert InvalidGovernanceControl();
        }
    }

    /**
     * @dev Checks if message was already processed.
     * @param messageNonce Message nonce to check.
     * @return isProcessed `true` if message was already processed, `false` otherwise.
     */
    function isProcessed(uint256 messageNonce) external view returns (bool) {
        return _processedMessages[messageNonce];
    }

    function _initialize(
        IGovernance governanceAdmin_,
        IGovernance governancePauser_,
        address emergencyStopAdmin_,
        address[] memory emergencyStopObservers_,
        IVerifier verifier_
    ) private {
        __AccessControl_init();
        __Pausable_init();

        _grantRole(DEFAULT_ADMIN_ROLE, address(governanceAdmin_));

        _grantRole(PAUSER_ROLE, address(governanceAdmin_));
        _grantRole(PAUSER_ROLE, address(governancePauser_));

        if (emergencyStopAdmin_ == address(0)) {
            revert InvalidEmergencyStopAdmin();
        }

        _governanceAdmin = governanceAdmin_;
        _governancePauser = governancePauser_;
        _emergencyStopAdmin = emergencyStopAdmin_;

        for (uint256 i = 0; i < emergencyStopObservers_.length; i++) {
            _emergencyStopObservers.add(emergencyStopObservers_[i]);
        }

        _verifier = verifier_;
    }

    function _validateBeefyVerifier(IVerifier verifier_) private view returns (BeefyClient, uint64) {
        if (address(verifier_).code.length == 0) {
            revert InvalidBeefyVerifier();
        }
        VaraQueueRootVerifier rootVerifier = VaraQueueRootVerifier(address(verifier_));
        if (rootVerifier.messageQueue() != address(this) || rootVerifier.destinationChainId() != block.chainid) {
            revert InvalidBeefyVerifier();
        }
        address clientAddress = address(rootVerifier.beefyClient());
        if (clientAddress.code.length == 0) {
            revert InvalidBeefyVerifier();
        }
        BeefyClient client = BeefyClient(clientAddress);
        return (client, client.mmrStartBlock());
    }

    function _setBeefyRecoveryTimelock(address recoveryTimelock) private {
        if (recoveryTimelock.code.length == 0 || TimelockController(payable(recoveryTimelock)).getMinDelay() == 0) {
            revert InvalidBeefyRecoveryTimelock();
        }
        beefyRecoveryTimelock = recoveryTimelock;
    }
}
