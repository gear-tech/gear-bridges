// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import {AccessControlUpgradeable} from "@openzeppelin/contracts-upgradeable/access/AccessControlUpgradeable.sol";
import {PausableUpgradeable} from "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {EnumerableSet} from "@openzeppelin/contracts/utils/structs/EnumerableSet.sol";
import {IGovernance} from "./interfaces/IGovernance.sol";
import {IMessageHandler} from "./interfaces/IMessageHandler.sol";
import {VaraMessage, IMessageQueue, Hasher} from "./interfaces/IMessageQueue.sol";
import {IPausable} from "./interfaces/IPausable.sol";
import {IVerifier} from "./interfaces/IVerifier.sol";
import {BinaryMerkleTree} from "./libraries/BinaryMerkleTree.sol";

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

    uint256 public constant CHALLENGE_ROOT_DELAY = 1 days;

    uint256 public constant PROCESS_ADMIN_MESSAGE_DELAY = 1 hours;
    uint256 public constant PROCESS_PAUSER_MESSAGE_DELAY = 3 minutes;
    uint256 public constant PROCESS_USER_MESSAGE_DELAY = 5 minutes;

    uint256 public constant MAX_BLOCK_DISTANCE = 5;

    IGovernance private _governanceAdmin;
    IGovernance private _governancePauser;
    address private _emergencyStopAdmin;
    EnumerableSet.AddressSet private _emergencyStopObservers;
    IVerifier private _verifier;
    uint256 private _challengingRootTimestamp;
    bool private _emergencyStop;
    uint256 private _genesisBlock;
    uint256 private _maxBlockNumber;
    mapping(uint256 blockNumber => bytes32 merkleRoot) private _blockNumbers;
    mapping(bytes32 merkleRoot => uint256 timestamp) private _merkleRootTimestamps;
    mapping(uint256 messageNonce => bool isProcessed) private _processedMessages;

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
     * @param governanceAdmin_ The address of the GovernanceAdmin contract that will process messages.
     * @param governancePauser_ The address of the GovernanceAdmin contract that will process pauser messages.
     * @param emergencyStopAdmin_ The address of EOA that will control `submitMerkleRoot` and `processMessage`
     *                            in case of an emergency stop.
     * @param verifier_ The address of the Verifier contract that will verify merkle roots.
     */
    function initialize(
        IGovernance governanceAdmin_,
        IGovernance governancePauser_,
        address emergencyStopAdmin_,
        address[] memory emergencyStopObservers_,
        IVerifier verifier_
    ) public initializer {
        __AccessControl_init();
        __Pausable_init();
        __UUPSUpgradeable_init();

        _grantRole(DEFAULT_ADMIN_ROLE, address(governanceAdmin_));

        _grantRole(PAUSER_ROLE, address(governanceAdmin_));
        _grantRole(PAUSER_ROLE, address(governancePauser_));

        _governanceAdmin = governanceAdmin_;
        _governancePauser = governancePauser_;
        _emergencyStopAdmin = emergencyStopAdmin_;

        for (uint256 i = 0; i < emergencyStopObservers_.length; i++) {
            _emergencyStopObservers.add(emergencyStopObservers_[i]);
        }

        _verifier = verifier_;
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
        return block.timestamp < _challengingRootTimestamp + CHALLENGE_ROOT_DELAY;
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
     *      - challenging root status is already enabled with `ChallengeRoot` error.
     *
     * @dev Emits `ChallengeRootEnabled(block.timestamp + CHALLENGE_ROOT_DELAY)` event.
     */
    function challengeRoot() external {
        if (!_emergencyStopObservers.contains(msg.sender)) {
            revert NotEmergencyStopObserver();
        }

        if (isChallengingRoot()) {
            revert ChallengeRoot();
        }

        _challengingRootTimestamp = block.timestamp;

        emit ChallengeRootEnabled(block.timestamp + CHALLENGE_ROOT_DELAY);
    }

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
     * @param proof Serialised Plonk proof (using gnark's `MarshalSolidity`).
     * @dev Reverts if challenging root status is enabled and caller is not emergency stop admin with `ChallengeRoot` error.
     * @dev Reverts if emergency stop status is set and caller is not emergency stop admin with `EmergencyStop` error.
     * @dev Reverts if `proof` or `publicInputs` are malformed with `InvalidPlonkProof` error.
     * @dev Reverts if block number is before genesis block with `BlockNumberBeforeGenesis` error.
     * @dev Reverts if block number is too far from max block number with `BlockNumberTooFar` error.
     */
    function submitMerkleRoot(uint256 blockNumber, bytes32 merkleRoot, bytes calldata proof) external {
        bool isFromEmergencyStopAdmin = msg.sender == _emergencyStopAdmin;

        if (isChallengingRoot() && !isFromEmergencyStopAdmin) {
            revert ChallengeRoot();
        }

        if (_emergencyStop && !isFromEmergencyStopAdmin) {
            revert EmergencyStop();
        }

        if (_genesisBlock == 0) {
            _genesisBlock = blockNumber;
            _maxBlockNumber = blockNumber;
        } else {
            if (blockNumber < _genesisBlock) {
                revert BlockNumberBeforeGenesis(blockNumber, _genesisBlock);
            }

            if (blockNumber > _maxBlockNumber + MAX_BLOCK_DISTANCE) {
                revert BlockNumberTooFar(blockNumber, _maxBlockNumber + MAX_BLOCK_DISTANCE);
            }
        }

        uint256[] memory publicInputs = new uint256[](2);
        publicInputs[0] = uint256(merkleRoot) >> 64;
        publicInputs[1] = ((uint256(merkleRoot) & uint256(type(uint64).max)) << 128)
            | ((blockNumber & uint256(type(uint32).max)) << 96);

        if (!_verifier.safeVerifyProof(proof, publicInputs)) {
            revert InvalidPlonkProof();
        }

        bytes32 previousMerkleRoot = _blockNumbers[blockNumber];
        if (previousMerkleRoot != 0) {
            if (previousMerkleRoot != merkleRoot) {
                delete _blockNumbers[blockNumber];
                delete _merkleRootTimestamps[previousMerkleRoot];

                if (!_emergencyStop) {
                    _emergencyStop = true;

                    emit EmergencyStopEnabled();
                }
            } else {
                revert MerkleRootAlreadySet(blockNumber);
            }
        } else {
            _blockNumbers[blockNumber] = merkleRoot;
            _merkleRootTimestamps[merkleRoot] = block.timestamp;

            if (blockNumber > _maxBlockNumber) {
                _maxBlockNumber = blockNumber;
            }

            emit MerkleRoot(blockNumber, merkleRoot);
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

    /**
     * @dev Returns timestamp when merkle root was set.
     *      Returns `0` if merkle root was not provided for specified block number.
     * @param merkleRoot Target merkle root.
     * @return timestamp Timestamp when merkle root was set.
     */
    function getMerkleRootTimestamp(bytes32 merkleRoot) external view returns (uint256) {
        return _merkleRootTimestamps[merkleRoot];
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

        bool isFromAdminOrPauser = message.source == governanceAdminAddress || message.source == governancePauserAddress;
        if (paused() && !isFromAdminOrPauser) {
            revert EnforcedPause();
        }

        bool isFromEmergencyStopAdmin = msg.sender == _emergencyStopAdmin;
        if (_emergencyStop && !isFromEmergencyStopAdmin) {
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
        if (message.source == governanceAdminAddress) {
            messageDelay = PROCESS_ADMIN_MESSAGE_DELAY;
        } else if (message.source == governancePauserAddress) {
            messageDelay = PROCESS_PAUSER_MESSAGE_DELAY;
        } else {
            messageDelay = PROCESS_USER_MESSAGE_DELAY;
        }

        uint256 timestamp = _merkleRootTimestamps[merkleRoot];
        if (block.timestamp < timestamp + messageDelay) {
            revert MerkleRootDelayNotPassed();
        }

        bytes32 messageHash = message.hashCalldata();
        if (!BinaryMerkleTree.verifyProofCalldata(merkleRoot, proof, totalLeaves, leafIndex, messageHash)) {
            revert InvalidMerkleProof();
        }

        _processedMessages[message.nonce] = true;

        IMessageHandler(message.destination).handleMessage(message.source, message.payload);

        emit MessageProcessed(blockNumber, messageHash, message.nonce, message.destination);
    }

    /**
     * @dev Checks if message was already processed.
     * @param messageNonce Message nonce to check.
     * @return isProcessed `true` if message was already processed, `false` otherwise.
     */
    function isProcessed(uint256 messageNonce) external view returns (bool) {
        return _processedMessages[messageNonce];
    }
}
