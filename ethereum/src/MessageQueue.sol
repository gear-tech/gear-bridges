// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import {AccessControlUpgradeable} from "@openzeppelin/contracts-upgradeable/access/AccessControlUpgradeable.sol";
import {PausableUpgradeable} from "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {IGovernance} from "./interfaces/IGovernance.sol";
import {IMessageHandler} from "./interfaces/IMessageHandler.sol";
import {VaraMessage, IMessageQueue, Hasher} from "./interfaces/IMessageQueue.sol";
import {IRelayer} from "./interfaces/IRelayer.sol";
import {BinaryMerkleTree} from "./libraries/BinaryMerkleTree.sol";

/**
 * @dev MessageQueue smart contract is responsible for verifying and processing
 *      received messages originated from Vara Network.
 */
contract MessageQueue is
    Initializable,
    AccessControlUpgradeable,
    PausableUpgradeable,
    UUPSUpgradeable,
    IMessageQueue
{
    using Hasher for VaraMessage;

    bytes32 public constant PAUSER_ROLE = bytes32(uint256(0x01));

    IGovernance private _governanceAdmin;
    IGovernance private _governancePauser;
    IRelayer private _relayer;
    mapping(uint256 messageNonce => bool isProcessed) private _processedMessages;

    /**
     * @custom:oz-upgrades-unsafe-allow constructor
     */
    constructor() {
        _disableInitializers();
    }

    /**
     * @dev Initializes the MessageQueue contract with the Relayer address.
     *      GovernanceAdmin contract is used to upgrade, pause/unpause the MessageQueue contract.
     *      GovernancePauser contract is used to pause/unpause the MessageQueue contract.
     * @param governanceAdmin The address of the GovernanceAdmin contract that will process messages.
     * @param governancePauser The address of the GovernanceAdmin contract that will process pauser messages.
     * @param relayer The address of the Relayer contract that will store merkle roots.
     */
    function initialize(IGovernance governanceAdmin, IGovernance governancePauser, IRelayer relayer)
        public
        initializer
    {
        __AccessControl_init();
        __Pausable_init();
        __UUPSUpgradeable_init();

        _grantRole(DEFAULT_ADMIN_ROLE, address(governanceAdmin));

        _grantRole(PAUSER_ROLE, address(governanceAdmin));
        _grantRole(PAUSER_ROLE, address(governancePauser));

        _governanceAdmin = governanceAdmin;
        _governancePauser = governancePauser;
        _relayer = relayer;
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
     * @dev Verifies and processes message originated from Vara Network.
     *
     *      In this process, MessageQueue smart contract will calculate Merkle root
     *      for message and validate that it corresponds to Merkle root which is already stored
     *      in Relayer smart contract for same block number. If proof is correct, nonce of received
     *      message will be stored in smart contract and message will be forwarded to adequate message
     *      processor, either ERC20Manager or Governance smart contract.
     *
     *      Upon successful processing of the message MessageProcessed event is emited.
     *
     *      It is important to note that anyone can submit a message because all messages
     *      will be validated against previously stored Merkle roots in the Relayer smart contract.
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
     *      - Relayer emergency stop status is set.
     *      - Message nonce is already processed.
     *      - Merkle root is not set for the block number in Relayer smart contract.
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

        if (_relayer.emergencyStop()) {
            revert RelayerEmergencyStop();
        }

        if (_processedMessages[message.nonce]) {
            revert MessageAlreadyProcessed(message.nonce);
        }

        bytes32 merkleRoot = _relayer.getMerkleRoot(blockNumber);
        if (merkleRoot == bytes32(0)) {
            revert MerkleRootNotSet(blockNumber);
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
