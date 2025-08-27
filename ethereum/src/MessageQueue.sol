// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import {AccessControlUpgradeable} from "@openzeppelin/contracts-upgradeable/access/AccessControlUpgradeable.sol";
import {PausableUpgradeable} from "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
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
    using Hasher for VaraMessage;

    bytes32 public constant PAUSER_ROLE = bytes32(uint256(0x01));
    uint256 public constant PROCESS_MESSAGE_DELAY = 5 minutes;

    IGovernance private _governanceAdmin;
    IGovernance private _governancePauser;
    address private _emergencyStopAdmin;
    IVerifier private _verifier;
    bool private _emergencyStop;
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
     * @param emergencyStopAdmin_ The address of EOA (multi-sig) that will control `submitMerkleRoot` and `processMessage`
     *                            in case of an emergency stop. This EOA will be able to disable the emergency stop at
     *                            the same time along with the verifier change.
     * @param verifier_ The address of the Verifier contract that will verify merkle roots.
     */
    function initialize(
        IGovernance governanceAdmin_,
        IGovernance governancePauser_,
        address emergencyStopAdmin_,
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
     * @dev Returns verifier address.
     * @return verifier Verifier address.
     */
    function verifier() external view returns (address) {
        return address(_verifier);
    }

    /**
     * @dev Returns emergency stop status.
     * @return isEmergencyStopped emergency stop status.
     */
    function isEmergencyStopped() external view returns (bool) {
        return _emergencyStop;
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
     * @dev Reverts if emergency stop status is set with `EmergencyStop` error.
     * @dev Reverts if `proof` or `publicInputs` are malformed with `InvalidPlonkProof` error.
     */
    function submitMerkleRoot(uint256 blockNumber, bytes32 merkleRoot, bytes calldata proof) external {
        bool isFromEmergencyStopAdmin = msg.sender == _emergencyStopAdmin;
        if (_emergencyStop && !isFromEmergencyStopAdmin) {
            revert EmergencyStop();
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
                _emergencyStop = true;

                delete _blockNumbers[blockNumber];
                delete _merkleRootTimestamps[previousMerkleRoot];

                emit EmergencyStopEnabled();
            } else {
                revert MerkleRootAlreadySet(blockNumber);
            }
        } else {
            _blockNumbers[blockNumber] = merkleRoot;
            _merkleRootTimestamps[merkleRoot] = block.timestamp;

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
        bool isFromAdminOrPauser =
            message.source == _governanceAdmin.governance() || message.source == _governancePauser.governance();
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

        uint256 timestamp = _merkleRootTimestamps[merkleRoot];
        if (block.timestamp < timestamp + PROCESS_MESSAGE_DELAY) {
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
     * @dev Disables emergency stop status and sets new verifier.
     *      This function can only be called by emergency stop admin.
     * @param newVerifier New verifier address.
     *
     * @dev Reverts if:
     *      - Emergency stop status is not enabled.
     *      - Caller is not emergency stop admin.
     */
    function disableEmergencyStop(address newVerifier) external {
        if (!_emergencyStop) {
            revert EmergencyStopNotEnabled();
        }

        if (msg.sender != _emergencyStopAdmin) {
            revert NotEmergencyStopAdmin();
        }

        _emergencyStop = false;
        _verifier = IVerifier(newVerifier);

        emit EmergencyStopDisabled();
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
