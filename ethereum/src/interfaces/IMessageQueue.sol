// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IPausable} from "./IPausable.sol";

/**
 * @dev Type representing message being bridged from Gear-based chain (Vara Network) to Ethereum.
 *      - https://github.com/gear-tech/gear/blob/v1.8.1/pallets/gear-eth-bridge/src/internal.rs#L58
 */
struct VaraMessage {
    uint256 nonce;
    bytes32 source;
    address destination;
    bytes payload;
}

/**
 * @dev Interface for the MessageQueue contract.
 */
interface IMessageQueue is IPausable {
    /**
     * @dev Challenge root status is enabled.
     */
    error ChallengeRoot();

    /**
     * @dev Emergency stop status is enabled.
     */
    error EmergencyStop();

    /**
     * @dev The plonk proof is invalid.
     */
    error InvalidPlonkProof();

    /**
     * @dev Message nonce is already processed.
     */
    error MessageAlreadyProcessed(uint256 messageNonce);

    /**
     * @dev Merkle root is not found for the block number in MessageQueue smart contract.
     */
    error MerkleRootNotFound(uint256 blockNumber);

    /**
     * @dev Merkle root delay is not passed.
     */
    error MerkleRootDelayNotPassed();

    /**
     * @dev Merkle proof is invalid.
     */
    error InvalidMerkleProof();

    /**
     * @dev Merkle root is already set.
     */
    error MerkleRootAlreadySet(uint256 blockNumber);

    /**
     * @dev Caller is not emergency stop observer.
     */
    error NotEmergencyStopObserver();

    /**
     * @dev Block number is before genesis block.
     */
    error BlockNumberBeforeGenesis(uint256 blockNumber, uint256 genesisBlock);

    /**
     * @dev Block number is too far from max block number.
     */
    error BlockNumberTooFar(uint256 blockNumber, uint256 maxBlockNumber);

    /**
     * @dev Emitted when challenging root status is enabled.
     */
    event ChallengeRootEnabled(uint256 untilTimestamp);

    /**
     * @dev Emitted when emergency stop status is enabled.
     */
    event EmergencyStopEnabled();

    /**
     * @dev Emitted when emergency stop status is disabled.
     *      Should be emitted on upgradeV2 function of the smart contract.
     */
    event EmergencyStopDisabled();

    /**
     * @dev Emitted when block number and merkle root are stored.
     */
    event MerkleRoot(uint256 blockNumber, bytes32 merkleRoot);

    /**
     * @dev Emitted when message is processed.
     */
    event MessageProcessed(uint256 blockNumber, bytes32 messageHash, uint256 messageNonce, address messageDestination);

    /**
     * @dev Returns governance admin address.
     * @return governanceAdmin Governance admin address.
     */
    function governanceAdmin() external view returns (address);

    /**
     * @dev Returns governance pauser address.
     * @return governancePauser Governance pauser address.
     */
    function governancePauser() external view returns (address);

    /**
     * @dev Returns emergency stop admin address.
     * @return emergencyStopAdmin Emergency stop admin address.
     */
    function emergencyStopAdmin() external view returns (address);

    /**
     * @dev Returns list of emergency stop observers.
     * @return emergencyStopObservers List of emergency stop observers.
     */
    function emergencyStopObservers() external view returns (address[] memory);

    /**
     * @dev Returns verifier address.
     *      Verifier is smart contract that is responsible for verifying
     *      the validity of the Merkle proof.
     * @return verifier Verifier address.
     */
    function verifier() external view returns (address);

    /**
     * @dev Returns challenging root status.
     * @return isChallengingRoot challenging root status.
     */
    function isChallengingRoot() external view returns (bool);

    /**
     * @dev Returns emergency stop status.
     * @return isEmergencyStopped emergency stop status.
     */
    function isEmergencyStopped() external view returns (bool);

    /**
     * @dev Returns genesis block number.
     * @return genesisBlock Genesis block number.
     */
    function genesisBlock() external view returns (uint256);

    /**
     * @dev Returns maximum block number.
     * @return maxBlockNumber Maximum block number.
     */
    function maxBlockNumber() external view returns (uint256);

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
    function challengeRoot() external;

    /**
     * @dev Receives, verifies and stores Merkle roots from Vara Network.
     *
     *      Upon successfully storing data about block number and corresponding Merkle root,
     *      MessageQueue smart contract will emit a `MerkleRoot` event.
     *
     *      It is important to note that anyone can submit a Merkle root because only
     *      validated Merkle roots will be stored in the MessageQueue smart contract.
     *
     * @param blockNumber Block number on Vara Network
     * @param merkleRoot Merkle root of transactions included in block with corresponding block number
     * @param proof Serialised Plonk proof (using gnark's `MarshalSolidity`).
     * @dev Reverts if emergency stop status is enabled with `EmergencyStop` error.
     * @dev Reverts if `proof` or `publicInputs` are malformed with `InvalidPlonkProof` error.
     */
    function submitMerkleRoot(uint256 blockNumber, bytes32 merkleRoot, bytes calldata proof) external;

    /**
     * @dev Returns merkle root for specified block number.
     *      Returns `bytes32(0)` if merkle root was not provided for specified block number.
     * @param blockNumber Target block number.
     * @return merkleRoot Merkle root for specified block number.
     */
    function getMerkleRoot(uint256 blockNumber) external view returns (bytes32);

    /**
     * @dev Returns timestamp when merkle root was set.
     *      Returns `0` if merkle root was not provided for specified block number.
     * @param merkleRoot Target merkle root.
     * @return timestamp Timestamp when merkle root was set.
     */
    function getMerkleRootTimestamp(bytes32 merkleRoot) external view returns (uint256);

    /**
     * @dev Verifies and processes message originated from Vara Network.
     *
     *      In this process, MessageQueue smart contract will calculate Merkle root
     *      for message and validate that it corresponds to Merkle root which is already stored
     *      in MessageQueue smart contract for same block number. If proof is correct, nonce of received
     *      message will be stored in smart contract and message will be forwarded to adequate message
     *      processor, either ERC20Manager or Governance smart contract.
     *
     *      Upon successful processing of the message `MessageProcessed` event is emitted.
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
     *      - MessageQueue emergency stop status is enabled and caller is not emergency stop admin.
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
    ) external;

    /**
     * @dev Checks if message was already processed.
     * @param messageNonce Message nonce to check.
     * @return isProcessed `true` if message was already processed, `false` otherwise.
     */
    function isProcessed(uint256 messageNonce) external view returns (bool);
}

/**
 * @dev Library for hashing VaraMessage.
 */
library Hasher {
    /**
     * @dev Hashes VaraMessage.
     * @param message Message to hash.
     * @return hash Hash of the message.
     */
    function hashCalldata(VaraMessage calldata message) internal pure returns (bytes32) {
        /// forge-lint: disable-next-line(asm-keccak256)
        return keccak256(abi.encodePacked(message.nonce, message.source, message.destination, message.payload));
    }

    /**
     * @dev Hashes VaraMessage.
     * @param message Message to hash.
     * @return hash Hash of the message.
     */
    function hash(VaraMessage memory message) internal pure returns (bytes32) {
        /// forge-lint: disable-next-line(asm-keccak256)
        return keccak256(abi.encodePacked(message.nonce, message.source, message.destination, message.payload));
    }
}
